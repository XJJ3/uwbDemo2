use std::io::{Read, Write};
use std::time::Duration;
use uwb_delay_test::nlink;

const PROBE_MAGIC: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
const PROBE_REPLY: &[u8] = &[0xCA, 0xFE, 0xBA, 0xBE];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <master|slave>", args[0]);
        std::process::exit(1);
    }
    
    match args[1].as_str() {
        "master" => run_master(),
        "slave" => run_slave(),
        _ => {
            eprintln!("Unknown role: {}", args[1]);
            std::process::exit(1);
        }
    }
}

fn run_master() {
    let mut serial = serialport::new("/dev/cu.wchusbserial585C0089431", 921600)
        .timeout(Duration::from_millis(200))
        .open()
        .expect("Failed to open master port");
    
    println!("Master connected");
    
    // 发送探测帧
    let mut probe_payload = [0u8; 16];
    probe_payload[0..4].copy_from_slice(PROBE_MAGIC);
    
    for id in 0u8..=5 {
        let frame = nlink::build_user_frame1(nlink::Role::Slave as u8, id, &probe_payload);
        println!("Sending probe to ID {}: {:02X?}", id, &frame[..16]);
        let _ = serial.write_all(&frame);
        
        std::thread::sleep(Duration::from_millis(50));
        
        let mut buf = [0u8; 4096];
        match serial.read(&mut buf) {
            Ok(n) if n > 0 => {
                println!("  Received {} bytes: {:02X?}", n, &buf[..n.min(32)]);
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => println!("  Error: {}", e),
        }
    }
}

fn run_slave() {
    let mut serial = serialport::new("/dev/cu.wchusbserial5AB50010561", 921600)
        .timeout(Duration::from_millis(100))
        .open()
        .expect("Failed to open slave port");
    
    println!("Slave connected, listening...");
    
    let mut buf = [0u8; 4096];
    let mut frame_buf = FrameBuffer::new();
    
    loop {
        match serial.read(&mut buf) {
            Ok(n) if n > 0 => {
                println!("Received {} bytes: {:02X?}", n, &buf[..n.min(32)]);
                
                for payload in frame_buf.feed(&buf[..n]) {
                    println!("  Payload: {:02X?}", &payload[..payload.len().min(16)]);
                    
                    if payload.len() >= 4 && &payload[0..4] == PROBE_MAGIC {
                        println!("  *** PROBE DETECTED! Sending reply ***");
                        
                        let mut reply_payload = [0u8; 16];
                        reply_payload[0..4].copy_from_slice(PROBE_REPLY);
                        
                        let reply = nlink::build_user_frame1(nlink::Role::Master as u8, 0, &reply_payload);
                        println!("  Reply frame: {:02X?}", &reply[..reply.len().min(16)]);
                        let _ = serial.write_all(&reply);
                    }
                }
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => println!("Error: {}", e),
        }
    }
}

struct FrameBuffer {
    buf: Vec<u8>,
    len: usize,
}

impl FrameBuffer {
    fn new() -> Self {
        Self { buf: vec![0u8; 65536], len: 0 }
    }

    fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        if self.len + data.len() > self.buf.len() { self.len = 0; }
        self.buf[self.len..self.len + data.len()].copy_from_slice(data);
        self.len += data.len();

        while self.len >= 11 {
            if self.buf[0] != 0x54 || self.buf[1] != 0xF1 {
                self.buf.copy_within(1..self.len, 0);
                self.len -= 1;
                continue;
            }
            let dlen = self.buf[8] as usize | ((self.buf[9] as usize) << 8);
            let total = 11 + dlen;
            if total > self.len { break; }
            
            let mut cs: u8 = 0;
            for i in 0..total - 1 { cs = cs.wrapping_add(self.buf[i]); }
            if cs != self.buf[total - 1] {
                self.buf.copy_within(1..self.len, 0);
                self.len -= 1;
                continue;
            }
            
            if dlen >= 4 {
                let payload = self.buf[10..10 + dlen].to_vec();
                frames.push(payload);
            }
            
            self.buf.copy_within(total..self.len, 0);
            self.len -= total;
        }
        frames
    }
}
