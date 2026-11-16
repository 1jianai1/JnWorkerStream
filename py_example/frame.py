"""统一帧协议（基准实现，来源：docs/上位机通信验证.md 第 3、4 章）。

帧布局（所有多字节整数均为小端）::

    0    1    2    3     4      5  6      7  8      9 ...          末 2
    +----+----+----+----+------+------+------+------+----------+------+
    | A5 | 5A |ver |type|flags |seq lo|seq hi|len lo|len hi| payload | crc16|
    +----+----+----+----+------+------+------+------+----------+------+
    |<----------------- 9 B 帧头 ----------------->|<-- len -->|<-2B->|

- 整帧长度 = ``11 + len``；
- CRC-16/CCITT-FALSE（poly=0x1021, init=0xFFFF, 不反射, xorout=0），
  覆盖 ``frame[2 : 9+len]``；
- 自检向量：``crc16(b"123456789") == 0x29B1``、``crc16(b"") == 0xFFFF``。

本模块**不含任何传输/线程相关代码**，USB、BLE、模拟链路共用。
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Callable, Dict, Optional

# ---------------------------------------------------------------------------
# 常量
# ---------------------------------------------------------------------------

SYNC1 = 0xA5
SYNC2 = 0x5A
SYNC = b"\xA5\x5A"

PROTO_VER = 0x01

HEADER_LEN = 9          # A5 5A + ver type flags seq(2) len(2)
CRC_LEN = 2
OVERHEAD = HEADER_LEN + CRC_LEN   # 11

MAX_PAYLOAD = 1024      # 协议绝对上限，同时受链路 MTU 约束
MAX_BUFFER = 64 * 1024  # 缓冲上限，防止脏数据把内存吃满（文档 §9）

TYPE_AUDIO_DATA = 0x01
TYPE_AUDIO_CTL = 0x02
TYPE_TEST_DATA = 0x60

#: 解析器认识的帧类型；其余类型按 unknown_type 计数后丢弃。
KNOWN_TYPES = frozenset((TYPE_AUDIO_DATA, TYPE_AUDIO_CTL, TYPE_TEST_DATA))

TYPE_NAMES: Dict[int, str] = {
    TYPE_AUDIO_DATA: "AUDIO_DATA",
    TYPE_AUDIO_CTL: "AUDIO_CTL",
    TYPE_TEST_DATA: "TEST_DATA",
}

# 帧头 flags：bit0=需要 ACK，bit2=重传（bit1 保留，link 不分片）
FRAME_FLAG_ACK = 0x01
FRAME_FLAG_RETRANSMIT = 0x04

# TEST_DATA payload 内的 flags：bit0=ECHO（设备对主机数据的回显）
TEST_FLAG_ECHO = 0x01


def crc16(data: bytes) -> int:
    """CRC-16/CCITT-FALSE，逐位实现（与设备端 C 实现保持一致）。"""
    crc = 0xFFFF
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x1021) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def build_frame(
    frame_type: int,
    flags: int,
    seq: int,
    payload: bytes = b"",
    ver: int = PROTO_VER,
) -> bytes:
    """组装一帧完整字节（含同步头与 CRC）。"""
    if not 0 <= frame_type <= 0xFF:
        raise ValueError(f"type 超出 1 字节: {frame_type!r}")
    if not 0 <= flags <= 0xFF:
        raise ValueError(f"flags 超出 1 字节: {flags!r}")
    if not 0 <= seq <= 0xFFFF:
        raise ValueError(f"seq 超出 2 字节: {seq!r}")
    payload = bytes(payload)
    if len(payload) > MAX_PAYLOAD:
        raise ValueError(
            f"payload {len(payload)} B 超过协议上限 {MAX_PAYLOAD} B，应由上层切分"
        )
    body = (
        bytes((ver & 0xFF, frame_type & 0xFF, flags & 0xFF))
        + seq.to_bytes(2, "little")
        + len(payload).to_bytes(2, "little")
        + payload
    )
    return SYNC + body + crc16(body).to_bytes(2, "little")


def build_test_payload(dev_ms: int, data: bytes, flags: int = 0) -> bytes:
    """组装 TEST_DATA 的 payload：dev_ms(4) + flags(1) + data(n)。"""
    return (dev_ms & 0xFFFFFFFF).to_bytes(4, "little") + bytes((flags & 0xFF,)) + bytes(data)


@dataclass(frozen=True, slots=True)
class Frame:
    """一条通过 CRC 校验的帧。"""

    ver: int
    type: int
    flags: int
    seq: int
    payload: bytes
    crc: int = 0
    raw: bytes = field(default=b"", repr=False)

    @property
    def type_name(self) -> str:
        return TYPE_NAMES.get(self.type, f"0x{self.type:02X}")

    @property
    def is_echo(self) -> bool:
        """TEST_DATA 回显帧（payload flags bit0）。"""
        return (
            self.type == TYPE_TEST_DATA
            and len(self.payload) >= 5
            and bool(self.payload[4] & TEST_FLAG_ECHO)
        )

    @property
    def dev_ms(self) -> Optional[int]:
        if self.type == TYPE_TEST_DATA and len(self.payload) >= 5:
            return int.from_bytes(self.payload[0:4], "little")
        return None

    @property
    def data(self) -> bytes:
        """TEST_DATA 的 data 部分（跳过 dev_ms + flags）。"""
        if self.type == TYPE_TEST_DATA and len(self.payload) >= 5:
            return self.payload[5:]
        return b""

    def __len__(self) -> int:
        return OVERHEAD + len(self.payload)


class FrameParser:
    """环形缓冲 + 游标的流式帧解析器。

    设计要点（与文档 §4 一致）：

    - 支持拆包/粘包：一帧可跨多次 ``feed()``，一次 ``feed()`` 也可含多帧；
    - 错帧重同步：CRC 错 / 版本不认 / 超长时**只丢 1 字节**再找 ``A5 5A``；
    - 未知 type：完整帧跳过并计 ``unknown_type``，不影响后续帧；
    - ``stats`` 字段：frames / bytes / crc_err / oversize / unknown_type / resync。

    回调 ``on_frame(Frame)`` 只在 CRC 正确且 type==0x60 时触发（未知类型只计数）。
    解析器本身不是线程安全的：同一实例只应由一个线程 ``feed()``。
    """

    def __init__(
        self,
        on_frame: Callable[[Frame], None],
        max_payload: int = MAX_PAYLOAD,
        max_buffer: int = MAX_BUFFER,
    ) -> None:
        if max_payload <= 0:
            raise ValueError("max_payload 必须为正数")
        self._on_frame = on_frame
        self.max_payload = min(int(max_payload), MAX_PAYLOAD)
        self.max_buffer = int(max_buffer)
        self._buf = bytearray()
        self._pos = 0
        self.stats = {
            "frames": 0,
            "bytes": 0,
            "crc_err": 0,
            "oversize": 0,
            "unknown_type": 0,
            "resync": 0,
            "overflow": 0,
        }

    # -- 对外接口 ---------------------------------------------------------

    def feed(self, data: bytes) -> None:
        """向解析器投喂任意长度的字节流。"""
        if not data:
            return
        self.stats["bytes"] += len(data)
        self._buf += data
        self._parse()

    def reset(self) -> None:
        """清空未完成的半帧（重连/换链路时调用，不清统计）。"""
        self._buf.clear()
        self._pos = 0

    def stats_snapshot(self) -> Dict[str, int]:
        return dict(self.stats)

    @property
    def pending(self) -> int:
        """缓冲中尚未解析的字节数。"""
        return len(self._buf) - self._pos

    # -- 内部实现 ---------------------------------------------------------

    def _drop_byte(self, counter: str) -> None:
        self.stats[counter] += 1
        self._pos += 1

    def _parse(self) -> None:
        buf = self._buf
        while True:
            avail = len(buf) - self._pos
            if avail < 2:
                break

            idx = buf.find(SYNC, self._pos)
            if idx < 0:
                # 没有同步头。若最后一个字节可能是半帧头则保留，其余计入 resync。
                keep = 1 if buf[-1] == SYNC1 else 0
                drop = avail - keep
                if drop > 0:
                    self.stats["resync"] += drop
                    self._pos += drop
                break

            if idx > self._pos:
                self.stats["resync"] += idx - self._pos
                self._pos = idx

            avail = len(buf) - self._pos
            if avail < HEADER_LEN:
                break

            p = self._pos
            ver = buf[p + 2]
            typ = buf[p + 3]
            flags = buf[p + 4]
            seq = buf[p + 5] | (buf[p + 6] << 8)
            length = buf[p + 7] | (buf[p + 8] << 8)

            if ver != PROTO_VER:
                self._drop_byte("resync")
                continue
            if length > self.max_payload:
                self._drop_byte("oversize")
                continue

            total = OVERHEAD + length
            if avail < total:
                break

            body = bytes(buf[p + 2 : p + 9 + length])
            got_crc = buf[p + 9 + length] | (buf[p + 10 + length] << 8)
            calc_crc = crc16(body)
            if got_crc != calc_crc:
                self._drop_byte("crc_err")
                continue

            payload = bytes(buf[p + 9 : p + 9 + length])
            raw = bytes(buf[p : p + total])
            self._pos += total
            self.stats["frames"] += 1

            if typ not in KNOWN_TYPES:
                self.stats["unknown_type"] += 1
                continue

            self._on_frame(
                Frame(
                    ver=ver,
                    type=typ,
                    flags=flags,
                    seq=seq,
                    payload=payload,
                    crc=calc_crc,
                    raw=raw,
                )
            )

        self._compact()

    def _compact(self) -> None:
        pending = len(self._buf) - self._pos
        if pending > self.max_buffer:
            # 正常情况下每次 feed 都会把完整帧全部消费掉，pending 不应超过 1 帧。
            # 真的超限说明流已经不可信：整体丢弃并重新同步。
            self.stats["overflow"] += 1
            self.stats["resync"] += pending
            self._buf.clear()
            self._pos = 0
            return
        if self._pos == 0:
            return
        if self._pos == len(self._buf):
            self._buf.clear()
            self._pos = 0
        elif self._pos >= 4096:
            del self._buf[: self._pos]
            self._pos = 0
