#pragma once
#include <cstdint>
#include <cstring>
#include <vector>
#include <stdexcept>

namespace nlink {

// ========== 常量 ==========
constexpr uint8_t HEADER_WRITE = 0x54;
constexpr uint8_t HEADER_READ  = 0x55;
constexpr uint8_t MARK_USER_FRAME1 = 0xF1;

enum Role : uint8_t { NODE=0, ANCHOR=1, TAG=2, CONSOLE=3, MASTER=4, SLAVE=5 };

// ========== 校验和 ==========
inline uint8_t checksum(const uint8_t* data, size_t len) {
    uint32_t sum = 0;
    for (size_t i = 0; i < len; ++i) sum += data[i];
    return sum & 0xFF;
}

// ========== User_Frame1 构造 ==========
inline std::vector<uint8_t> build_user_frame1(uint8_t remote_role, uint8_t remote_id,
                                               const uint8_t* payload, uint16_t payload_len) {
    // 帧结构: 0x54 F1 [reserved 4B=0xFF] [role] [id] [len LE] [data] [checksum]
    size_t total = 11 + payload_len;
    std::vector<uint8_t> buf(total);
    buf[0] = HEADER_WRITE;
    buf[1] = MARK_USER_FRAME1;
    buf[2] = buf[3] = buf[4] = buf[5] = 0xFF;
    buf[6] = remote_role;
    buf[7] = remote_id;
    buf[8] = payload_len & 0xFF;
    buf[9] = (payload_len >> 8) & 0xFF;
    if (payload_len > 0) std::memcpy(&buf[10], payload, payload_len);
    buf[total - 1] = checksum(buf.data(), total - 1);
    return buf;
}

inline std::vector<uint8_t> build_broadcast_frame(const uint8_t* payload, uint16_t len) {
    return build_user_frame1(NODE, 0xFF, payload, len);
}

inline std::vector<uint8_t> build_unicast_frame(uint8_t slave_id, const uint8_t* payload, uint16_t len) {
    return build_user_frame1(SLAVE, slave_id, payload, len);
}

// ========== DT Payload (带 sync 头的透传载荷) ==========
constexpr uint8_t  SYNC0 = 0xA5;
constexpr uint8_t  SYNC1 = 0x5A;
constexpr uint16_t PAYLOAD_HEADER_SIZE = 12; // sync(2) + seq(2) + ts(8)

inline void pack_payload(uint8_t* dst, uint16_t data_size, uint16_t seq, double timestamp) {
    dst[0] = SYNC0;
    dst[1] = SYNC1;
    dst[2] = seq & 0xFF;
    dst[3] = (seq >> 8) & 0xFF;
    std::memcpy(&dst[4], &timestamp, 8);
    if (data_size > PAYLOAD_HEADER_SIZE)
        std::memset(dst + PAYLOAD_HEADER_SIZE, 0, data_size - PAYLOAD_HEADER_SIZE);
}

inline bool unpack_payload(const uint8_t* data, size_t len, uint16_t& seq, double& ts) {
    if (len < PAYLOAD_HEADER_SIZE) return false;
    if (data[0] != SYNC0 || data[1] != SYNC1) return false;
    seq = data[2] | (data[3] << 8);
    std::memcpy(&ts, &data[4], 8);
    return true;
}

} // namespace nlink
