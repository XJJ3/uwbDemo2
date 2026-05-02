use std::io::Write;
use std::time::Duration;
use uwb_delay_test::nlink;

const PROBE_MAGIC: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];

fn main() {
    let mut probe_payload = [0u8; 16];
    probe_payload[0..4].copy_from_slice(PROBE_MAGIC);
    
    let frame = nlink::build_user_frame1(nlink::Role::Slave as u8, 0, &probe_payload);
    println!("Probe frame ({} bytes): {:02X?}", frame.len(), frame);
    
    let mut serial = serialport::new("/dev/cu.wchusbserial5AB50010561", 921600)
        .timeout(Duration::from_millis(1000))
        .open()
        .expect("Failed to open");
    
    let _ = serial.write_all(&frame);
    println!("Sent probe frame");
    
    let mut buf = [0u8; 4096];
    match serial.read(&mut buf) {
        Ok(n) => {
            println!("Received {} bytes: {:02X?}", n, &buf[..n.min(64)]);
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
}
