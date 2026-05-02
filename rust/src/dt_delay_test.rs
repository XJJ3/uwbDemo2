use clap::Parser;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use uwb_delay_test::{nlink, pack_payload, RecvBuffer, Stats, PAYLOAD_HEADER_SIZE};

#[derive(Parser, Debug)]
#[command(name = "dt_delay_test")]
struct Args {
    #[arg(short = 'm', long)]
    master_port: String,

    #[arg(short = 's', long)]
    slave_port: String,

    #[arg(short = 'b', long, default_value = "921600")]
    baudrate: u32,

    #[arg(long, default_value = "0")]
    slave_id: u8,

    #[arg(long)]
    broadcast: bool,

    #[arg(short = 'i', long, default_value = "10")]
    interval_ms: u64,

    #[arg(short = 'c', long, default_value = "100")]
    count: u16,

    #[arg(short = 'd', long, default_value = "16")]
    data_size: usize,

    #[arg(long, default_value = "3000")]
    timeout_ms: u64,
}

fn open_serial(port: &str, baudrate: u32) -> Box<dyn SerialPort> {
    serialport::new(port, baudrate)
        .timeout(Duration::from_secs(0))
        .open()
        .unwrap_or_else(|e| panic!("cannot open {}: {}", port, e))
}

fn main() {
    let args = Args::parse();

    if args.data_size < PAYLOAD_HEADER_SIZE {
        eprintln!("data_size must be at least {}", PAYLOAD_HEADER_SIZE);
        std::process::exit(1);
    }

    let mode_str = if args.broadcast { "broadcast" } else { "unicast" };
    eprintln!("DT_MODE0 delay test (Rust)");
    eprintln!("  MASTER: {}  SLAVE: {}", args.master_port, args.slave_port);
    eprintln!("  mode: {}  interval: {}ms  count: {}  payload: {}B",
        mode_str, args.interval_ms, args.count, args.data_size);
    eprintln!();

    let mut master = open_serial(&args.master_port, args.baudrate);
    let mut slave = open_serial(&args.slave_port, args.baudrate);

    let mut seq: u16 = 0;
    let interval_ns = args.interval_ms * 1_000_000;
    let start_instant = Instant::now();
    let mut next_send_ns: u64 = 0;
    let mut done = false;

    let mut pending: Vec<(u64, bool)> = vec![(0u64, false); 65536];
    let mut stats = Stats::new();
    let mut recv_buf = RecvBuffer::new(args.data_size);
    let mut payload = vec![0u8; args.data_size];

    let mut display_seq: u32 = 0;

    println!("{:>5}  {:>10}  {:>14}  {:>14}  {:>5}",
        "seq", "delay", "avg", "recent20", "lost");
    println!("{}", "-".repeat(62));

    let mut read_buf = [0u8; 4096];

    while !done {
        let now_ns = start_instant.elapsed().as_nanos() as u64;

        if let Ok(n) = slave.read(&mut read_buf) {
            if n > 0 {
                for (s, recv_ns) in recv_buf.feed(&read_buf[..n], now_ns) {
                    if pending[s as usize].1 {
                        let delay = (recv_ns - pending[s as usize].0) as f64 / 1_000_000.0;
                        pending[s as usize].1 = false;
                        stats.add(delay);

                        if s as u32 > display_seq {
                            display_seq = s as u32;
                            println!("{:>5}  {:>10.3}  {:>14.3}  {:>14.3}  {:>5}",
                                s, delay, stats.avg_ms(), stats.avg_ms(), stats.lost);
                        }
                    }
                }
            }
        }

        let deadline_ns = now_ns.saturating_sub(args.timeout_ms * 1_000_000);
        for i in 0..65536 {
            if pending[i].1 && pending[i].0 < deadline_ns {
                pending[i].1 = false;
                stats.lost += 1;
            }
        }

        if now_ns >= next_send_ns && !done {
            if args.count == 0 || seq < args.count {
                pack_payload(&mut payload, seq, now_ns as f64 / 1e9);

                let frame = if args.broadcast {
                    nlink::build_broadcast_frame(&payload)
                } else {
                    nlink::build_unicast_frame(args.slave_id, &payload)
                };

                let send_ns = start_instant.elapsed().as_nanos() as u64;
                let _ = master.write_all(&frame);

                pending[seq as usize] = (send_ns, true);
                seq += 1;

                next_send_ns += interval_ns;
                if next_send_ns < now_ns {
                    next_send_ns = now_ns + interval_ns;
                }
            }

            if args.count > 0 && seq >= args.count {
                let has_pending = pending.iter().any(|(_, v)| *v);
                if !has_pending {
                    done = true;
                } else {
                    let elapsed = now_ns;
                    if elapsed > (args.count as u64) * interval_ns + args.timeout_ms * 1_000_000 {
                        for i in 0..65536 {
                            if pending[i].1 {
                                pending[i].1 = false;
                                stats.lost += 1;
                            }
                        }
                        done = true;
                    }
                }
            }
        }
    }

    let total_s = start_instant.elapsed().as_secs_f64();

    println!();
    println!("{}", "=".repeat(46));
    println!("                TEST REPORT");
    println!("{}", "=".repeat(46));
    println!("  sent: {}  recv: {}  lost: {}",
        stats.count + stats.lost, stats.count, stats.lost);

    if stats.count > 0 {
        let loss_rate = stats.lost as f64 / (stats.count + stats.lost) as f64 * 100.0;
        println!("  loss rate: {:.1}%", loss_rate);
        println!("  total time: {:.1}s  fps: {:.1}", total_s, stats.count as f64 / total_s);
        println!("  -------------------------------------");
        println!("  min delay: {:.3} ms", stats.min_ms);
        println!("  max delay: {:.3} ms", stats.max_ms);
        println!("  avg delay: {:.3} ms", stats.avg_ms());
        if stats.count > 1 {
            println!("  jitter: {:.3} ms", stats.stddev_ms());
        }
    }
    println!("{}", "=".repeat(46));
}
