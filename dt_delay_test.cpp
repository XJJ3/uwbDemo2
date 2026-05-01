#include "nlink_utils.hpp"
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <cstring>
#include <cinttypes>
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

static int open_serial_low_latency(const char* port, int baudrate) {
    int fd = open(port, O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (fd < 0) { perror("open"); return -1; }

    struct termios tty;
    if (tcgetattr(fd, &tty) < 0) { perror("tcgetattr"); close(fd); return -1; }

    cfmakeraw(&tty);
    tty.c_cflag &= ~(CRTSCTS | PARENB | CSTOPB | CSIZE);
    tty.c_cflag |= CS8 | CREAD | CLOCAL;
    tty.c_iflag &= ~(IXON | IXOFF | IXANY);
    tty.c_oflag &= ~OPOST;
    tty.c_lflag &= ~(ICANON | ECHO | ECHOE | ISIG);
    tty.c_cc[VMIN] = 0;
    tty.c_cc[VTIME] = 0;

    cfsetspeed(&tty, B230400);
    if (tcsetattr(fd, TCSANOW, &tty) < 0) { perror("tcsetattr"); close(fd); return -1; }

    speed_t speed = (speed_t)baudrate;
    if (ioctl(fd, IOSSIOSPEED, &speed) < 0) {
        fprintf(stderr, "[警告] 无法设置波特率 %d, 使用 230400\n", baudrate);
    }

    tcflush(fd, TCIOFLUSH);

    // 🔑 关键优化：将 USB 转串口延迟定时器降到 1ms
    int latency = 1;
    if (ioctl(fd, IOSSDATALAT, &latency) < 0) {
        fprintf(stderr, "[警告] 无法设置 IOSSDATALAT, 延迟可能偏高\n");
    }

    return fd;
}

struct RecvBuffer {
    uint8_t buf[65536];
    size_t len = 0;
    size_t payload_size;

    explicit RecvBuffer(size_t ps) : payload_size(ps) {}

    struct Result { uint16_t seq; double ts; uint64_t recv_ns; };
    std::vector<Result> feed(const uint8_t* data, size_t n, uint64_t recv_ns) {
        std::vector<Result> out;
        if (len + n > sizeof(buf)) len = 0;
        memcpy(buf + len, data, n);
        len += n;

        while (len >= payload_size) {
            auto* p = buf;
            if (p[0] == nlink::SYNC0 && p[1] == nlink::SYNC1) {
                uint16_t seq; double ts;
                if (nlink::unpack_payload(p, payload_size, seq, ts)) {
                    out.push_back({seq, ts, recv_ns});
                }
                memmove(buf, buf + payload_size, len - payload_size);
                len -= payload_size;
            } else {
                memmove(buf, buf + 1, len - 1);
                --len;
            }
        }
        return out;
    }
};

struct Stats {
    uint64_t count = 0, lost = 0;
    double min_ms = 1e9, max_ms = 0, sum_ms = 0, sum_sq_ms = 0;
    void add(double delay_ms) {
        ++count; sum_ms += delay_ms; sum_sq_ms += delay_ms * delay_ms;
        if (delay_ms < min_ms) min_ms = delay_ms;
        if (delay_ms > max_ms) max_ms = delay_ms;
    }
};

// ========== 主程序 ==========
int main(int argc, char** argv) {
    const char* master_port = nullptr;
    const char* slave_port = nullptr;
    int baudrate = 921600;
    int slave_id = 0;
    bool broadcast = false;
    int interval_ms = 10;
    int count = 100;
    int data_size = 16;
    int timeout_ms = 3000;

    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "-m") && i+1 < argc) master_port = argv[++i];
        else if (!strcmp(argv[i], "-s") && i+1 < argc) slave_port = argv[++i];
        else if (!strcmp(argv[i], "-b") && i+1 < argc) baudrate = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--slave-id") && i+1 < argc) slave_id = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--broadcast")) broadcast = true;
        else if (!strcmp(argv[i], "-i") && i+1 < argc) interval_ms = atoi(argv[++i]);
        else if (!strcmp(argv[i], "-c") && i+1 < argc) count = atoi(argv[++i]);
        else if (!strcmp(argv[i], "-d") && i+1 < argc) data_size = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--timeout") && i+1 < argc) timeout_ms = atoi(argv[++i]);
        else { fprintf(stderr, "未知参数: %s\n", argv[i]); return 1; }
    }

    if (!master_port || !slave_port) {
        fprintf(stderr, "用法: %s -m <master> -s <slave> [--broadcast] [--slave-id N] [-i ms] [-c N] [-d N]\n", argv[0]);
        return 1;
    }
    if (data_size < 12) { fprintf(stderr, "data_size 至少为 12\n"); return 1; }

    fprintf(stderr, "DT_MODE0 延迟测试 (C++)\n");
    fprintf(stderr, "  MASTER: %s  SLAVE: %s\n", master_port, slave_port);
    fprintf(stderr, "  模式: %s  间隔: %dms  帧数: %d  载荷: %dB\n\n",
            broadcast ? "广播" : "定向", interval_ms, count, data_size);

    int master_fd = open_serial_low_latency(master_port, baudrate);
    int slave_fd  = open_serial_low_latency(slave_port, baudrate);
    if (master_fd < 0 || slave_fd < 0) return 1;

    fprintf(stderr, "%s", "串口已配置 (延迟定时器=1ms)\n");

    // 发送状态
    uint16_t seq = 0;
    uint64_t interval_ns = (uint64_t)interval_ms * 1000000ULL;
    uint64_t next_send_ns = nanotime() + interval_ns;
    bool done = false;

    // 待匹配的发送记录
    struct { uint64_t send_ns; bool valid; } pending[65536] = {};
    Stats stats;

    // 接收缓冲区
    RecvBuffer recv(data_size);

    // 载荷缓冲
    std::vector<uint8_t> payload(data_size);

    // poll
    struct pollfd pfd;
    pfd.fd = slave_fd;
    pfd.events = POLLIN;

    uint32_t display_seq = 0;
    uint64_t start_ns = nanotime();

    fprintf(stdout, "%5s  %10s  %14s  %14s  %5s\n",
            "序号", "延迟(ms)", "累计平均(ms)", "最近20帧(ms)", "丢包");
    fprintf(stdout, "--------------------------------------------------------------\n");

    while (!done) {
        uint64_t now_ns = nanotime();
        int64_t wait_ns = (int64_t)(next_send_ns - now_ns);
        int poll_timeout = (wait_ns > 0) ? (int)(wait_ns / 1000000) : 0;

        int ret = poll(&pfd, 1, poll_timeout);
        now_ns = nanotime();

        // 接收
        if (ret > 0 && (pfd.revents & POLLIN)) {
            uint8_t rbuf[4096];
            ssize_t n = read(slave_fd, rbuf, sizeof(rbuf));
            if (n > 0) {
                auto results = recv.feed(rbuf, (size_t)n, now_ns);
                for (auto& r : results) {
                    if (pending[r.seq].valid) {
                        double delay = (double)(r.recv_ns - pending[r.seq].send_ns) / 1000000.0;
                        pending[r.seq].valid = false;
                        stats.add(delay);

                        if (r.seq > display_seq) {
                            display_seq = r.seq;
                            double avg = stats.sum_ms / (double)stats.count;
                            fprintf(stdout, "%5u  %10.3f  %14.3f  %14.3f  %5llu\n",
                                    r.seq, delay, avg, avg, (unsigned long long)stats.lost);
                            fflush(stdout);
                        }
                    }
                }
            }
        }

        // 超时清理
        {
            uint64_t deadline_ns = now_ns - (uint64_t)timeout_ms * 1000000ULL;
            for (int i = 0; i < 65536; ++i) {
                if (pending[i].valid && pending[i].send_ns < deadline_ns) {
                    pending[i].valid = false;
                    ++stats.lost;
                }
            }
        }

        // 发送
        if (now_ns >= next_send_ns && !done) {
            if (count == 0 || seq < (uint16_t)count) {
                double ts = (double)nanotime() / 1e9;
                nlink::pack_payload(payload.data(), data_size, seq, ts);

                std::vector<uint8_t> frame;
                if (broadcast)
                    frame = nlink::build_broadcast_frame(payload.data(), data_size);
                else
                    frame = nlink::build_unicast_frame(slave_id, payload.data(), data_size);

                uint64_t send_ns = nanotime();
                ssize_t written = write(master_fd, frame.data(), frame.size());
                if (written < 0) { perror("write master"); break; }

                pending[seq].send_ns = send_ns;
                pending[seq].valid = true;
                ++seq;

                next_send_ns += interval_ns;
                if (next_send_ns < now_ns) next_send_ns = now_ns + interval_ns;
            }

            if (count > 0 && seq >= (uint16_t)count) {
                // 等pending清空
                bool has_pending = false;
                for (int i = 0; i < 65536; ++i) if (pending[i].valid) { has_pending = true; break; }
                if (!has_pending) done = true;
                else {
                    uint64_t elapsed = now_ns - start_ns;
                    if (elapsed > (uint64_t)count * interval_ns + (uint64_t)timeout_ms * 1000000ULL) {
                        for (int i = 0; i < 65536; ++i) if (pending[i].valid) { pending[i].valid = false; ++stats.lost; }
                        done = true;
                    }
                }
            }
        }
    }

    close(master_fd);
    close(slave_fd);

    double total_s = (double)(nanotime() - start_ns) / 1e9;

    fprintf(stdout, "\n==============================================\n");
    fprintf(stdout, "                测试报告\n");
    fprintf(stdout, "==============================================\n");
    fprintf(stdout, "  发送: %llu  接收: %llu  丢包: %llu\n",
            (unsigned long long)(stats.count + stats.lost),
            (unsigned long long)stats.count,
            (unsigned long long)stats.lost);
    if (stats.count > 0) {
        double loss_rate = (double)stats.lost / (double)(stats.count + stats.lost) * 100.0;
        fprintf(stdout, "  丢包率: %.1f%%\n", loss_rate);
        fprintf(stdout, "  总耗时: %.1fs  帧率: %.1f fps\n", total_s, stats.count / total_s);
        fprintf(stdout, "  -------------------------------------\n");
        fprintf(stdout, "  最小延迟: %.3f ms\n", stats.min_ms);
        fprintf(stdout, "  最大延迟: %.3f ms\n", stats.max_ms);
        fprintf(stdout, "  平均延迟: %.3f ms\n", stats.sum_ms / stats.count);
        if (stats.count > 1) {
            double var = stats.sum_sq_ms / stats.count - 
                         (stats.sum_ms / stats.count) * (stats.sum_ms / stats.count);
            if (var < 0) var = 0;
            fprintf(stdout, "  抖动 (σ): %.3f ms\n", sqrt(var));
        }
    }
    fprintf(stdout, "==============================================\n");

    return 0;
}
