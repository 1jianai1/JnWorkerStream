"""模拟设备传输（无需硬件）。

用途：

- 没有板子时演示完整输出格式（``FRAME`` / ``DATA`` / ``STAT``）；
- 复现验收项 H5/H6/H7/H8：拆包、粘包、插入脏字节、制造 CRC 错误；
- 回环：主机写入的 TEST_DATA 会按设备规则回一条（seq 不变、payload flags 置 ECHO）。

它模拟的是设备端 ``test_src`` 的字节，不参与任何产品逻辑。
"""

from __future__ import annotations

import queue
import random
import threading
import time
from typing import List, Optional

from ..frame import (
    SYNC,
    TEST_FLAG_ECHO,
    TYPE_TEST_DATA,
    Frame,
    FrameParser,
    build_frame,
    build_test_payload,
)
from .base import Transport, TransportError

ANOMALIES = ("none", "split", "sticky", "dirty", "badcrc", "all")

MAX_SIM_SIZE = 1000


def make_test_data(seq: int, size: int, pattern: str) -> bytes:
    """按设备端 test_src 的三种模式生成确定性数据。"""
    if pattern == "inc":
        return bytes(((seq * 7 + i) & 0xFF) for i in range(size))
    if pattern == "ascii":
        if size <= 0:
            return b""
        base = f"CUSTOM_DATA_TEST_{seq % 10000:04d}|".encode("ascii")
        return (base * (size // len(base) + 2))[:size]
    if pattern == "zero":
        return bytes(size)
    raise ValueError(f"sim pattern 仅支持 inc/ascii/zero，收到 {pattern!r}")


class SimTransport(Transport):
    """确定性测试源 + 可选链路异常注入。"""

    name = "SIM"
    auto_reconnect = True

    def __init__(
        self,
        rate_hz: float = 20.0,
        size: int = 64,
        pattern: str = "inc",
        anomaly: str = "none",
        burst: int = 0,
        mtu: int = 1024,
        seed: int = 20251026,
        echo: bool = True,
    ) -> None:
        super().__init__()
        if anomaly not in ANOMALIES:
            raise ValueError(f"anomaly 可选 {ANOMALIES}，收到 {anomaly!r}")
        if pattern not in ("inc", "ascii", "zero"):
            raise ValueError("sim pattern 仅支持 inc/ascii/zero")
        self.rate_hz = max(0.0, float(rate_hz))
        self.size = max(0, min(int(size), MAX_SIM_SIZE))
        self.pattern = pattern
        self.anomaly = anomaly
        self.burst = max(0, int(burst))
        self.echo = bool(echo)
        self._mtu = max(32, int(mtu))
        self._random = random.Random(seed)

        self._thread: Optional[threading.Thread] = None
        self._running = False
        self._seq = 0
        self._sent = 0
        self._tx_in: "queue.Queue[bytes]" = queue.Queue(maxsize=256)
        self._sticky: List[bytes] = []
        self._parser = FrameParser(self._on_host_frame)
        self._started_at = 0.0

    # -- 能力 -------------------------------------------------------------

    @property
    def mtu(self) -> int:
        return self._mtu

    @property
    def exhausted(self) -> bool:
        """有限突发模式是否已经发完（便于 CLI 自动收尾）。"""
        return self.burst > 0 and self._sent >= self.burst and self._tx_in.empty()

    # -- 生命周期 ---------------------------------------------------------

    def open(self) -> None:
        if self._running:
            return
        self._running = True
        self._started_at = time.monotonic()
        self._thread = threading.Thread(target=self._run, name="sim-tx", daemon=True)
        self._thread.start()
        self._connected = True
        self._emit(
            f"LINK  sim device rate={self.rate_hz:g}Hz size={self.size}B "
            f"pattern={self.pattern} anomaly={self.anomaly} mtu={self.mtu}"
        )

    def close(self) -> None:
        self._running = False
        self._connected = False
        thread, self._thread = self._thread, None
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=2.0)
        self._emit("LINK  closed")

    # -- 发送（主机 -> 模拟设备，只用于回环）-------------------------------

    def write(self, data: bytes) -> int:
        if not self._connected:
            raise TransportError("SIM 未连接")
        if len(data) > self._mtu:
            raise TransportError(f"帧 {len(data)} B 超过模拟链路 MTU {self._mtu} B")
        try:
            self._tx_in.put_nowait(bytes(data))
        except queue.Full:
            self._emit("SIM  WARN: echo queue full, drop")
            return 0
        return len(data)

    # -- 模拟设备线程 -----------------------------------------------------

    def _dev_ms(self) -> int:
        return int((time.monotonic() - self._started_at) * 1000.0) & 0xFFFFFFFF

    def _on_host_frame(self, frame: Frame) -> None:
        if not self.echo:
            return
        if frame.type != TYPE_TEST_DATA:
            return
        # 与设备端 test_on_frame_rx 一致：data 原样返回，seq 不变，payload flags 置 ECHO
        payload = build_test_payload(self._dev_ms(), frame.data, TEST_FLAG_ECHO)
        reply = build_frame(TYPE_TEST_DATA, 0, frame.seq, payload)
        self._deliver_chunk(reply)

    def _drain_tx(self) -> None:
        while True:
            try:
                raw = self._tx_in.get_nowait()
            except queue.Empty:
                return
            self._parser.feed(raw)

    def _deliver_chunk(self, chunk: bytes) -> None:
        if chunk:
            self._deliver(chunk)

    def _frame_bytes(self, seq: int) -> bytes:
        data = make_test_data(seq, self.size, self.pattern)
        payload = build_test_payload(self._dev_ms(), data, 0)
        return build_frame(TYPE_TEST_DATA, 0, seq, payload)

    def _inject(self, frame: bytes, index: int) -> None:
        """按 anomaly 注入拆包/粘包/脏字节/CRC 错。"""
        mode = self.anomaly
        if mode in ("badcrc", "all") and index % 17 == 5:
            frame = frame[:-1] + bytes((frame[-1] ^ 0xFF,))
        if mode in ("dirty", "all") and index % 11 == 0:
            frame = b"\x00" + frame
        if mode in ("split", "all") and index % 4 == 1:
            cut1 = max(1, len(frame) // 3)
            cut2 = max(cut1 + 1, (len(frame) * 2) // 3)
            self._deliver_chunk(frame[:cut1])
            time.sleep(0.001)
            self._deliver_chunk(frame[cut1:cut2])
            time.sleep(0.001)
            self._deliver_chunk(frame[cut2:])
            return
        if mode in ("sticky", "all") and index % 3 != 2:
            self._sticky.append(frame)
            return
        if self._sticky:
            self._sticky.append(frame)
            frame = b"".join(self._sticky)
            self._sticky.clear()
        self._deliver_chunk(frame)

    def _run(self) -> None:
        period = 1.0 / self.rate_hz if self.rate_hz > 0 else 0.0
        next_at = time.monotonic()
        while self._running:
            self._drain_tx()

            if self.rate_hz <= 0 or (self.burst and self._sent >= self.burst):
                time.sleep(0.01)
                continue

            now = time.monotonic()
            if now < next_at:
                time.sleep(min(0.02, next_at - now))
                continue
            next_at = max(next_at + period, now)

            frame = self._frame_bytes(self._seq)
            self._inject(frame, self._sent % 3600)
            self._seq = (self._seq + 1) & 0xFFFF
            self._sent += 1

        self._drain_tx()


# 单帧 data 的合理上限（payload 仍受 1024 B 协议上限约束）
