//! Finding Cast devices over mDNS (`_googlecast._tcp.local.`).
//!
//! Browsing only runs while someone could use it (the app decides when): the first multicast
//! socket is what makes Windows Firewall and macOS's Local Network permission ask, and nobody
//! should be asked at launch for a feature they never touch.

use std::collections::HashMap;
use std::net::IpAddr;

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use tokio::sync::mpsc;

const SERVICE: &str = "_googlecast._tcp.local.";

/// What kind of device it is, for the icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Speaker,
    Tv,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastDevice {
    /// The device's own id (the TXT `id`), stable across renames and address changes.
    pub id: String,
    /// The name its owner gave it (`fn`).
    pub name: String,
    /// The model (`md`): "Chromecast", "Google Nest Mini", "Google Cast Group".
    pub model: Option<String>,
    pub kind: Kind,
    pub ip: IpAddr,
    /// Always the announced one: a speaker group listens on a port of its own, not 8009.
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(CastDevice),
    Lost(String),
}

/// A running browse. Dropping it stops the browse and the daemon.
pub struct Browser {
    daemon: ServiceDaemon,
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE);
        let _ = self.daemon.shutdown();
    }
}

/// Start browsing. Each device found or gone arrives on the receiver.
pub fn browse() -> Result<(mpsc::UnboundedReceiver<DiscoveryEvent>, Browser), String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let events = daemon.browse(SERVICE).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        // The removal names the mDNS instance, not the device id the app keys on.
        let mut instances: HashMap<String, String> = HashMap::new();
        while let Ok(event) = events.recv_async().await {
            let sent = match event {
                ServiceEvent::ServiceResolved(info) => match device(&info) {
                    Some(d) => {
                        instances.insert(info.get_fullname().to_owned(), d.id.clone());
                        tx.send(DiscoveryEvent::Found(d))
                    }
                    None => Ok(()),
                },
                ServiceEvent::ServiceRemoved(_, fullname) => match instances.remove(&fullname) {
                    Some(id) => tx.send(DiscoveryEvent::Lost(id)),
                    None => Ok(()),
                },
                _ => Ok(()),
            };
            if sent.is_err() {
                return;
            }
        }
    });
    Ok((rx, Browser { daemon }))
}

fn device(info: &ResolvedService) -> Option<CastDevice> {
    let txt = |key| info.get_property_val_str(key).map(str::to_owned);
    // A device whose address record hasn't arrived yet is left for the next event.
    let ip = IpAddr::V4(*info.get_addresses_v4().iter().min()?);
    Some(device_from(txt("id")?, txt("fn"), txt("md"), txt("ca"), ip, info.get_port()))
}

/// The device a TXT record describes. Apart from the mDNS types so it can be tested.
fn device_from(
    id: String,
    name: Option<String>,
    model: Option<String>,
    capabilities: Option<String>,
    ip: IpAddr,
    port: u16,
) -> CastDevice {
    // `ca` is a capability bitmask; bit 0 is video out.
    let video = capabilities.and_then(|c| c.parse::<u64>().ok()).is_some_and(|c| c & 1 == 1);
    let group = model.as_deref().is_some_and(|m| m.contains("Group"));
    let kind = if group {
        Kind::Group
    } else if video {
        Kind::Tv
    } else {
        Kind::Speaker
    };
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.clone());
    CastDevice { id, name, model, kind, ip, port }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "192.168.1.20".parse().unwrap()
    }

    #[test]
    fn the_txt_record_names_the_kind() {
        let s = |v: &str| Some(v.to_owned());
        let tv = device_from("a".into(), s("Office TV"), s("Chromecast"), s("4101"), ip(), 8009);
        assert_eq!(tv.kind, Kind::Tv);
        assert_eq!(tv.name, "Office TV");
        let mini =
            device_from("b".into(), s("Kitchen"), s("Google Nest Mini"), s("2052"), ip(), 8009);
        assert_eq!(mini.kind, Kind::Speaker);
        let group = device_from(
            "c".into(),
            s("Downstairs"),
            s("Google Cast Group"),
            s("2084"),
            ip(),
            32187,
        );
        assert_eq!(group.kind, Kind::Group);
        assert_eq!(group.port, 32187);
    }

    #[test]
    fn a_device_without_a_name_is_shown_by_its_id() {
        let d = device_from("abc".into(), Some("  ".into()), None, None, ip(), 8009);
        assert_eq!(d.name, "abc");
        assert_eq!(d.kind, Kind::Speaker);
    }
}
