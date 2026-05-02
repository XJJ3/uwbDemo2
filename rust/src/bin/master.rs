use std::collections::HashMap;
use clap::Parser;
use std::io::{self, Write, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uwb_delay_test::{device, nlink, Stats, SYNC0, SYNC1};

const PROBE_MAGIC: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
const PROBE_REPLY: &[u8] = &[0xCA, 0xFE, 0xBA, 0xBE];

#[derive(Parser, Debug)]
#[command(name = "master")]
struct Args {
    #[arg(short = 'b', long, default_value = "921600")]
    baudrate: u32,
}

fn main() {
    let args = Args::parse();

    println!("DT_MODE0 Master 节点 (Rust)");
    println!("正在扫描 MASTER 设备...");

    let devices = device::scan_master_devices(args.baudrate);
    
    if devices.is_empty() {
        println!("未找到 MASTER 设备，程序退出。");
        std::process::exit(0);
    }

    println!("找到 {} 个 MASTER 设备:", devices.len());
    for (i, d) in devices.iter().enumerate() {
        println!("  [{}] {} - ID: {}", i, d.port, d.id);
    }

    let selected = &devices[0];
    println!("\n正在连接: {} (ID: {})", selected.port, selected.id);

    let mut serial = serialport::new(&selected.port, args.baudrate)
        .timeout(Duration::from_millis(5))
        .open()
        .unwrap_or_else(|e| panic!("无法打开 {}: {}", selected.port, e));
    
    println!("已连接到 MASTER 设备: {} (ID: {})", selected.port, selected.id);

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).ok();

    println!("\n命令: /s=扫描Slave, /q=退出, /h=帮助\n");
    
    let mut online_slaves: Vec<u8> = Vec::new();
    let mut read_buf = [0u8; 4096];
    let mut frame_buf = FrameBuffer::new();
    
    print!("清空串口缓冲区");
    for _ in 0..20 {
        let cleared = serial.read(&mut read_buf).unwrap_or(0);
        if cleared > 0 {
            print!(" {}", cleared);
            io::stdout().flush().ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!(" 完成");
    frame_buf.clear();

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

    let mut slave_id: Option<u8> = None;
    let mut test_limit: u64 = 500;
    let mut in_test = false;
    let mut stats = Stats::new();
    let mut seq: u16 = 0;
    let mut pending_send = true;
    let mut pending: HashMap<u16, Instant> = HashMap::new();
    let mut test_complete = false;
    let mut sent_count: u64 = 0;
    let mut recv_count: u64 = 0;

    while running.load(Ordering::SeqCst) {
        match input_rx.try_recv() {
            Ok(line) => {
                match line.as_str() {
                    "/q" | "/quit" | "/exit" => {
                        println!("正在退出...");
                        running.store(false, Ordering::SeqCst);
                    }
                    "/s" | "/scan" => {
                        if in_test {
                            println!("测试进行中，请先等待测试完成");
                            continue;
                        }
                        println!("\n正在探测在线 Slave 设备 (ID: 0-255)...");
                        online_slaves.clear();
                        frame_buf.clear();
                        
                        while serial.read(&mut read_buf).is_ok() {}
                        
                        let probe_seq: u8 = (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() & 0xFF) as u8;
                        
                        let start_time = std::time::Instant::now();
                        
                        for id in 0u8..=255 {
                            let mut probe_payload = [0u8; 16];
                            probe_payload[0..4].copy_from_slice(PROBE_MAGIC);
                            probe_payload[4] = id;
                            probe_payload[5] = probe_seq;
                            
                            let frame = nlink::build_user_frame1(nlink::Role::Slave as u8, id, &probe_payload);
                            let _ = serial.write_all(&frame);
                            let _ = serial.flush();
                            
                            let deadline = std::time::Instant::now() + Duration::from_millis(8);
                            while std::time::Instant::now() < deadline {
                                match serial.read(&mut read_buf) {
                                    Ok(n) if n > 0 => {
                                        for payload in frame_buf.feed(&read_buf[..n]) {
                                            if payload.len() >= 7 && &payload[0..4] == PROBE_REPLY {
                                                let probed_id = payload[4];
                                                let responder_id = payload[5];
                                                let reply_seq = payload[6];
                                                
                                                if probed_id == responder_id && reply_seq == probe_seq {
                                                    if !online_slaves.contains(&responder_id) {
                                                        online_slaves.push(responder_id);
                                                        print!("\r发现在线 Slave: ID {}  ", responder_id);
                                                        io::stdout().flush().ok();
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => break,
                                }
                            }
                            
                            if id % 32 == 0 {
                                print!("\r正在探测: {}/256...", id + 1);
                                io::stdout().flush().ok();
                            }
                        }
                        
                        println!("\r探测完成 (耗时 {:.1}s)    ", start_time.elapsed().as_secs_f64());
                        
                        online_slaves.sort();
                        online_slaves.dedup();
                        
                        if online_slaves.is_empty() {
                            println!("未发现在线 Slave 设备");
                        } else {
                            println!("\n发现 {} 个在线 Slave 设备:", online_slaves.len());
                            for (i, id) in online_slaves.iter().enumerate() {
                                println!("  [{}] ID: {}", i, id);
                            }
                        }
                        println!();
                    }
                    "/t" | "/test" => {
                        if in_test {
                            println!("测试进行中，请先等待测试完成");
                            continue;
                        }
                        if online_slaves.is_empty() {
                            println!("请先使用 /s 扫描在线 Slave 设备");
                            continue;
                        }
                        
                        println!("\n请选择要测试的 Slave 设备 (输入索引或 ID，直接回车选择第一个):");
                        print!("> ");
                        io::stdout().flush().ok();
                        
                        let mut input = String::new();
                        io::stdin().read_line(&mut input).ok();
                        
                        let sid = if input.trim().is_empty() {
                            online_slaves[0]
                        } else if let Ok(idx) = input.trim().parse::<usize>() {
                            if idx < online_slaves.len() {
                                online_slaves[idx]
                            } else {
                                println!("索引无效，使用第一个 Slave (ID: {})", online_slaves[0]);
                                online_slaves[0]
                            }
                        } else if let Ok(id) = input.trim().parse::<u8>() {
                            if online_slaves.contains(&id) {
                                id
                            } else {
                                println!("Slave ID {} 不在线，使用第一个 Slave (ID: {})", id, online_slaves[0]);
                                online_slaves[0]
                            }
                        } else {
                            println!("输入无效，使用第一个 Slave (ID: {})", online_slaves[0]);
                            online_slaves[0]
                        };
                        
                        print!("请输入测试次数 (默认 500): ");
                        io::stdout().flush().ok();
                        
                        input.clear();
                        io::stdin().read_line(&mut input).ok();
                        test_limit = input.trim().parse().unwrap_or(500);
                        
                        slave_id = Some(sid);
                        in_test = true;
                        test_complete = false;
                        sent_count = 0;
                        recv_count = 0;
                        seq = 0;
                        pending_send = true;
                        stats = Stats::new();
                        pending.clear();
                        
                        let handshake = nlink::build_user_frame1(nlink::Role::Slave as u8, sid, &[]);
                        let _ = serial.write_all(&handshake);
                        std::thread::sleep(Duration::from_millis(200));
                        
                        println!("\n开始测试 (目标 Slave ID: {}, 测试次数: {})...\n", sid, test_limit);
                    }
                    "/h" | "/help" => {
                        println!("\n命令:");
                        println!("  /s, /scan  - 扫描在线 Slave 设备");
                        println!("  /t, /test  - 开始测试");
                        println!("  /q, /quit  - 退出程序");
                        println!("  /h, /help  - 显示帮助\n");
                    }
                    _ => {
                        println!("未知命令: {}。输入 /h 查看帮助。", line);
                    }
                }
            }
            Err(_) => {}
        }

        if in_test && pending_send && !test_complete {
            let sid = slave_id.unwrap_or(0);
            let now_ts = get_timestamp_us();
            
            let mut payload = [0u8; 16];
            payload[0] = SYNC0;
            payload[1] = SYNC1;
            payload[2] = (seq & 0xFF) as u8;
            payload[3] = ((seq >> 8) & 0xFF) as u8;
            payload[4..12].copy_from_slice(&now_ts.to_le_bytes());
            
            let frame = nlink::build_user_frame1(nlink::Role::Slave as u8, sid, &payload);
            let write_start = Instant::now();
            let _ = serial.write_all(&frame);
            
            pending.insert(seq, write_start);
            sent_count += 1;
            pending_send = false;
            
            if sent_count % 100 == 0 {
                println!("[发送] seq={} 总发送={}", seq, sent_count);
            }
        }

        if in_test {
            match serial.read(&mut read_buf) {
                Ok(n) if n > 0 => {
                    for frame in frame_buf.feed(&read_buf[..n]) {
                        if frame.len() >= 4 && &frame[0..4] == PROBE_REPLY {
                            continue;
                        }
                        
                        if let Some(recv_seq) = parse_ack(&frame) {
                            recv_count += 1;
                            
                            if let Some(send_time) = pending.remove(&recv_seq) {
                                let rtt = send_time.elapsed().as_micros() as f64 / 1000.0;
                                stats.add(rtt);
                                
                                if recv_count % 100 == 0 {
                                    println!("[接收] seq={} rtt={:.3}ms 平均={:.3}ms", recv_seq, rtt, stats.avg_ms());
                                }
                            }
                            
                            if sent_count < test_limit {
                                seq = seq.wrapping_add(1);
                                pending_send = true;
                            } else if recv_count >= test_limit {
                                test_complete = true;
                                in_test = false;
                                
                                println!();
                                println!("{}", "=".repeat(60));
                                println!("                 MASTER 测试报告");
                                println!("{}", "=".repeat(60));
                                println!("  目标 Slave:    ID {}", slave_id.unwrap_or(0));
                                println!("  总发送:        {}", sent_count);
                                println!("  总接收:        {}", recv_count);
                                println!("  丢包:          {} ({:.1}%)", 
                                    sent_count.saturating_sub(recv_count),
                                    if sent_count > 0 { (sent_count - recv_count) as f64 / sent_count as f64 * 100.0 } else { 0.0 });
                                
                                if recv_count > 0 {
                                    println!("  最小 RTT:      {:.3} ms", stats.min_ms);
                                    println!("  最大 RTT:      {:.3} ms", stats.max_ms);
                                    println!("  平均 RTT:      {:.3} ms", stats.avg_ms());
                                }
                                println!("{}\n", "=".repeat(60));
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => {}
            }
        }

        if !pending_send {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn get_timestamp_us() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64
}

fn parse_ack(data: &[u8]) -> Option<u16> {
    if data.len() < 4 { return None; }
    if data[0] != SYNC1 || data[1] != SYNC0 { return None; }
    let seq = data[2] as u16 | ((data[3] as u16) << 8);
    Some(seq)
}

struct FrameBuffer {
    buf: Vec<u8>,
    len: usize,
}

impl FrameBuffer {
    fn new() -> Self {
        Self { buf: vec![0u8; 65536], len: 0 }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut payloads = Vec::new();
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
                payloads.push(payload);
            }
            
            self.buf.copy_within(total..self.len, 0);
            self.len -= total;
        }
        payloads
    }
}
