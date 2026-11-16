"""USB CDC 传输（pyserial）。

设备侧对应文档 2 的 ``link_usb``：Windows 上枚举出一个 COM 口，二进制帧走 Bulk。
本模块只负责打开 COM、后台 ``read()`` 后回调、``write()`` 写回，不解析协议。

依赖：``pip install pyserial``。
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass
from typing import List, Optional

from .base import Transport, TransportError

try:  # pyserial 是可选依赖：没装也能运行自检/--compare
    import serial  # type: ignore
    from serial.tools import list_ports  # type: ignore
except ImportError:  # pragma: no cover - 环境相关
    serial = None  # type: ignore[assignment]
    list_ports = None  # type: ignore[assignment]


@dataclass(slots=True)
class SerialPortInfo:
    device: str
    description: str = ""
    hwid: str = ""
    vid: Optional[int] = None
    pid: Optional[int] = None
    serial_number: Optional[str] = None
    manufacturer: Optional[str] = None
    product: Optional[str] = None

    def pretty(self) -> str:
        ids = ""
        if self.vid is not None and self.pid is not None:
            ids = f" [{self.vid:04X}:{self.pid:04X}]"
        extra = ""
        if self.serial_number:
            extra = f" sn={self.serial_number}"
        return f"{self.device}{ids} {self.description or self.product or ''}{extra}".rstrip()


def list_serial_ports() -> List[SerialPortInfo]:
    """列出当前系统所有串口（未安装 pyserial 时抛 TransportError）。"""
    if serial is None or list_ports is None:
        raise TransportError(
            "未安装 pyserial，无法枚举/打开串口。请执行: pip install -r host_test/requirements.txt"
        )
    ports: List[SerialPortInfo] = []
    for p in list_ports.comports():
        ports.append(
            SerialPortInfo(
                device=p.device,
                description=p.description or "",
                hwid=p.hwid or "",
                vid=p.vid,
                pid=p.pid,
                serial_number=p.serial_number,
                manufacturer=p.manufacturer,
                product=p.product,
            )
        )
    return sorted(ports, key=lambda x: x.device)


class SerialTransport(Transport):
    """USB CDC（虚拟串口）传输。"""

    name = "USB-CDC"
    auto_reconnect = False

    def __init__(
        self,
        port: Optional[str] = None,
        baudrate: int = 115200,
        vid: Optional[int] = None,
        pid: Optional[int] = None,
        serial_number: Optional[str] = None,
        description: Optional[str] = None,
        read_size: int = 4096,
        read_timeout: float = 0.05,
    ) -> None:
        super().__init__()
        self.port = port
        self.baudrate = int(baudrate)
        self.vid = vid
        self.pid = pid
        self.serial_number = serial_number
        self.description_filter = description
        self.read_size = max(64, int(read_size))
        self.read_timeout = float(read_timeout)

        self._ser: Optional["serial.Serial"] = None  # type: ignore[name-defined]
        self._thread: Optional[threading.Thread] = None
        self._running = False
        self._lock = threading.RLock()
        self._disconnect_reported = False
        self._resolved_device: Optional[str] = None

    # -- 能力 -------------------------------------------------------------

    @property
    def mtu(self) -> int:
        return 1024

    # -- 打开/关闭 ---------------------------------------------------------

    def _resolve_port(self) -> str:
        if self.port:
            return self.port
        if serial is None:
            raise TransportError("未安装 pyserial，请执行: pip install pyserial")

        candidates = list_serial_ports()
        if self.vid is not None:
            candidates = [p for p in candidates if p.vid == self.vid]
        if self.pid is not None:
            candidates = [p for p in candidates if p.pid == self.pid]
        if self.serial_number:
            candidates = [p for p in candidates if p.serial_number == self.serial_number]
        if self.description_filter:
            needle = self.description_filter.lower()
            candidates = [
                p
                for p in candidates
                if needle in (p.description or "").lower()
                or needle in (p.product or "").lower()
                or needle in p.device.lower()
            ]

        if not candidates:
            available = ", ".join(p.pretty() for p in list_serial_ports()) or "(无)"
            raise TransportError(
                "未找到匹配的串口。请用 --port 指定，或检查 USB 连接。当前串口: " + available
            )
        if len(candidates) > 1:
            available = ", ".join(p.pretty() for p in candidates)
            raise TransportError(
                f"匹配到多个串口，请用 --port 明确指定: {available}"
            )
        return candidates[0].device

    def open(self) -> None:
        if serial is None:
            raise TransportError(
                "未安装 pyserial，无法打开 USB CDC。请执行: "
                "pip install -r host_test/requirements.txt"
            )
        with self._lock:
            if self._connected:
                return
            device = self._resolve_port()
            try:
                ser = serial.Serial(
                    port=device,
                    baudrate=self.baudrate,
                    timeout=self.read_timeout,
                    write_timeout=2.0,
                )
            except (serial.SerialException, OSError, ValueError) as exc:
                raise TransportError(f"打开串口 {device} 失败: {exc}") from exc

            # CDC 不依赖波特率，但清空上电/上次残留的缓冲，避免半帧脏数据。
            try:
                ser.reset_input_buffer()
                ser.reset_output_buffer()
            except (serial.SerialException, OSError):
                pass

            self._ser = ser
            self._resolved_device = device
            self._connected = True
            self._running = True
            self._disconnect_reported = False
            self._thread = threading.Thread(
                target=self._reader_loop,
                name=f"usb-rx-{device}",
                daemon=True,
            )
            self._thread.start()
        self._emit(f"LINK  open {device} ({self.baudrate} baud) mtu={self.mtu}")

    def close(self) -> None:
        with self._lock:
            was_connected = self._connected
            self._running = False
            self._connected = False
            ser, self._ser = self._ser, None
            thread = self._thread
            self._thread = None
        if ser is not None:
            try:
                ser.close()
            except Exception:
                pass
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=1.0)
        if was_connected:
            self._emit("LINK  closed")

    # -- 接收线程 ---------------------------------------------------------

    def _reader_loop(self) -> None:
        while self._running:
            ser = self._ser
            if ser is None:
                break
            try:
                data = ser.read(self.read_size)
            except (serial.SerialException, OSError, TypeError) as exc:
                if self._running:
                    self._fail(f"read error: {exc}")
                break
            except Exception as exc:  # 不让后台线程静默死掉
                if self._running:
                    self._fail(f"unexpected read error: {exc}")
                break
            if data:
                self._deliver(bytes(data))
            else:
                # 正常超时，让出 CPU（read(timeout) 本身已经阻塞）
                time.sleep(0)

    def _fail(self, reason: str) -> None:
        with self._lock:
            if self._disconnect_reported:
                return
            self._disconnect_reported = True
            self._running = False
            self._connected = False
            ser, self._ser = self._ser, None
            if ser is not None:
                try:
                    ser.close()
                except Exception:
                    pass
        self._notify_disconnect(reason)

    # -- 发送 -------------------------------------------------------------

    def write(self, data: bytes) -> int:
        payload = bytes(data)
        with self._lock:
            if not self._connected or self._ser is None:
                raise TransportError("USB CDC 未连接，无法写入")
            ser = self._ser
            try:
                written = 0
                while written < len(payload):
                    n = ser.write(payload[written:])
                    if not n:
                        break
                    written += n
                if written != len(payload):
                    raise TransportError(
                        f"串口写入不完整: {written}/{len(payload)} B"
                    )
                ser.flush()
            except (serial.SerialException, OSError) as exc:
                self._fail(f"write error: {exc}")
                raise TransportError(f"写串口失败: {exc}") from exc
        return written
