use std::io::Read;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <port>", args[0]);
        std::process::exit(1);
    }
    
    let mut serial = serialport::new(&args[1], 921600)
        .timeout(Duration::from_millis(1000))
        .open()
        .expect("Failed to open port");
    
    let mut buf = [0u8; 4096];
    let mut total = 0;
    
    eprintln!("Reading from {}...", args[1]);
    
    loop {
        match serial.read(&mut buf) {
            Ok(n) if n > 0 => {
                total += n;
                eprintln!("Read {} bytes (total {}): {:02X?}", n, total, &buf[..n.min(32)]);
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                eprintln!("Timeout (total read: {})", total);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }
}
