#include "nlink_utils.hpp"
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <cstring>
#include <unistd.h>
#include <fcntl.h>
#include <termios.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <IOKit/serial/ioss.h>
#include <time.h>
#include <vector>

inline uint64_t nanotime() {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static int open_serial(const char* port, int baudrate) {
    int fd = open(port, O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (fd < 0) { perror("open"); return -1; }
    struct termios tty;
    tcgetattr(fd, &tty);
    cfmakeraw(&tty);
    tty.c_cflag &= ~(CRTSCTS | PARENB | CSTOPB | CSIZE);
    tty.c_cflag |= CS8 | CREAD | CLOCAL;
    tty.c_iflag &= ~(IXON | IXOFF | IXANY);
    tty.c_oflag &= ~OPOST;
    tty.c_lflag &= ~(ICANON | ECHO | ECHOE | ISIG);
    tty.c_cc[VMIN] = 0;
    tty.c_cc[VTIME] = 0;
    cfsetspeed(&tty, B230400);
    tcsetattr(fd, TCSANOW, &tty);
    speed_t speed = (speed_t)baudrate;
    ioctl(fd, IOSSIOSPEED, &speed);
    int latency = 1;
    ioctl(fd, IOSSDATALAT, &latency);
    tcflush(fd, TCIOFLUSH);
    return fd;
}

struct Stats {
    uint64_t count = 0;
    double min = 1e9, max = 0, sum = 0, sum_sq = 0;
    void add(double v) {
        ++count; sum += v; sum_sq += v*v;
        if (v < min) min = v; if (v > max) max = v;
    }
    double avg() const { return count ? sum/count : 0; }
    double stddev() const {
        if (count < 2) return 0;
        double m = avg(), var = sum_sq/count - m*m;
        return var > 0 ? sqrt(var) : 0;
    }
};

// 从原始字节流中扫描 sync 头提取帧
static void scan_raw(uint8_t* buf, size_t& len, size_t payload_sz,
                     uint8_t sync0, uint8_t sync1,
                     std::vector<uint16_t>& seqs, std::vector<uint64_t>& recv_ns_list, uint64_t now_ns) {
    while (len >= payload_sz) {
        if (buf[0] == sync0 && buf[1] == sync1) {
            uint16_t seq = buf[2] | (buf[3] << 8);
            seqs.push_back(seq);
            recv_ns_list.push_back(now_ns);
            memmove(buf, buf + payload_sz, len - payload_sz);
            len -= payload_sz;
        } else {
            memmove(buf, buf + 1, len - 1);
            --len;
        }
    }
}

// 从原始流中提取 User_Frame1 帧
static void scan_user_frame1(uint8_t* buf, size_t& len,
                             std::vector<uint16_t>& seqs, std::vector<uint64_t>& recv_ns_list,
                             uint64_t now_ns, size_t payload_sz) {
    while (len >= 12) {  // min User_Frame1 size: 11 + 1 byte data
        if (buf[0] != 0x54 || buf[1] != 0xF1) {
            memmove(buf, buf + 1, len - 1); --len; continue;
        }
        if (len < 11) break;
        uint16_t dlen = buf[8] | (buf[9] << 8);
        size_t total = 11 + dlen;
        if (total > len) break;
        // verify checksum
        uint8_t cs = 0;
        for (size_t i = 0; i < total - 1; ++i) cs += buf[i];
        if (cs != buf[total - 1]) {
            memmove(buf, buf + 1, len - 1); --len; continue;
        }
        // extract payload
        if (dlen >= payload_sz && buf[10] == 0x5A && buf[11] == 0xA5) {
            uint16_t seq = buf[12] | (buf[13] << 8);
            seqs.push_back(seq);
            recv_ns_list.push_back(now_ns);
        }
        memmove(buf, buf + total, len - total);
        len -= total;
    }
}

int main(int argc, char** argv) {
    const char* mport = nullptr, *sport = nullptr;
    int slave_id = 0, baud = 921600, interval_ms = 10, count = 200, data_sz = 16;
    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "-m") && i+1<argc) mport = argv[++i];
        else if (!strcmp(argv[i], "-s") && i+1<argc) sport = argv[++i];
        else if (!strcmp(argv[i], "--id") && i+1<argc) slave_id = atoi(argv[++i]);
        else if (!strcmp(argv[i], "-b") && i+1<argc) baud = atoi(argv[++i]);
        else if (!strcmp(argv[i], "-i") && i+1<argc) interval_ms = atoi(argv[++i]);
        else if (!strcmp(argv[i], "-c") && i+1<argc) count = atoi(argv[++i]);
        else if (!strcmp(argv[i], "-d") && i+1<argc) data_sz = atoi(argv[++i]);
    }
    if (!mport || !sport) { fprintf(stderr, "用法: %s -m <master> -s <slave> [--id N] [-c N]\n", argv[0]); return 1; }
    if (data_sz < 12) data_sz = 12;

    fprintf(stderr, "DT_MODE0 双向延迟测试\n  MASTER: %s  SLAVE: %s  ID:%d  间隔:%dms  帧数:%d\n\n", mport, sport, slave_id, interval_ms, count);

    int mfd = open_serial(mport, baud);
    int sfd = open_serial(sport, baud);
    if (mfd < 0 || sfd < 0) return 1;

    // 建立 SLAVE→MASTER 双向联系
    {
        auto f = nlink::build_user_frame1(0x05, slave_id, nullptr, 0);
        write(mfd, f.data(), f.size());
        usleep(200000);
    }

    // 状态
    uint16_t seq = 0;
    uint64_t interval_ns = (uint64_t)interval_ms * 1000000ULL;
    uint64_t next_send_ns = nanotime() + interval_ns;
    bool done = false;

    struct Pending { uint64_t ms_send_ns; uint64_t sm_send_ns; };
    Pending pending[65536] = {};
    bool has_pending[65536] = {};

    Stats st_ms, st_sm, st_rt;
    uint64_t total_sent = 0, total_ms_rcv = 0, total_sm_rcv = 0, total_rt_done = 0;
    uint64_t lost_ms = 0, lost_sm = 0;

    // 接收缓冲
    uint8_t mbuf[65536], sbuf[65536];
    size_t mlen = 0, slen = 0;
    std::vector<uint8_t> payload(data_sz);

    struct pollfd pfds[2];
    pfds[0].fd = mfd; pfds[0].events = POLLIN;
    pfds[1].fd = sfd; pfds[1].events = POLLIN;

    uint64_t start_ns = nanotime();
    uint32_t display_seq = 0;

    fprintf(stdout, "%5s  %9s  %9s  %9s  %5s  %5s  %5s\n",
            "序号", "M→S(ms)", "S→M(ms)", "往返(ms)", "M→S丢", "S→M丢", "完成");
    fprintf(stdout, "------------------------------------------------------------\n");

    while (!done) {
        uint64_t now_ns = nanotime();
        int64_t wait_ns = (int64_t)(next_send_ns - now_ns);
        int timeout = (wait_ns > 0) ? (int)(wait_ns / 1000000) : 0;

        (void)poll(pfds, 2, timeout);
        now_ns = nanotime();

        // MASTER 接收 (S→M ACK)
        if (pfds[0].revents & POLLIN) {
            uint8_t rbuf[4096];
            ssize_t n = read(mfd, rbuf, sizeof(rbuf));
            if (n > 0 && mlen + n <= sizeof(mbuf)) {
                memcpy(mbuf + mlen, rbuf, n); mlen += n;
                std::vector<uint16_t> seqs; std::vector<uint64_t> ns_list;
                scan_user_frame1(mbuf, mlen, seqs, ns_list, now_ns, (size_t)data_sz);
                for (size_t i = 0; i < seqs.size(); ++i) {
                    uint16_t s = seqs[i];
                    if (has_pending[s]) {
                        double sm_ms = (double)(ns_list[i] - pending[s].sm_send_ns) / 1000000.0;
                        double rt_ms = (double)(ns_list[i] - pending[s].ms_send_ns) / 1000000.0;
                        st_sm.add(sm_ms); st_rt.add(rt_ms);
                        has_pending[s] = false;
                        ++total_sm_rcv; ++total_rt_done;
                    }
                }
            }
        }

        // SLAVE 接收 (M→S data)
        if (pfds[1].revents & POLLIN) {
            uint8_t rbuf[4096];
            ssize_t n = read(sfd, rbuf, sizeof(rbuf));
            if (n > 0 && slen + n <= sizeof(sbuf)) {
                memcpy(sbuf + slen, rbuf, n); slen += n;
                std::vector<uint16_t> seqs; std::vector<uint64_t> ns_list;
                scan_raw(sbuf, slen, (size_t)data_sz, 0xA5, 0x5A, seqs, ns_list, now_ns);
                for (size_t i = 0; i < seqs.size(); ++i) {
                    uint16_t s = seqs[i];
                    if (has_pending[s]) {
                        double ms_ms = (double)(ns_list[i] - pending[s].ms_send_ns) / 1000000.0;
                        st_ms.add(ms_ms);
                        ++total_ms_rcv;

                        // 发送 ACK: S→M
                        uint8_t ack_payload[16];
                        memset(ack_payload, 0, data_sz);
                        ack_payload[0] = 0x5A; ack_payload[1] = 0xA5;
                        ack_payload[2] = s & 0xFF; ack_payload[3] = (s >> 8) & 0xFF;
                        auto ack = nlink::build_user_frame1(0x05, slave_id, ack_payload, data_sz);
                        pending[s].sm_send_ns = nanotime();
                        write(sfd, ack.data(), ack.size());
                    }
                }
            }
        }

        // 超时清理 M→S
        {
            uint64_t deadline = now_ns - 5000000000ULL; // 5s
            for (int i = 0; i < 65536; ++i)
                if (has_pending[i] && pending[i].ms_send_ns < deadline)
                    { has_pending[i] = false; ++lost_ms; }
        }

        // 发送 M→S
        if (now_ns >= next_send_ns && !done) {
            if (count == 0 || seq < (uint16_t)count) {
                nlink::pack_payload(payload.data(), data_sz, seq, (double)nanotime()/1e9);
                auto frame = nlink::build_unicast_frame(slave_id, payload.data(), data_sz);
                pending[seq].ms_send_ns = nanotime();
                write(mfd, frame.data(), frame.size());
                has_pending[seq] = true;
                ++total_sent;

                if (seq > display_seq) {
                    display_seq = seq;
                    fprintf(stdout, "%5u  %9.3f  %9.3f  %9.3f  %5llu  %5llu  %5llu\n",
                            seq, st_ms.avg(), st_sm.avg(), st_rt.avg(),
                            (unsigned long long)lost_ms, (unsigned long long)lost_sm,
                            (unsigned long long)total_rt_done);
                    fflush(stdout);
                }

                ++seq;
                next_send_ns += interval_ns;
                if (next_send_ns < now_ns) next_send_ns = now_ns + interval_ns;
            }
            if (count > 0 && seq >= (uint16_t)count) {
                bool any = false;
                for (int i = 0; i < 65536; ++i) if (has_pending[i]) { any = true; break; }
                if (!any) done = true;
                else if (now_ns - start_ns > (uint64_t)count * interval_ns + 10000000000ULL) done = true;
            }
        }
    }

    close(mfd); close(sfd);
    double total_s = (double)(nanotime() - start_ns) / 1e9;

    fprintf(stdout, "\n==============================\n");
    fprintf(stdout, "       双向延迟测试报告\n");
    fprintf(stdout, "==============================\n");
    fprintf(stdout, "  发送: %llu  M→S收到: %llu  S→M收到: %llu  往返完成: %llu\n",
            (unsigned long long)total_sent, (unsigned long long)total_ms_rcv,
            (unsigned long long)total_sm_rcv, (unsigned long long)total_rt_done);
    fprintf(stdout, "  总耗时: %.1fs\n", total_s);
    fprintf(stdout, "  --------------------------------\n");
    if (st_ms.count) {
        fprintf(stdout, "  M→S  平均: %8.3f ms  最小: %8.3f  最大: %8.3f  σ: %.3f\n",
                st_ms.avg(), st_ms.min, st_ms.max, st_ms.stddev());
    }
    if (st_sm.count) {
        fprintf(stdout, "  S→M  平均: %8.3f ms  最小: %8.3f  最大: %8.3f  σ: %.3f\n",
                st_sm.avg(), st_sm.min, st_sm.max, st_sm.stddev());
    }
    if (st_rt.count) {
        fprintf(stdout, "  往返 平均: %8.3f ms  最小: %8.3f  最大: %8.3f  σ: %.3f\n",
                st_rt.avg(), st_rt.min, st_rt.max, st_rt.stddev());
    }
    fprintf(stdout, "==============================\n");
    return 0;
}
