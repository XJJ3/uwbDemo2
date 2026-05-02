use std::time::Duration;
use std::thread;
use std::io::Read;

fn main() {
    let mut serial = serialport::new("/dev/cu.wchusbserial5AB50010561", 921600)
        .timeout(Duration::from_millis(100))
        .open()
        .expect("Failed to open port");
    
    let mut buf = [0u8; 4096];
    
    loop {
        match serial.read(&mut buf) {
            Ok(n) if n > 0 => {
                eprintln!("Read {} bytes", n);
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
}
