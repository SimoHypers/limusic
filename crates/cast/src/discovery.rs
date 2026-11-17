use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(CastDevice),
    Lost(String),
}

pub struct WatchHandle {
    daemon: ServiceDaemon,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse("_googlecast._tcp.local.");
    }
}

/// Pure function to parse TXT properties into (id, friendly_name, model).
pub fn parse_txt_properties(
    get_val: impl Fn(&str) -> Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let id = get_val("id");
    let name = get_val("fn");
    let model = get_val("md");
    (id, name, model)
}

fn service_info_to_device(info: &ResolvedService) -> Option<CastDevice> {
    let (id_opt, name_opt, model_opt) =
        parse_txt_properties(|key| info.get_property_val_str(key).map(|s| s.to_string()));
    let id = id_opt.or_else(|| Some(info.get_fullname().to_string()))?;

    let name = name_opt.unwrap_or_else(|| info.get_hostname().to_string());
    let port = info.get_port();

    // Prefer an IPv4 address
    let addresses = info.get_addresses();
    let mut chosen_ip = None;
    for addr in addresses {
        if let Ok(ip) = addr.to_string().parse::<IpAddr>() {
            if ip.is_ipv4() {
                chosen_ip = Some(ip);
                break;
            }
        }
    }
    let ip = chosen_ip
        .or_else(|| addresses.iter().find_map(|a| a.to_string().parse::<IpAddr>().ok()))?;

    Some(CastDevice { id, name, model: model_opt, ip, port })
}

pub fn browse(timeout: Duration) -> anyhow::Result<Vec<CastDevice>> {
    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse("_googlecast._tcp.local.")?;

    let deadline = std::time::Instant::now() + timeout;
    let mut devices = HashMap::new();

    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match receiver.recv_timeout(remaining) {
            Ok(event) => match event {
                ServiceEvent::ServiceResolved(info) => {
                    if let Some(device) = service_info_to_device(&info) {
                        tracing::debug!(device_id = %device.id, device_name = %device.name, "Discovered Cast device");
                        devices.insert(device.id.clone(), device);
                    }
                }
                ServiceEvent::ServiceRemoved(_service_type, _fullname) => {}
                _ => {}
            },
            Err(_) => break,
        }
    }

    let _ = mdns.stop_browse("_googlecast._tcp.local.");

    Ok(devices.into_values().collect())
}

pub fn watch() -> (mpsc::Receiver<DiscoveryEvent>, WatchHandle) {
    let (tx, rx) = mpsc::channel();
    let mdns = ServiceDaemon::new().expect("failed to start mdns daemon");
    let receiver = mdns.browse("_googlecast._tcp.local.").expect("failed to browse mdns");

    std::thread::spawn(move || {
        let mut known_devices: HashMap<String, String> = HashMap::new();
        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if let Some(device) = service_info_to_device(&info) {
                        tracing::debug!(device_id = %device.id, device_name = %device.name, "Watch: discovered Cast device");
                        known_devices.insert(device.id.clone(), info.get_fullname().to_string());
                        let _ = tx.send(DiscoveryEvent::Found(device));
                    }
                }
                ServiceEvent::ServiceRemoved(_service_type, fullname) => {
                    let mut removed_id = None;
                    for (id, fn_str) in &known_devices {
                        if fn_str == &fullname {
                            removed_id = Some(id.clone());
                            break;
                        }
                    }
                    if let Some(id) = removed_id {
                        tracing::debug!(device_id = %id, "Watch: lost Cast device");
                        known_devices.remove(&id);
                        let _ = tx.send(DiscoveryEvent::Lost(id));
                    }
                }
                _ => {}
            }
        }
    });

    (rx, WatchHandle { daemon: mdns })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_txt_properties() {
        let map = HashMap::from([
            ("id".to_string(), "12345".to_string()),
            ("fn".to_string(), "Living Room TV".to_string()),
            ("md".to_string(), "Chromecast Ultra".to_string()),
        ]);

        let (id, name, model) = parse_txt_properties(|k| map.get(k).cloned());
        assert_eq!(id.as_deref(), Some("12345"));
        assert_eq!(name.as_deref(), Some("Living Room TV"));
        assert_eq!(model.as_deref(), Some("Chromecast Ultra"));
    }
}
