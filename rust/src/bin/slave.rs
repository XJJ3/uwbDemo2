use clap::Parser;
use std::io::{self, Read, Write, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uwb_delay_test::{device, nlink, SYNC0, SYNC1};

const PROBE_MAGIC: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
const PROBE_REPLY: &[u8] = &[0xCA, 0xFE, 0xBA, 0xBE];

#[derive(Parser, Debug)]
#[command(name = "slave")]
struct Args {
    #[arg(short = 'b', long, default_value = "921600")]
    baudrate: u32,
    
    #[arg(short = 'i', long, default_value = "0")]
    id: u8,
}

fn main() {
    let args = Args::parse();

    println!("DT_MODE0 Slave 节点 (Rust)");
    println!("正在扫描本地 Slave 设备...");

    let devices = device::scan_slave_devices(args.baudrate);
    
    if devices.is_empty() {
        println!("未找到 Slave 设备，程序退出。");
        std::process::exit(0);
    }

    println!("找到 {} 个 Slave 设备:", devices.len());
    for (i, d) in devices.iter().enumerate() {
        println!("  [{}] {} - ID: {}", i, d.port, d.id);
    }

    let selected = if args.id > 0 {
        devices.iter().find(|d| d.id == args.id).unwrap_or_else(|| {
            eprintln!("错误: 未找到 ID 为 {} 的 Slave 设备", args.id);
            std::process::exit(1);
        })
    } else {
        &devices[0]
    };
    println!("\n正在连接: {} (ID: {})", selected.port, selected.id);

    let mut serial = serialport::new(&selected.port, args.baudrate)
        .timeout(Duration::from_millis(100))
        .open()
        .unwrap_or_else(|e| panic!("无法打开 {}: {}", selected.port, e));
    
    println!("已连接到 Slave 设备: {} (ID: {})", selected.port, selected.id);

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).ok();

    let start_instant = Instant::now();
    let mut recv_count: u64 = 0;
    let mut send_count: u64 = 0;
    let mut read_buf = [0u8; 4096];
    let mut frame_buf = FrameBuffer::new();
    let self_slave_id = selected.id;

    let (input_tx, input_rx) = crossbeam_channel::bounded(64);
    let input_running = running.clone();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        
        loop {
            if !input_running.load(Ordering::SeqCst) { break; }
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        if input_tx.send(trimmed).is_err() { break; }
                    }
                }
                Err(_) => break,
            }
        }
    });

    println!("\nSlave 运行中，等待 Master 连接...");
    println!("命令: /q=退出, /s=状态, /h=帮助\n");

    while running.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1));

        match input_rx.try_recv() {
            Ok(line) => {
                match line.as_str() {
                    "/q" | "/quit" | "/exit" => {
                        println!("正在退出...");
                        running.store(false, Ordering::SeqCst);
                    }
                    "/s" | "/status" => {
                        println!("[状态] 已接收: {}  已发送 ACK: {}", recv_count, send_count);
                    }
                    "/h" | "/help" => {
                        println!("命令: /q=退出, /s=状态, /h=帮助");
                    }
                    _ => {
                        println!("未知命令: {}。输入 /h 查看帮助。", line);
                    }
                }
            }
            Err(_) => {}
        }

        match serial.read(&mut read_buf) {
            Ok(n) if n > 0 => {
                let data = &read_buf[..n];
                
                if data.len() >= 6 && &data[0..4] == PROBE_MAGIC {
                    let probed_id = data[4];
                    let probe_seq = data[5];
                    
                    let mut reply_payload = [0u8; 16];
                    reply_payload[0..4].copy_from_slice(PROBE_REPLY);
                    reply_payload[4] = probed_id;
                    reply_payload[5] = self_slave_id;
                    reply_payload[6] = probe_seq;
                    
                    let reply = nlink::build_user_frame1(nlink::Role::Master as u8, 0, &reply_payload);
                    let _ = serial.write_all(&reply);
                    println!("[探测] 收到探测请求 (ID {})，已回复", probed_id);
                    continue;
                }
                
                if let Some((seq, _ts)) = parse_payload(data) {
                    recv_count += 1;
                    
                    let mut ack_payload = [0u8; 16];
                    ack_payload[0] = SYNC1;
                    ack_payload[1] = SYNC0;
                    ack_payload[2] = (seq & 0xFF) as u8;
                    ack_payload[3] = ((seq >> 8) & 0xFF) as u8;
                    
                    let ack = nlink::build_user_frame1(nlink::Role::Slave as u8, self_slave_id, &ack_payload);
                    let _ = serial.write_all(&ack);
                    send_count += 1;
                    
                    if recv_count <= 3 || recv_count % 100 == 0 {
                        println!("[接收] seq={} 总接收={} 已发ACK={}", seq, recv_count, send_count);
                    }
                }
            }
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => {}
        }
    }
    
    println!();
    println!("{}", "=".repeat(50));
    println!("                 SLAVE 测试报告");
    println!("{}", "=".repeat(50));
    println!("  总接收: {}", recv_count);
    println!("  ACK 已发送: {}", send_count);
    println!("  运行时间: {:.1}s", start_instant.elapsed().as_secs_f64());
    println!("{}", "=".repeat(50));
}

fn parse_payload(data: &[u8]) -> Option<(u16, f64)> {
    if data.len() < 12 { return None; }
    if data[0] != SYNC0 || data[1] != SYNC1 { return None; }
    let seq = data[2] as u16 | ((data[3] as u16) << 8);
    let ts = f64::from_le_bytes(data[4..12].try_into().unwrap());
    Some((seq, ts))
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
