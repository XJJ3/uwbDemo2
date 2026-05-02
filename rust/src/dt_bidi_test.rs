use clap::Parser;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::time::Instant;
use uwb_delay_test::{nlink, pack_payload, Stats, SYNC0, SYNC1, PAYLOAD_HEADER_SIZE};

#[derive(Parser, Debug)]
#[command(name = "dt_bidi_test")]
struct Args {
    #[arg(short = 'm', long)]
    master_port: String,

    #[arg(short = 's', long)]
    slave_port: String,

    #[arg(long, default_value = "0")]
    id: u8,

    #[arg(short = 'b', long, default_value = "921600")]
    baudrate: u32,

    #[arg(short = 'i', long, default_value = "10")]
    interval_ms: u64,

    #[arg(short = 'c', long, default_value = "200")]
    count: u16,

    #[arg(short = 'd', long, default_value = "16")]
    data_size: usize,
}

fn open_serial(port: &str, baudrate: u32) -> Box<dyn SerialPort> {
    serialport::new(port, baudrate)
        .timeout(std::time::Duration::from_secs(0))
        .open()
        .unwrap_or_else(|e| panic!("cannot open {}: {}", port, e))
}

#[derive(Clone, Copy)]
struct Pending {
    ms_send_ns: u64,
    sm_send_ns: u64,
    active: bool,
}

fn main() {
    let args = Args::parse();

    let data_size = if args.data_size < PAYLOAD_HEADER_SIZE { PAYLOAD_HEADER_SIZE } else { args.data_size };

    eprintln!("DT_MODE0 bidirectional delay test (Rust optimized)");
    eprintln!("  MASTER: {}  SLAVE: {}  ID:{}  interval:{}ms  count:{}",
        args.master_port, args.slave_port, args.id, args.interval_ms, args.count);
    eprintln!();

    let mut master = open_serial(&args.master_port, args.baudrate);
    let mut slave = open_serial(&args.slave_port, args.baudrate);

    {
        let f = nlink::build_user_frame1(nlink::Role::Slave as u8, args.id, &[]);
        let _ = master.write_all(&f);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let mut seq: u16 = 0;
    let interval_ns = args.interval_ms * 1_000_000;
    let start_instant = Instant::now();
    let mut next_send_ns: u64 = 0;
    let mut done = false;

    let mut pending: Box<[Pending; 65536]> = Box::new([Pending { ms_send_ns: 0, sm_send_ns: 0, active: false }; 65536]);

    let mut st_ms = Stats::new();
    let mut st_sm = Stats::new();
    let mut st_rt = Stats::new();

    let mut total_sent: u64 = 0;
    let mut total_ms_rcv: u64 = 0;
    let mut total_sm_rcv: u64 = 0;
    let mut total_rt_done: u64 = 0;
    let mut lost_ms: u64 = 0;

    let mut mbuf: Box<[u8; 65536]> = Box::new([0u8; 65536]);
    let mut mlen: usize = 0;
    let mut sbuf: Box<[u8; 65536]> = Box::new([0u8; 65536]);
    let mut slen: usize = 0;
    
    let mut payload = vec![0u8; data_size];
    let mut ack_payload = vec![0u8; data_size];
    ack_payload[0] = 0x5A;
    ack_payload[1] = 0xA5;

    let mut display_seq: u32 = 0;

    println!("{:>5}  {:>9}  {:>9}  {:>9}  {:>5}  {:>5}  {:>5}",
        "seq", "M-S(ms)", "S-M(ms)", "RT(ms)", "M-S-lost", "S-M-lost", "done");
    println!("{}", "-".repeat(60));

    let mut read_buf = [0u8; 4096];
    let mut active_seqs: Vec<u16> = Vec::with_capacity(256);

    while !done {
        let now_ns = start_instant.elapsed().as_nanos() as u64;

        if let Ok(n) = master.read(&mut read_buf) {
            if n > 0 && mlen + n <= mbuf.len() {
                mbuf[mlen..mlen + n].copy_from_slice(&read_buf[..n]);
                mlen += n;

                for result in scan_user_frame1(&mut mbuf, &mut mlen, data_size, now_ns) {
                    let (s, recv_ns) = result;
                    if pending[s as usize].active {
                        let sm_ms = (recv_ns - pending[s as usize].sm_send_ns) as f64 / 1_000_000.0;
                        let rt_ms = (recv_ns - pending[s as usize].ms_send_ns) as f64 / 1_000_000.0;
                        st_sm.add(sm_ms);
                        st_rt.add(rt_ms);
                        pending[s as usize].active = false;
                        total_sm_rcv += 1;
                        total_rt_done += 1;
                    }
                }
            }
        }

        if let Ok(n) = slave.read(&mut read_buf) {
            if n > 0 && slen + n <= sbuf.len() {
                sbuf[slen..slen + n].copy_from_slice(&read_buf[..n]);
                slen += n;

                for result in scan_raw(&mut sbuf, &mut slen, data_size, now_ns) {
                    let (s, recv_ns) = result;
                    if pending[s as usize].active {
                        let ms_ms = (recv_ns - pending[s as usize].ms_send_ns) as f64 / 1_000_000.0;
                        st_ms.add(ms_ms);
                        total_ms_rcv += 1;

                        ack_payload[2] = (s & 0xFF) as u8;
                        ack_payload[3] = ((s >> 8) & 0xFF) as u8;
                        let ack = nlink::build_user_frame1(nlink::Role::Slave as u8, args.id, &ack_payload);
                        pending[s as usize].sm_send_ns = start_instant.elapsed().as_nanos() as u64;
                        let _ = slave.write_all(&ack);
                    }
                }
            }
        }

        if !active_seqs.is_empty() {
            let deadline = now_ns.saturating_sub(5_000_000_000);
            active_seqs.retain(|&s| {
                if pending[s as usize].ms_send_ns < deadline {
                    pending[s as usize].active = false;
                    lost_ms += 1;
                    false
                } else {
                    true
                }
            });
        }

        if now_ns >= next_send_ns && !done {
            if args.count == 0 || seq < args.count {
                pack_payload(&mut payload, seq, now_ns as f64 / 1e9);
                let frame = nlink::build_unicast_frame(args.id, &payload);
                pending[seq as usize].ms_send_ns = start_instant.elapsed().as_nanos() as u64;
                let _ = master.write_all(&frame);
                pending[seq as usize].active = true;
                active_seqs.push(seq);
                total_sent += 1;

                if seq as u32 > display_seq {
                    display_seq = seq as u32;
                    println!("{:>5}  {:>9.3}  {:>9.3}  {:>9.3}  {:>5}  {:>5}  {:>5}",
                        seq, st_ms.avg_ms(), st_sm.avg_ms(), st_rt.avg_ms(),
                        lost_ms, 0, total_rt_done);
                }

                seq += 1;
                next_send_ns += interval_ns;
                if next_send_ns < now_ns {
                    next_send_ns = now_ns + interval_ns;
                }
            }

            if args.count > 0 && seq >= args.count {
                if active_seqs.is_empty() {
                    done = true;
                } else if now_ns > (args.count as u64) * interval_ns + 10_000_000_000 {
                    done = true;
                }
            }
        }
    }

    let total_s = start_instant.elapsed().as_secs_f64();

    println!();
    println!("{}", "=".repeat(30));
    println!("   BIDIRECTIONAL TEST REPORT");
    println!("{}", "=".repeat(30));
    println!("  sent: {}  M-S-recv: {}  S-M-recv: {}  RT-done: {}",
        total_sent, total_ms_rcv, total_sm_rcv, total_rt_done);
    println!("  total time: {:.1}s", total_s);
    println!("  --------------------------------");
    if st_ms.count > 0 {
        println!("  M-S  avg: {:8.3} ms  min: {:8.3}  max: {:8.3}  sigma: {:.3}",
            st_ms.avg_ms(), st_ms.min_ms, st_ms.max_ms, st_ms.stddev_ms());
    }
    if st_sm.count > 0 {
        println!("  S-M  avg: {:8.3} ms  min: {:8.3}  max: {:8.3}  sigma: {:.3}",
            st_sm.avg_ms(), st_sm.min_ms, st_sm.max_ms, st_sm.stddev_ms());
    }
    if st_rt.count > 0 {
        println!("  RT   avg: {:8.3} ms  min: {:8.3}  max: {:8.3}  sigma: {:.3}",
            st_rt.avg_ms(), st_rt.min_ms, st_rt.max_ms, st_rt.stddev_ms());
    }
    println!("{}", "=".repeat(30));
}

struct ScanIter<'a> {
    buf: &'a mut [u8; 65536],
    len: &'a mut usize,
    payload_sz: usize,
    now_ns: u64,
}

impl<'a> Iterator for ScanIter<'a> {
    type Item = (u16, u64);

    fn next(&mut self) -> Option<Self::Item> {
        while *self.len >= self.payload_sz {
            if self.buf[0] == SYNC0 && self.buf[1] == SYNC1 {
                let seq = self.buf[2] as u16 | ((self.buf[3] as u16) << 8);
                self.buf.copy_within(self.payload_sz..*self.len, 0);
                *self.len -= self.payload_sz;
                return Some((seq, self.now_ns));
            } else {
                self.buf.copy_within(1..*self.len, 0);
                *self.len -= 1;
            }
        }
        None
    }
}

#[inline]
fn scan_raw<'a>(buf: &'a mut [u8; 65536], len: &'a mut usize, payload_sz: usize, now_ns: u64) -> ScanIter<'a> {
    ScanIter { buf, len, payload_sz, now_ns }
}

struct UserFrameIter<'a> {
    buf: &'a mut [u8; 65536],
    len: &'a mut usize,
    payload_sz: usize,
    now_ns: u64,
}

impl<'a> Iterator for UserFrameIter<'a> {
    type Item = (u16, u64);

    fn next(&mut self) -> Option<Self::Item> {
        while *self.len >= 12 {
            if self.buf[0] != 0x54 || self.buf[1] != 0xF1 {
                self.buf.copy_within(1..*self.len, 0);
                *self.len -= 1;
                continue;
            }
            if *self.len < 11 {
                break;
            }
            let dlen = self.buf[8] as usize | ((self.buf[9] as usize) << 8);
            let total = 11 + dlen;
            if total > *self.len {
                break;
            }

            let cs = self.buf[..total - 1].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            if cs != self.buf[total - 1] {
                self.buf.copy_within(1..*self.len, 0);
                *self.len -= 1;
                continue;
            }

            let result = if dlen >= self.payload_sz && self.buf[10] == 0x5A && self.buf[11] == 0xA5 {
                let seq = self.buf[12] as u16 | ((self.buf[13] as u16) << 8);
                Some((seq, self.now_ns))
            } else {
                None
            };
            
            self.buf.copy_within(total..*self.len, 0);
            *self.len -= total;
            
            if result.is_some() {
                return result;
            }
        }
        None
    }
}

#[inline]
fn scan_user_frame1<'a>(buf: &'a mut [u8; 65536], len: &'a mut usize, payload_sz: usize, now_ns: u64) -> UserFrameIter<'a> {
    UserFrameIter { buf, len, payload_sz, now_ns }
}
