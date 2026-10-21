//! Serial ports for the Settings page's port list. Only lists; a port is never
//! opened here (the instrument may be connected to it and running a method).

/// A serial port and a human description ("FTDI · FT232R USB UART").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

/// The serial ports of this computer, sorted by name.
pub fn list_ports() -> Vec<PortInfo> {
    let mut ports = platform_ports();
    ports.sort_by_key(|p| natural_key(&p.name));
    ports.dedup_by(|a, b| a.name == b.name);
    ports
}

/// "COM10" after "COM9", "/dev/ttyUSB10" after "/dev/ttyUSB2".
fn natural_key(name: &str) -> (String, u64) {
    let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (stem, num) = name.split_at(name.len() - digits);
    (stem.to_string(), num.parse().unwrap_or(0))
}

/// Linux: read sysfs. serialport's udev enumeration opens every serial8250
/// port to see whether a UART is behind it, which must not happen here.
#[cfg(target_os = "linux")]
fn platform_ports() -> Vec<PortInfo> {
    use std::fs;
    use std::path::Path;

    let read = |p: &Path| {
        fs::read_to_string(p)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let Ok(entries) = fs::read_dir("/sys/class/tty") else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for entry in entries.flatten() {
        let tty = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // virtual terminals and ptys have no device behind them
        let Ok(device) = fs::canonicalize(tty.join("device")) else {
            continue;
        };
        // serial-core ports report their UART type; 0 means no hardware (the
        // kernel registers ttyS0…31 whether or not they exist)
        if read(&tty.join("type")).is_some_and(|t| t == "0") {
            continue;
        }
        // USB adaptors: the USB device (with idVendor) is a parent of the tty device
        let usb = device
            .ancestors()
            .take(4)
            .find(|d| d.join("idVendor").is_file());
        let description = match usb {
            Some(usb) => {
                let parts: Vec<String> = ["manufacturer", "product"]
                    .iter()
                    .filter_map(|f| read(&usb.join(f)))
                    .collect();
                if parts.is_empty() {
                    let id = |f: &str| read(&usb.join(f)).unwrap_or_default();
                    format!("USB {}:{}", id("idVendor"), id("idProduct"))
                } else {
                    parts.join(" · ")
                }
            }
            None if name.starts_with("rfcomm") => "Bluetooth".to_string(),
            None => "built-in".to_string(),
        };
        ports.push(PortInfo {
            name: format!("/dev/{name}"),
            description,
        });
    }
    ports
}

/// Windows (SetupAPI) and macOS (IOKit) list ports without opening them.
#[cfg(not(target_os = "linux"))]
fn platform_ports() -> Vec<PortInfo> {
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .map(|p| {
                let description = match p.port_type {
                    serialport::SerialPortType::UsbPort(u) => {
                        let parts: Vec<String> =
                            [u.manufacturer, u.product].into_iter().flatten().collect();
                        if parts.is_empty() {
                            format!("USB {:04x}:{:04x}", u.vid, u.pid)
                        } else {
                            parts.join(" · ")
                        }
                    }
                    serialport::SerialPortType::PciPort => "built-in".to_string(),
                    serialport::SerialPortType::BluetoothPort => "Bluetooth".to_string(),
                    serialport::SerialPortType::Unknown => String::new(),
                };
                PortInfo {
                    name: p.port_name,
                    description,
                }
            })
            .collect(),
        Err(err) => {
            log::warn!("Could not list the serial ports: {err}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_sort_naturally() {
        assert!(natural_key("COM9") < natural_key("COM10"));
        assert!(natural_key("/dev/ttyUSB2") < natural_key("/dev/ttyUSB10"));
        assert!(natural_key("/dev/ttyACM0") < natural_key("/dev/ttyUSB0"));
    }

    #[test]
    fn listing_does_not_fail() {
        // only reads sysfs / the OS port list; never opens a port
        for p in list_ports() {
            assert!(!p.name.is_empty());
        }
    }
}
