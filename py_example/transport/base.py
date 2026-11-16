"""传输层抽象（来源：docs/上位机通信验证.md §5.2）。

传输层只负责"搬字节"，不解析协议。USB、BLE、模拟链路都把收到的原始字节
交给 :class:`host_test.frame.FrameParser`。
"""

from __future__ import annotations

from typing import Callable, Optional


class TransportError(RuntimeError):
    """传输层打开/读写失败。"""


RxCallback = Callable[[bytes], None]
EventCallback = Callable[[str], None]
DisconnectCallback = Callable[[str], None]


class Transport:
    """所有链路实现的基类。"""

    name = "base"
    #: True 表示断开后由传输自身负责重连（BLE），False 表示由 CLI 重建传输对象。
    auto_reconnect = False

    def __init__(self) -> None:
        self._rx: Optional[RxCallback] = None
        self._event: Optional[EventCallback] = None
        self._disconnect: Optional[DisconnectCallback] = None
        self._connected = False

    # -- 回调 -------------------------------------------------------------

    def set_rx(self, cb: Optional[RxCallback]) -> None:
        """设置接收回调：收到任意字节块时调用 ``cb(bytes)``。"""
        self._rx = cb

    def set_event(self, cb: Optional[EventCallback]) -> None:
        """设置日志回调：传输层内部状态变化时调用 ``cb(text)``。"""
        self._event = cb

    def set_disconnect(self, cb: Optional[DisconnectCallback]) -> None:
        """设置断连回调：断开时调用 ``cb(reason)``。"""
        self._disconnect = cb

    def _deliver(self, data: bytes) -> None:
        if data and self._rx is not None:
            self._rx(data)

    def _emit(self, message: str) -> None:
        if self._event is not None:
            try:
                self._event(message)
            except Exception:  # 日志失败不影响数据通路
                pass

    def _notify_disconnect(self, reason: str) -> None:
        self._connected = False
        self._emit(f"LINK  disconnected: {reason}")
        cb = self._disconnect
        if cb is not None:
            try:
                cb(reason)
            except Exception:
                pass

    # -- 能力 -------------------------------------------------------------

    @property
    def connected(self) -> bool:
        return self._connected

    @property
    def mtu(self) -> int:
        """链路 MTU。USB CDC 固定 1024；BLE 为协商后的 ATT_MTU。"""
        return 1024

    @property
    def max_payload(self) -> int:
        """单帧 payload 上限。

        文档 §5.2：``payload <= mtu - 11``；BLE 侧还受 ATT 头 3 字节约束，
        见 :class:`host_test.transport.ble_tx.BleTransport`。
        """
        return max(0, self.mtu - 11)

    @property
    def description(self) -> str:
        return self.name

    # -- 生命周期 ---------------------------------------------------------

    def open(self) -> None:
        raise NotImplementedError

    def close(self) -> None:
        raise NotImplementedError

    def write(self, data: bytes) -> int:
        """写入原始字节，返回实际写入字节数；链路不可用时抛 TransportError。"""
        raise NotImplementedError

    def reopen(self) -> None:
        self.close()
        self.open()

    def __enter__(self) -> "Transport":
        self.open()
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()
