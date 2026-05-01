"""
NLink 协议工具模块 (V1.4)
用于 LinkTrack UWB 设备的协议帧构造与解析

协议参考: NLink_V1.4.txt, LinkTrack_User_Manual_V2.3_zh.txt
"""

from enum import IntEnum
from dataclasses import dataclass
from typing import List, Optional, Tuple
import struct


# =============================================================================
# 常量定义
# =============================================================================

class Role(IntEnum):
    """角色枚举 (Role Table, NLink V1.4)"""
    NODE    = 0x00
    ANCHOR  = 0x01
    TAG     = 0x02
    CONSOLE = 0x03
    MASTER  = 0x04
    SLAVE   = 0x05


class FrameMark(IntEnum):
    """协议帧功能码"""
    ANCHOR_FRAME0 = 0x00
    TAG_FRAME0    = 0x01
    NODE_FRAME0   = 0x02
    NODE_FRAME1   = 0x03
    NODE_FRAME2   = 0x04
    NODE_FRAME3   = 0x05
    NODE_FRAME4   = 0x06
    NODE_FRAME5   = 0x08
    NODE_FRAME6   = 0x09
    USER_FRAME1   = 0xF1
    ERROR_FRAME0  = 0xFA


# 帧头
FRAME_HEADER_READ  = 0x55  # 只读输出帧
FRAME_HEADER_WRITE = 0x54  # 读写帧
FRAME_HEADER_SYSTEM = 0x52  # 系统帧


# =============================================================================
# 工具函数
# =============================================================================

def calc_checksum(data: bytes) -> int:
    """计算单字节累加和校验 (所有字节相加取低8位)"""
    return sum(data) & 0xFF


def verify_checksum(data: bytes) -> bool:
    """验证校验和：帧数据(含末尾校验字节)的累加和应为0（低8位）"""
    if len(data) < 2:
        return False
    return calc_checksum(data[:-1]) == data[-1]


def int24_to_int32(data: bytes, offset: int = 0, little_endian: bool = True) -> int:
    """
    将3字节有符号整数(int24)转换为Python int
    NLink协议中小端存储，需左移8位后算术右移8位保持符号
    """
    if little_endian:
        b0, b1, b2 = data[offset], data[offset + 1], data[offset + 2]
    else:
        b0, b1, b2 = data[offset + 2], data[offset + 1], data[offset]
    # 拼接为32位，左移8位后算术右移8位
    raw = (b0 << 8) | (b1 << 16) | (b2 << 24)
    # Python整数是无限精度，模拟有符号32位算术右移
    if raw & 0x80000000:
        raw = raw - 0x100000000
    return raw >> 8


def uint24_to_int(data: bytes, offset: int = 0, little_endian: bool = True) -> int:
    """将3字节无符号整数(uint24)转换为Python int"""
    if little_endian:
        return data[offset] | (data[offset + 1] << 8) | (data[offset + 2] << 16)
    else:
        return (data[offset] << 16) | (data[offset + 1] << 8) | data[offset + 2]


# =============================================================================
# User_Frame1 构造（DT_MODE0 MASTER → 数传输入）
# =============================================================================

@dataclass
class UserFrame1:
    """
    NLink_LinkTrack_User_Frame1 (DT_MODE0 数传输入帧)
    长度: 11 + data_length 字节, WO (只写)

    帧结构:
      0x54 F1 [reserved 4B 0xFF] [remote_role 1B] [remote_id 1B]
      [data_length 2B LE] [data N bytes] [checksum 1B]
    """
    remote_role: int  # NODE(广播) 或 SLAVE(定向)
    remote_id: int    # 广播时为0xFF, 定向时为SLAVE ID
    data: bytes       # 透传数据

    @staticmethod
    def broadcast(data: bytes) -> 'UserFrame1':
        """广播模式：向所有SLAVE发送"""
        return UserFrame1(remote_role=Role.NODE, remote_id=0xFF, data=data)

    @staticmethod
    def unicast(slave_id: int, data: bytes) -> 'UserFrame1':
        """定向模式：向指定SLAVE发送"""
        return UserFrame1(remote_role=Role.SLAVE, remote_id=slave_id, data=data)

    def to_bytes(self) -> bytes:
        """序列化为协议帧字节"""
        buf = bytearray()
        buf.append(FRAME_HEADER_WRITE)
        buf.append(FrameMark.USER_FRAME1)
        buf.extend(b'\xFF\xFF\xFF\xFF')
        buf.append(self.remote_role & 0xFF)
        buf.append(self.remote_id & 0xFF)
        buf.extend(struct.pack('<H', len(self.data)))
        buf.extend(self.data)
        buf.append(calc_checksum(buf))
        return bytes(buf)


# =============================================================================
# Node_Frame0 解析（数传接收帧）
# =============================================================================

@dataclass
class NodeFrame0Block:
    """Node_Frame0 中的每个 Block"""
    role: int
    node_id: int
    data: bytes


@dataclass
class NodeFrame0:
    """
    NLink_LinkTrack_Node_Frame0 (数传输出帧, 变长)
    帧结构:
      0x55 0x02 [frame_length 2B LE] [role 1B] [id 1B]
      [reserved 4B] [valid_node_quantity 1B]
      [Block0] [Block1] ...
      [checksum 1B]

    每个 Block:
      [role 1B] [id 1B] [data_length 2B LE] [data N bytes]
    """
    role: int
    node_id: int
    blocks: List[NodeFrame0Block]
    raw: bytes

    @staticmethod
    def parse(data: bytes) -> Optional['NodeFrame0']:
        """从原始字节解析 Node_Frame0"""
        if len(data) < 13:
            return None
        if data[0] != FRAME_HEADER_READ or data[1] != FrameMark.NODE_FRAME0:
            return None

        frame_length = struct.unpack_from('<H', data, 2)[0]
        total_expected = 4 + frame_length
        if len(data) < total_expected:
            return None

        if not verify_checksum(data[:total_expected]):
            return None

        offset = 4
        role = data[offset]; offset += 1
        node_id = data[offset]; offset += 1
        offset += 4
        valid_count = data[offset]; offset += 1

        blocks = []
        for _ in range(valid_count):
            if offset + 4 > len(data):
                break
            block_role = data[offset]; offset += 1
            block_id = data[offset]; offset += 1
            block_len = struct.unpack_from('<H', data, offset)[0]; offset += 2
            if offset + block_len > len(data):
                break
            block_data = data[offset:offset + block_len]; offset += block_len
            blocks.append(NodeFrame0Block(role=block_role, node_id=block_id, data=block_data))

        return NodeFrame0(
            role=role,
            node_id=node_id,
            blocks=blocks,
            raw=data[:total_expected],
        )


# =============================================================================
# 帧同步器 — 从串口流中提取完整帧
# =============================================================================

class FrameSynchronizer:
    """
    从串口字节流中提取以 0x55 为帧头的完整协议帧
    支持变长帧，通过帧长度字段确定帧边界
    """

    # 各帧类型的 (帧头, 功能码) → 长度策略
    # 0: 定长, 1: 通过frame_length字段, 2: 固定末尾校验
    FRAME_LENGTH_MAP: dict = {}  # 在 __init__ 中初始化

    def __init__(self):
        self._buffer = bytearray()
        # 预计算各帧类型的固定长度
        self._FIXED_LENGTHS = {
            (FRAME_HEADER_READ, FrameMark.ANCHOR_FRAME0): 896,   # Anchor_Frame0
            (FRAME_HEADER_READ, FrameMark.TAG_FRAME0): 128,      # Tag_Frame0
            (FRAME_HEADER_WRITE, FrameMark.ERROR_FRAME0): 32,    # Error_Frame0
            (FRAME_HEADER_SYSTEM, 0x00): 32,                     # System_Common_Frame0
        }

    def feed(self, data: bytes) -> List[Tuple[int, bytes]]:
        """喂入原始字节，返回已提取的完整帧列表 [(帧头, 帧数据), ...]"""
        self._buffer.extend(data)
        frames = []

        while len(self._buffer) >= 2:
            header_pos = -1
            for i in range(len(self._buffer) - 1):
                if self._buffer[i] in (FRAME_HEADER_READ, FRAME_HEADER_WRITE, FRAME_HEADER_SYSTEM):
                    header_pos = i
                    break

            if header_pos == -1:
                self._buffer = self._buffer[-1:]
                break

            if header_pos > 0:
                del self._buffer[:header_pos]

            if len(self._buffer) < 4:
                break

            header = self._buffer[0]
            mark = self._buffer[1]

            fixed_key = (header, mark)
            if fixed_key in self._FIXED_LENGTHS:
                expected_len = self._FIXED_LENGTHS[fixed_key]
                if len(self._buffer) >= expected_len:
                    frame = bytes(self._buffer[:expected_len])
                    frames.append((header, frame))
                    del self._buffer[:expected_len]
                else:
                    break
                continue

            frame_content_len = struct.unpack_from('<H', self._buffer, 2)[0]
            total_len = 4 + frame_content_len

            if total_len < 7 or total_len > 4096:
                del self._buffer[:1]
                continue

            if len(self._buffer) >= total_len:
                frame = bytes(self._buffer[:total_len])
                frames.append((header, frame))
                del self._buffer[:total_len]
            else:
                break

        return frames

    def reset(self):
        """清空缓冲区"""
        self._buffer.clear()


# =============================================================================
# DT_MODE0 透传载荷 (SLAVE 侧输出为原始字节，无协议帧包装)
# 载荷格式: [sync 0xA5 0x5A] [seq 2B LE] [timestamp 8B double LE] [padding]
# =============================================================================

DT_PAYLOAD_SYNC = b'\xA5\x5A'
DT_PAYLOAD_MIN_SIZE = 12  # sync(2) + seq(2) + timestamp(8)


def dt_payload_pack(seq: int, timestamp: float, data_size: int = 16) -> bytes:
    """构造 DT_MODE0 透传载荷"""
    payload = bytearray()
    payload.extend(DT_PAYLOAD_SYNC)
    payload.extend(struct.pack('<H', seq & 0xFFFF))
    payload.extend(struct.pack('<d', timestamp))
    if len(payload) < data_size:
        payload.extend(b'\x00' * (data_size - len(payload)))
    return bytes(payload[:data_size])


def dt_payload_unpack(data: bytes) -> Optional[Tuple[int, float]]:
    """解析 DT_MODE0 透传载荷，返回 (seq, timestamp) 或 None"""
    if len(data) < DT_PAYLOAD_MIN_SIZE:
        return None
    if data[0:2] != DT_PAYLOAD_SYNC:
        return None
    seq = struct.unpack_from('<H', data, 2)[0]
    timestamp = struct.unpack_from('<d', data, 4)[0]
    return (seq, timestamp)


class DTPayloadSynchronizer:
    """从 SLAVE 原始字节流中按 sync 头提取透传载荷"""

    def __init__(self, payload_size: int):
        self.payload_size = payload_size
        self._buffer = bytearray()

    def feed(self, raw: bytes) -> List[Tuple[int, float]]:
        """喂入原始字节，返回 [(seq, timestamp), ...]"""
        self._buffer.extend(raw)
        results = []

        while len(self._buffer) >= self.payload_size:
            pos = self._buffer.find(DT_PAYLOAD_SYNC)
            if pos == -1:
                self._buffer.clear()
                break
            if pos > 0:
                del self._buffer[:pos]
            if len(self._buffer) < self.payload_size:
                break
            payload = self._buffer[:self.payload_size]
            del self._buffer[:self.payload_size]
            parsed = dt_payload_unpack(payload)
            if parsed:
                results.append(parsed)
            # sync头对不上的帧丢弃（可能是数据损坏导致的偏移）

        return results

    def reset(self):
        self._buffer.clear()
