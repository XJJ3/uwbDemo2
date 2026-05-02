use std::io::{Read, Write};
use std::time::Duration;
use uwb_delay_test::nlink;

const PROBE_MAGIC: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];

fn main() {
    let mut serial = serialport::new("/dev/cu.wchusbserial5AB50010561", 921600)
        .timeout(Duration::from_millis(5000))
        .open()
        .expect("Failed to open");
    
    println!("Listening for probe frames...");
    
    let mut buf = [0u8; 4096];
    let mut frame_buf = FrameBuffer::new();
    
    loop {
        match serial.read(&mut buf) {
            Ok(n) if n > 0 => {
                println!("Received {} bytes: {:02X?}", n, &buf[..n.min(32)]);
                
                for frame in frame_buf.feed(&buf[..n]) {
                    println!("Frame payload: {:02X?}", frame);
                    
                    if frame.len() >= 4 && &frame[0..4] == PROBE_MAGIC {
                        println!("*** DETECTED PROBE FRAME ***");
                        
                        let reply_payload = [0xCA, 0xFE, 0xBA, 0xBE, 0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
                        let reply = nlink::build_user_frame1(nlink::Role::Master as u8, 0, &reply_payload);
                        println!("Sending reply: {:02X?}", reply);
                        let _ = serial.write_all(&reply);
                    }
                }
            }
            Err(e) => {
                println!("Error: {}", e);
            }
            _ => {}
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
