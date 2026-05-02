use serialport::{SerialPort, SerialPortInfo, SerialPortType};
use std::time::Duration;

pub const ROLE_NODE: u8 = 0x00;
pub const ROLE_ANCHOR: u8 = 0x01;
pub const ROLE_TAG: u8 = 0x02;
pub const ROLE_CONSOLE: u8 = 0x03;
pub const ROLE_MASTER: u8 = 0x04;
pub const ROLE_SLAVE: u8 = 0x05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Node = 0,
    Anchor = 1,
    Tag = 2,
    Console = 3,
    Master = 4,
    Slave = 5,
    Unknown = 255,
}

impl From<u8> for Role {
    fn from(v: u8) -> Self {
        match v {
            0 => Role::Node,
            1 => Role::Anchor,
            2 => Role::Tag,
            3 => Role::Console,
            4 => Role::Master,
            5 => Role::Slave,
            _ => Role::Unknown,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Node => write!(f, "NODE"),
            Role::Anchor => write!(f, "ANCHOR"),
            Role::Tag => write!(f, "TAG"),
            Role::Console => write!(f, "CONSOLE"),
            Role::Master => write!(f, "MASTER"),
            Role::Slave => write!(f, "SLAVE"),
            Role::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub port: String,
    pub role: Role,
    pub id: u8,
}

pub fn list_serial_ports() -> Vec<SerialPortInfo> {
    serialport::available_ports().unwrap_or_default()
}

pub fn open_serial(port: &str, baudrate: u32) -> Box<dyn SerialPort> {
    serialport::new(port, baudrate)
        .timeout(Duration::from_millis(100))
        .open()
        .unwrap_or_else(|e| panic!("cannot open {}: {}", port, e))
}

fn build_system_query_frame() -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    buf[0] = 0x52;
    buf[1] = 0x00;
    buf[2] = 0x01;
    buf[3..11].fill(0xFF);
    buf[11..22].fill(0xFF);
    let cs = buf[..31].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    buf[31] = cs;
    buf
}

fn parse_system_frame(data: &[u8]) -> Option<(Role, u8)> {
    if data.len() < 32 {
        return None;
    }
    if data[0] != 0x52 || data[1] != 0x00 {
        return None;
    }
    let cs = data[..31].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    if cs != data[31] {
        return None;
    }
    let role = Role::from(data[22]);
    let id = data[23];
    Some((role, id))
}

pub fn query_device(port: &str, baudrate: u32, timeout_ms: u64) -> Option<DeviceInfo> {
    let mut serial = match serialport::new(port, baudrate)
        .timeout(Duration::from_millis(timeout_ms))
        .open()
    {
        Ok(s) => s,
        Err(_) => return None,
    };

    let query = build_system_query_frame();
    if serial.write_all(&query).is_err() {
        return None;
    }
    let _ = serial.flush();

    let mut buf = [0u8; 256];
    let mut total = 0;
    let start = std::time::Instant::now();

    while total < 32 && start.elapsed().as_millis() < timeout_ms as u128 {
        match serial.read(&mut buf[total..]) {
            Ok(n) => total += n,
            Err(_) => break,
        }
    }

    for i in 0..total {
        if i + 32 <= total {
            if let Some((role, id)) = parse_system_frame(&buf[i..i + 32]) {
                return Some(DeviceInfo {
                    port: port.to_string(),
                    role,
                    id,
                });
            }
        }
    }
    None
}

pub fn scan_devices(baudrate: u32, target_role: Role) -> Vec<DeviceInfo> {
    let ports = list_serial_ports();
    let usb_ports: Vec<_> = ports
        .into_iter()
        .filter(|p| {
            matches!(&p.port_type, SerialPortType::UsbPort(_)) &&
            p.port_name.contains("/dev/cu.")
        })
        .collect();
    
    let mut devices = Vec::new();
    
    for info in usb_ports {
        if let Some(device) = query_device(&info.port_name, baudrate, 100) {
            if device.role == target_role {
                devices.push(device);
            }
        }
    }
    devices.sort_by_key(|d| d.id);
    devices
}

pub fn scan_master_devices(baudrate: u32) -> Vec<DeviceInfo> {
    scan_devices(baudrate, Role::Master)
}

pub fn scan_slave_devices(baudrate: u32) -> Vec<DeviceInfo> {
    scan_devices(baudrate, Role::Slave)
}
