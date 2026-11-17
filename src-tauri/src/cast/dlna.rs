//! DLNA / UPnP MediaRenderer: finding them over SSDP, and playing on them over AVTransport.
//!
//! A renderer answers an SSDP `M-SEARCH` with a `LOCATION` pointing at its description XML,
//! which names the human-readable device and the `AVTransport` service that takes a URI and
//! plays it. The service is SOAP over HTTP: `SetAVTransportURI` hands it the song, `Play`
//! starts it, `GetPositionInfo` says where it is.
//!
//! Two things make this harder than it looks. A renderer only speaks `http://`, while a
//! YouTube stream is `https://` — so the URI handed over is the LAN-facing proxy URL, not the
//! stream itself. And renderers are picky about the MIME type, so the track is resolved as AAC
//! in an MP4 container rather than the Opus-in-WebM the local player uses.
//!
//! Every renderer is a different box with a different idea of what a SOAP fault means, so the
//! errors say what was tried rather than what went wrong.

use std::time::Duration;

/// The address SSDP listens on.
const SSDP_ADDR: &str = "239.255.255.250:1900";
/// The search target for a media renderer.
const ST_MEDIA_RENDERER: &str = "urn:schemas-upnp-org:device:MediaRenderer:1";
/// The service a renderer plays through. Some renderers answer only a search for the
/// service, some only one for the device, so both are asked.
const ST_AVTRANSPORT: &str = "urn:schemas-upnp-org:service:AVTransport:1";
/// The AVTransport service, which is what actually plays things.
const AVTRANSPORT: &str = "urn:schemas-upnp-org:service:AVTransport:1";
/// The RenderingControl service, which is what holds the volume.
const RENDERING_CONTROL: &str = "urn:schemas-upnp-org:service:RenderingControl:1";

/// How long one search listens for answers.
pub const SEARCH_WINDOW: Duration = Duration::from_secs(3);

/// A renderer found on the network.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// The device's UDN — stable across renames.
    pub id: String,
    /// The name the owner gave it.
    pub name: String,
    /// The model, when the description carries one.
    pub model: Option<String>,
    /// Where it is.
    pub host: String,
    /// The port its description was served on.
    pub port: u16,
    /// The `AVTransport` control URL, absolute.
    pub control: String,
    /// The `RenderingControl` control URL, absolute: that is the service that holds the
    /// volume.
    pub rendering: Option<String>,
}

/// The client's view of what a renderer is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportState {
    Playing,
    Paused,
    Stopped,
    /// A state this app doesn't know.
    Other,
}

/// The HTTP client for the renderers. No proxy: a proxy on the internet cannot reach a
/// renderer on the local network, and a user proxy set for YouTube would break every cast.
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}

/// Ask the network for media renderers.
///
/// SSDP is multicast: one search reaches every device on the LAN, and each answers with the
/// address of its description. The descriptions are then fetched one by one — the search only
/// says that a renderer exists, not what it is called.
pub async fn discover() -> Result<Vec<Device>, String> {
    let socket = tokio::net::UdpSocket::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| format!("could not open the discovery socket: {e}"))?;
    socket
        .set_multicast_loop_v4(true)
        .map_err(|e| format!("could not set the discovery socket: {e}"))?;
    // Joining on the default interface: a renderer on any of the local networks answers there.
    socket
        .join_multicast_v4("239.255.255.250".parse().unwrap(), "0.0.0.0".parse().unwrap())
        .map_err(|e| format!("could not join the discovery group: {e}"))?;

    for target in [ST_MEDIA_RENDERER, ST_AVTRANSPORT] {
        let search = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n"
        );
        socket
            .send_to(search.as_bytes(), SSDP_ADDR)
            .await
            .map_err(|e| format!("the discovery search did not go out: {e}"))?;
    }

    let mut buf = vec![0u8; 8192];
    let mut locations: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + SEARCH_WINDOW;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let (len, _) = match tokio::time::timeout(left, socket.recv_from(&mut buf)).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "ssdp: receive failed");
                break;
            }
            Err(_) => break,
        };
        let answer = String::from_utf8_lossy(&buf[..len]);
        if let Some(location) = header(&answer, "location") {
            if !locations.iter().any(|l| l == &location) {
                locations.push(location);
            }
        }
    }

    let http = client()?;
    let mut devices = Vec::new();
    for location in locations {
        match describe(&http, &location).await {
            Ok(Some(device)) => devices.push(device),
            Ok(None) => {}
            Err(e) => tracing::debug!(error = %e, location, "ssdp: could not read the description"),
        }
    }
    Ok(devices)
}

/// Read a renderer's description: its name, its id, and where to play on it.
async fn describe(http: &reqwest::Client, location: &str) -> Result<Option<Device>, String> {
    let body = http
        .get(location)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let name = tag_text(&body, "friendlyName").unwrap_or_else(|| "Media renderer".to_owned());
    let id = tag_text(&body, "UDN").unwrap_or_else(|| location.to_owned());
    let model = tag_text(&body, "modelName");

    // The control URL is inside the AVTransport service block, so the search starts at the
    // service type and stops at the next one.
    let Some(service_at) = body.find(AVTRANSPORT) else {
        return Ok(None);
    };
    let rest = &body[service_at..];
    let control = rest
        .split_once("<controlURL>")
        .and_then(|(_, after)| after.split_once("</controlURL>"))
        .map(|(url, _)| url.trim().to_owned());
    let Some(control) = control else {
        return Ok(None);
    };
    // A relative control URL is resolved against the `URLBase` the description declares,
    // or against the description's own address when it declares none.
    let base = tag_text(&body, "URLBase")
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
        .map(|url| absolute(location, &url))
        .unwrap_or_else(|| location.to_owned());
    let control = absolute(&base, &control);
    // The volume lives in another service, and a renderer without one cannot be turned down.
    let rendering = body.find(RENDERING_CONTROL).and_then(|at| {
        body[at..]
            .split_once("<controlURL>")
            .and_then(|(_, after)| after.split_once("</controlURL>"))
            .map(|(url, _)| absolute(&base, url.trim()))
    });

    let (host, port) = split_host_port(location)?;
    Ok(Some(Device { id, name, model, host, port, control, rendering }))
}

/// The host and port of a URL.
fn split_host_port(url: &str) -> Result<(String, u16), String> {
    let rest =
        url.strip_prefix("http://").ok_or_else(|| format!("{url} is not an http address"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_owned(), p.parse().unwrap_or(80)),
        None => (authority.to_owned(), 80),
    };
    if host.is_empty() {
        return Err(format!("{url} has no host"));
    }
    Ok((host, port))
}

/// Make a possibly relative URL absolute against the address it was found at.
///
/// UPnP resolves a path-relative target against the description's own directory (or the
/// `URLBase` it declares), so `control/AVTransport` found at `/device/desc.xml` is
/// `/device/control/AVTransport`, not `/control/AVTransport`.
fn absolute(base: &str, target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        return target.to_owned();
    }
    let Some((scheme, rest)) = base.split_once("://") else { return target.to_owned() };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let origin = format!("{scheme}://{authority}");
    if target.starts_with('/') {
        return format!("{origin}{target}");
    }
    let target = target.strip_prefix("./").unwrap_or(target);
    let dir = if path.ends_with('/') {
        path.as_str()
    } else {
        match path.rfind('/') {
            Some(at) => &path[..=at],
            None => "/",
        }
    };
    format!("{origin}{dir}{target}")
}

/// The value of a header in an SSDP answer.
fn header(answer: &str, name: &str) -> Option<String> {
    for line in answer.lines() {
        // The status line has no colon; a header without one is not a header either.
        let Some((key, value)) = line.split_once(':') else { continue };
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// The text inside a tag, with the XML entities undone.
fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(unescape(&xml[start..end]))
}

/// Undo the five XML entities a device name can carry.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The metadata a renderer wants alongside the URI: what the track is, and how it is encoded.
#[allow(clippy::too_many_arguments)]
pub fn metadata(
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    artwork: Option<&str>,
    mime: &str,
    url: &str,
    duration_secs: Option<f64>,
) -> String {
    let mut item = String::new();
    item.push_str("<item id=\"0\" parentID=\"0\" restricted=\"1\">");
    item.push_str(&format!("<dc:title>{}</dc:title>", escape(title)));
    if let Some(a) = artist {
        item.push_str(&format!("<upnp:artist>{}</upnp:artist>", escape(a)));
    }
    if let Some(a) = album {
        item.push_str(&format!("<upnp:album>{}</upnp:album>", escape(a)));
    }
    item.push_str("<upnp:class>object.item.audioItem.musicTrack</upnp:class>");
    if let Some(art) = artwork {
        item.push_str(&format!("<upnp:albumArtURI>{}</upnp:albumArtURI>", escape(art)));
    }
    if let Some(d) = duration_secs {
        item.push_str(&format!(
            "<res protocolInfo=\"http-get:*:{mime}:*\" duration=\"{}\">{}</res>",
            upnp_time(d),
            escape(url)
        ));
    } else {
        item.push_str(&format!("<res protocolInfo=\"http-get:*:{mime}:*\">{}</res>", escape(url)));
    }
    item.push_str("</item>");
    format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
         xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\" \
         xmlns:res=\"http://www.w3.org/2000/09/xmldns/\">{item}</DIDL-Lite>"
    )
}

/// The XML-escaped form of a string.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The `H:MM:SS` duration a renderer expects.
fn upnp_time(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    format!("{}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
}

/// The `H:MM:SS` position a renderer answers with, in seconds.
fn parse_upnp_time(value: &str) -> f64 {
    let mut parts = value.trim().split(':');
    let mut secs = 0f64;
    let mut any = false;
    for part in parts.by_ref() {
        match part.parse::<f64>() {
            Ok(v) => {
                secs = secs * 60.0 + v;
                any = true;
            }
            Err(_) => return 0.0,
        }
    }
    if any {
        secs
    } else {
        0.0
    }
}

/// Hand a renderer a track and start it.
pub async fn play(
    control: &str,
    url: &str,
    meta: &str,
    start_at: Option<f64>,
) -> Result<(), String> {
    let body = format!(
        "<u:SetAVTransportURI xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID>\
         <CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData></u:SetAVTransportURI>",
        escape(url),
        escape(meta)
    );
    soap(AVTRANSPORT, control, "SetAVTransportURI", &body).await?;
    soap(
        AVTRANSPORT,
        control,
        "Play",
        &format!(
            "<u:Play xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID><Speed>1</Speed></u:Play>"
        ),
    )
    .await?;
    if let Some(position) = start_at.filter(|position| position.is_finite() && *position > 0.0) {
        seek(control, position).await?;
    }
    Ok(())
}

/// Pause a renderer.
pub async fn pause(control: &str) -> Result<(), String> {
    soap(
        AVTRANSPORT,
        control,
        "Pause",
        &format!("<u:Pause xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID></u:Pause>"),
    )
    .await
    .map(|_| ())
}

/// Resume a renderer.
pub async fn resume(control: &str) -> Result<(), String> {
    soap(
        AVTRANSPORT,
        control,
        "Play",
        &format!(
            "<u:Play xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID><Speed>1</Speed></u:Play>"
        ),
    )
    .await
    .map(|_| ())
}

/// Stop a renderer.
pub async fn stop(control: &str) -> Result<(), String> {
    soap(
        AVTRANSPORT,
        control,
        "Stop",
        &format!("<u:Stop xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID></u:Stop>"),
    )
    .await
    .map(|_| ())
}

/// Move a renderer to `seconds` in the track.
pub async fn seek(control: &str, seconds: f64) -> Result<(), String> {
    let body = format!(
        "<u:Seek xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{}</Target></u:Seek>",
        upnp_time(seconds)
    );
    soap(AVTRANSPORT, control, "Seek", &body).await.map(|_| ())
}

/// Where the renderer is, and how long the track is.
pub async fn position(control: &str) -> Result<(f64, f64), String> {
    let body = format!(
        "<u:GetPositionInfo xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID></u:GetPositionInfo>"
    );
    let answer = soap(AVTRANSPORT, control, "GetPositionInfo", &body).await?;
    let duration = tag_text(&answer, "TrackDuration").map(|d| parse_upnp_time(&d)).unwrap_or(0.0);
    let position = tag_text(&answer, "RelTime").map(|d| parse_upnp_time(&d)).unwrap_or(0.0);
    Ok((position, duration))
}

/// What the renderer is doing.
pub async fn transport_state(control: &str) -> Result<TransportState, String> {
    let body = format!(
        "<u:GetTransportInfo xmlns:u=\"{AVTRANSPORT}\"><InstanceID>0</InstanceID></u:GetTransportInfo>"
    );
    let answer = soap(AVTRANSPORT, control, "GetTransportInfo", &body).await?;
    let state = tag_text(&answer, "CurrentTransportState").unwrap_or_default();
    Ok(match state.as_str() {
        "PLAYING" => TransportState::Playing,
        "PAUSED_PLAYBACK" | "PAUSED_RECORDING" => TransportState::Paused,
        "STOPPED" => TransportState::Stopped,
        _ => TransportState::Other,
    })
}

/// Set a renderer's volume, 0.0 to 1.0.
pub async fn set_volume(control: &str, level: f64) -> Result<(), String> {
    let volume = (level.clamp(0.0, 1.0) * 100.0).round() as u32;
    let body = format!(
        "<u:SetVolume xmlns:u=\"{RENDERING_CONTROL}\"><InstanceID>0</InstanceID>\
         <Channel>Master</Channel><DesiredVolume>{volume}</DesiredVolume></u:SetVolume>"
    );
    soap(RENDERING_CONTROL, control, "SetVolume", &body).await.map(|_| ())
}

/// One SOAP call.
async fn soap(service: &str, control: &str, action: &str, body: &str) -> Result<String, String> {
    let envelope = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>{body}</s:Body></s:Envelope>"
    );
    let response = client()?
        .post(control)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("SOAPACTION", format!("\"{service}#{action}\""))
        .body(envelope)
        .send()
        .await
        .map_err(|e| format!("the renderer did not answer: {e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("the renderer refused {action} ({status})"));
    }
    if text.contains("Fault") {
        let fault = tag_text(&text, "errorDescription")
            .or_else(|| tag_text(&text, "UPnPError"))
            .unwrap_or_else(|| "the renderer reported an error".to_owned());
        return Err(fault);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_yields_the_name_and_the_control_url() {
        let xml = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device>
    <friendlyName>Kitchen &amp; Dining</friendlyName>
    <modelName>Some Renderer</modelName>
    <UDN>uuid:1234-5678</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
        <controlURL>/control/RenderingControl</controlURL>
      </service>
      <service>
        <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <controlURL>/control/AVTransport</controlURL>
      </service>
    </serviceList>
  </device>
</root>"#;
        assert_eq!(tag_text(xml, "friendlyName").as_deref(), Some("Kitchen & Dining"));
        assert_eq!(tag_text(xml, "modelName").as_deref(), Some("Some Renderer"));
        assert_eq!(tag_text(xml, "UDN").as_deref(), Some("uuid:1234-5678"));
        let at = xml.find(AVTRANSPORT).unwrap();
        let rest = &xml[at..];
        let control = rest
            .split_once("<controlURL>")
            .and_then(|(_, after)| after.split_once("</controlURL>"))
            .map(|(u, _)| u.trim())
            .unwrap();
        assert_eq!(control, "/control/AVTransport");
    }

    #[test]
    fn a_relative_control_url_becomes_absolute() {
        assert_eq!(
            absolute("http://192.168.1.5:8080/desc.xml", "/control/AVTransport"),
            "http://192.168.1.5:8080/control/AVTransport"
        );
        assert_eq!(
            absolute("http://192.168.1.5:8080/desc.xml", "http://other/ctl"),
            "http://other/ctl"
        );
    }

    #[test]
    fn a_path_relative_control_url_resolves_against_the_description_directory() {
        assert_eq!(
            absolute("http://192.168.1.5:8080/device/desc.xml", "control/AVTransport"),
            "http://192.168.1.5:8080/device/control/AVTransport"
        );
        assert_eq!(
            absolute("http://192.168.1.5:8080/device/desc.xml", "./control/AVTransport"),
            "http://192.168.1.5:8080/device/control/AVTransport"
        );
        // A `URLBase` that ends in a directory is the directory itself.
        assert_eq!(
            absolute("http://192.168.1.5:8080/upnp/", "control/AVTransport"),
            "http://192.168.1.5:8080/upnp/control/AVTransport"
        );
        assert_eq!(
            absolute("http://192.168.1.5:8080", "control/AVTransport"),
            "http://192.168.1.5:8080/control/AVTransport"
        );
    }

    #[test]
    fn a_host_and_port_come_out_of_the_description_url() {
        assert_eq!(
            split_host_port("http://192.168.1.5:8080/desc.xml").unwrap(),
            ("192.168.1.5".to_owned(), 8080)
        );
        assert_eq!(
            split_host_port("http://192.168.1.5/desc.xml").unwrap(),
            ("192.168.1.5".to_owned(), 80)
        );
    }

    #[test]
    fn ssdp_headers_are_read_case_insensitively() {
        let answer =
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.5:8080/desc.xml\r\nST: ssdp:all\r\n";
        assert_eq!(header(answer, "location").as_deref(), Some("http://192.168.1.5:8080/desc.xml"));
        assert_eq!(header(answer, "Location").as_deref(), Some("http://192.168.1.5:8080/desc.xml"));
        assert_eq!(header(answer, "usn"), None);
    }

    #[test]
    fn upnp_times_round_trip() {
        assert_eq!(upnp_time(213.0), "0:03:33");
        assert_eq!(parse_upnp_time("0:03:33"), 213.0);
        assert_eq!(parse_upnp_time("1:00:00"), 3600.0);
        assert_eq!(parse_upnp_time("0:00:01.500"), 1.5);
        assert_eq!(parse_upnp_time("nonsense"), 0.0);
    }

    #[test]
    fn the_metadata_carries_the_track_and_its_encoding() {
        let meta = metadata(
            "Song",
            Some("Artist"),
            Some("Album"),
            None,
            "audio/mp4",
            "http://192.168.1.2:9000/a/1",
            Some(213.0),
        );
        assert!(meta.contains("<dc:title>Song</dc:title>"));
        assert!(meta.contains("<upnp:artist>Artist</upnp:artist>"));
        assert!(meta.contains("audio/mp4"));
        assert!(meta.contains("duration=\"0:03:33\""));
    }
}
