#!/usr/bin/env python3
"""
DT_MODE0 数传延迟测试工具
===========================
单机双串口方案：一台电脑同时连接 MASTER 和 SLAVE。
通过 MASTER 发送 User_Frame1 协议帧，从 SLAVE 接收原始透传字节，
测量端到端延迟。

用法:
  python3 dt_delay_test.py -m /dev/cu.wchusbserial585C0089431 -s /dev/cu.wchusbserial5AB50010561

依赖: pip3 install pyserial
"""

import argparse
import struct
import sys
import time
import threading
import queue
from collections import deque
from dataclasses import dataclass, field
from typing import Optional

import serial

from nlink_utils import (
    UserFrame1, dt_payload_pack, DTPayloadSynchronizer,
)

@dataclass
class SendRecord:
    seq: int
    send_time: float


@dataclass
class RecvRecord:
    seq: int
    recv_time: float


@dataclass
class DelaySample:
    seq: int
    delay_ms: float


@dataclass
class Stats:
    count: int = 0
    lost: int = 0
    min_ms: float = float('inf')
    max_ms: float = float('-inf')
    total_ms: float = 0.0
    samples: deque = field(default_factory=lambda: deque(maxlen=10000))

    def add(self, sample: DelaySample):
        self.count += 1
        self.total_ms += sample.delay_ms
        self.samples.append(sample)
        if sample.delay_ms < self.min_ms:
            self.min_ms = sample.delay_ms
        if sample.delay_ms > self.max_ms:
            self.max_ms = sample.delay_ms

    @property
    def avg_ms(self) -> float:
        return self.total_ms / self.count if self.count else 0.0

    def recent_avg(self, n: int = 20) -> float:
        recent = list(self.samples)[-n:]
        return sum(s.delay_ms for s in recent) / len(recent) if recent else 0.0


def open_serial(port: str, baudrate: int = 921600, timeout: float = 0.01) -> serial.Serial:
    try:
        ser = serial.Serial(
            port=port, baudrate=baudrate,
            bytesize=serial.EIGHTBITS, parity=serial.PARITY_NONE,
            stopbits=serial.STOPBITS_ONE, timeout=timeout,
        )
        print(f"[串口] {port} 已打开 (baud={baudrate})")
        return ser
    except serial.SerialException as e:
        print(f"[错误] 无法打开串口 {port}: {e}")
        sys.exit(1)


def sender_thread(
    master_serial: serial.Serial,
    slave_id: int,
    broadcast: bool,
    data_size: int,
    interval_ms: float,
    max_count: int,
    send_queue: queue.Queue,
    stop_event: threading.Event,
):
    seq = 0
    next_send_time = time.monotonic()

    while not stop_event.is_set() and (max_count == 0 or seq < max_count):
        now = time.monotonic()
        if now < next_send_time:
            time.sleep(min(next_send_time - now, 0.001))
            continue

        send_timestamp = time.time()
        payload = dt_payload_pack(seq, send_timestamp, data_size)

        if broadcast:
            frame = UserFrame1.broadcast(payload)
        else:
            frame = UserFrame1.unicast(slave_id, payload)

        send_mono = time.monotonic()
        try:
            master_serial.write(frame.to_bytes())
            master_serial.flush()
        except serial.SerialException as e:
            print(f"[错误] MASTER 串口写入失败: {e}")
            stop_event.set()
            break

        send_queue.put(SendRecord(seq=seq, send_time=send_mono))
        seq += 1
        next_send_time += interval_ms / 1000.0

        if next_send_time < time.monotonic():
            next_send_time = time.monotonic() + interval_ms / 1000.0


def receiver_thread(
    slave_serial: serial.Serial,
    payload_size: int,
    recv_queue: queue.Queue,
    stop_event: threading.Event,
):
    sync = DTPayloadSynchronizer(payload_size)

    while not stop_event.is_set():
        try:
            raw = slave_serial.read(4096)
        except serial.SerialException as e:
            print(f"[错误] SLAVE 串口读取失败: {e}")
            stop_event.set()
            break

        if not raw:
            continue

        recv_mono = time.monotonic()
        for seq, send_ts in sync.feed(raw):
            recv_queue.put(RecvRecord(seq=seq, recv_time=recv_mono))


def print_header():
    print()
    print("=" * 72)
    print(f"{'序号':>5}  {'延迟(ms)':>10}  {'累计平均(ms)':>14}  {'最近20帧(ms)':>14}  {'丢包':>5}")
    print("-" * 72)


def print_stats(stats: Stats, sample: Optional[DelaySample] = None):
    if sample:
        print(f"{sample.seq:>5}  {sample.delay_ms:>10.3f}  {stats.avg_ms:>14.3f}  {stats.recent_avg():>14.3f}  {stats.lost:>5}")


def print_final_report(stats: Stats, total_elapsed: float, args):
    print()
    print("=" * 72)
    print("                        测试报告")
    print("=" * 72)
    mode = '广播' if args.broadcast else f'定向 → SLAVE {args.slave_id}'
    print(f"  模式:              {mode}")
    print(f"  发送间隔:          {args.interval_ms} ms")
    print(f"  载荷大小:          {args.data_size} bytes")
    print(f"  目标帧数:          {args.count if args.count else '无限'}")
    print(f"  实际发送:          {stats.count + stats.lost}")
    print(f"  成功接收:          {stats.count}")
    print(f"  丢包:              {stats.lost}")
    if stats.count > 0:
        loss_rate = stats.lost / (stats.count + stats.lost) * 100
        print(f"  丢包率:            {loss_rate:.1f}%")
        print(f"  总耗时:            {total_elapsed:.1f}s")
        print(f"  有效帧率:          {stats.count / total_elapsed:.1f} fps")
        print("-" * 72)
        print(f"  最小延迟:          {stats.min_ms:.3f} ms")
        print(f"  最大延迟:          {stats.max_ms:.3f} ms")
        print(f"  平均延迟:          {stats.avg_ms:.3f} ms")
        if stats.count > 1:
            mean = stats.avg_ms
            var = sum((s.delay_ms - mean) ** 2 for s in stats.samples) / stats.count
            std = var ** 0.5
            print(f"  抖动 (标准差):     {std:.3f} ms")
    print("=" * 72)


def main():
    parser = argparse.ArgumentParser(
        description="LinkTrack DT_MODE0 数传延迟测试工具",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  python3 dt_delay_test.py -m /dev/cu.wchusbserial585C0089431 -s /dev/cu.wchusbserial5AB50010561 --slave-id 0 -i 100 -c 100
  python3 dt_delay_test.py -m /dev/cu.wchusbserial585C0089431 -s /dev/cu.wchusbserial5AB31140701 --slave-id 1 -i 50 --broadcast
        """,
    )
    parser.add_argument('-m', '--master-port', required=True,
                        help='MASTER 串口路径')
    parser.add_argument('-s', '--slave-port', required=True,
                        help='SLAVE 串口路径')
    parser.add_argument('-b', '--baudrate', type=int, default=921600,
                        help='波特率 (默认: 921600)')
    parser.add_argument('--slave-id', type=int, default=0,
                        help='目标 SLAVE ID (默认: 0)')
    parser.add_argument('--broadcast', action='store_true',
                        help='广播模式')
    parser.add_argument('-i', '--interval-ms', type=float, default=100,
                        help='发送间隔 ms (默认: 100)')
    parser.add_argument('-c', '--count', type=int, default=100,
                        help='发送帧数，0=无限 (默认: 100)')
    parser.add_argument('-d', '--data-size', type=int, default=16,
                        help='载荷大小 bytes (默认: 16, 最小: 12)')
    parser.add_argument('--timeout', type=float, default=5.0,
                        help='单帧超时秒 (默认: 5.0)')

    args = parser.parse_args()

    if args.data_size < 12:
        print("[错误] 载荷大小至少为 12 字节 (2B sync + 2B seq + 8B timestamp)")
        sys.exit(1)

    print(f"DT_MODE0 延迟测试")
    print(f"  MASTER: {args.master_port}")
    print(f"  SLAVE:  {args.slave_port}")
    print(f"  模式:   {'广播' if args.broadcast else f'定向 → SLAVE {args.slave_id}'}")
    print(f"  间隔:   {args.interval_ms}ms  帧数: {args.count if args.count else '无限'}  载荷: {args.data_size}B")
    print()

    master = open_serial(args.master_port, args.baudrate)
    slave = open_serial(args.slave_port, args.baudrate)

    send_queue, recv_queue = queue.Queue(), queue.Queue()
    stop_event = threading.Event()

    recv_thread = threading.Thread(
        target=receiver_thread,
        args=(slave, args.data_size, recv_queue, stop_event),
        daemon=True,
    )
    recv_thread.start()

    send_thread = threading.Thread(
        target=sender_thread,
        args=(master, args.slave_id, args.broadcast, args.data_size,
              args.interval_ms, args.count, send_queue, stop_event),
        daemon=True,
    )
    send_thread.start()

    stats = Stats()
    pending: dict[int, SendRecord] = {}
    print_header()

    start_time = time.monotonic()
    last_display_seq = -1

    try:
        while not stop_event.is_set():
            try:
                while True:
                    sr = send_queue.get_nowait()
                    pending[sr.seq] = sr
            except queue.Empty:
                pass

            try:
                while True:
                    rr = recv_queue.get_nowait()
                    if rr.seq in pending:
                        sr = pending.pop(rr.seq)
                        delay = (rr.recv_time - sr.send_time) * 1000.0
                        sample = DelaySample(seq=rr.seq, delay_ms=delay)
                        stats.add(sample)
                        if rr.seq > last_display_seq:
                            print_stats(stats, sample)
                            last_display_seq = rr.seq
            except queue.Empty:
                pass

            now = time.monotonic()
            expired = [
                seq for seq, rec in pending.items()
                if (now - rec.send_time) * 1000.0 > args.timeout * 1000.0
            ]
            for seq in expired:
                del pending[seq]
                stats.lost += 1

            if args.count > 0 and stats.count + stats.lost >= args.count:
                if not pending:
                    break
                elif now - start_time > (args.count * args.interval_ms / 1000.0) + args.timeout:
                    stats.lost += len(pending)
                    break

            time.sleep(0.001)

    except KeyboardInterrupt:
        print("\n[中断] 用户取消测试")

    finally:
        stop_event.set()
        total_elapsed = time.monotonic() - start_time
        send_thread.join(timeout=2)
        recv_thread.join(timeout=2)
        master.close()
        slave.close()

    print_final_report(stats, total_elapsed, args)


if __name__ == '__main__':
    main()
