use std::time::Duration;

fn main() {
    println!("Browsing for Chromecast devices for 5 seconds...");
    match cast::browse(Duration::from_secs(5)) {
        Ok(devices) => {
            println!("Found {} devices:", devices.len());
            for d in devices {
                println!(
                    " - ID: {}, Name: {}, Model: {:?}, IP: {}:{}",
                    d.id, d.name, d.model, d.ip, d.port
                );
            }
        }
        Err(e) => eprintln!("Error browsing for devices: {}", e),
    }
}
