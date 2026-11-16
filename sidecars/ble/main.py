"""BLE sidecar（headless）：stdin 收请求，stdout 只输出协议帧，stderr 打日志。

支持的 method（`docs/interfaces.md` §4.3）::

    ble.scan        {timeout_ms?, name_filter?, service_uuid?}
    ble.list_paired {}
    ble.connect     {address?, name?, paired_only?, timeout_ms?}
    ble.disconnect  {}
    ble.write       {data_b64}
    ble.set_notify  {enabled}
    ping            {}
    shutdown        {}

事件：`ble.bytes`（body=原始字节）、`ble.link`（state/reason/mtu）。

启动方式（开发环境使用 kbhost Python）::

    python -m sidecars.ble
    # 或
    python sidecars/ble/main.py
"""

from __future__ import annotations

import asyncio
import base64
import sys
import threading
from typing import Any, Dict, Optional, Tuple

try:
    from . import protocol
    from .ble_scan import SERVICE_UUID, BleError, enumerate_paired_async, scan_async
    from .ble_win import BleLink
except ImportError:  # 直接以脚本运行
    import protocol  # type: ignore[no-redef]
    from ble_scan import (  # type: ignore[no-redef]
        SERVICE_UUID,
        BleError,
        enumerate_paired_async,
        scan_async,
    )
    from ble_win import BleLink  # type: ignore[no-redef]


def log(message: str) -> None:
    print(f"[ble] {message}", file=sys.stderr, flush=True)


class Sidecar:
    def __init__(self) -> None:
        self.link = BleLink(self._on_bytes, self._on_disconnect, log)
        self._shutdown = False

    # -- 事件出口（都在事件循环线程） ---------------------------------

    def _write(self, header: Dict[str, Any], body: bytes = b"") -> None:
        protocol.write_frame(sys.stdout.buffer, header, body)

    def _on_bytes(self, data: bytes) -> None:
        header, body = protocol.event("ble.bytes", data, len=len(data))
        self._write(header, body)

    def _on_disconnect(self, reason: str) -> None:
        header, body = protocol.event(
            "ble.link",
            state="disconnected",
            reason=reason,
            mtu=self.link.mtu,
        )
        self._write(header, body)

    # -- 请求分发 ---------------------------------------------------------

    async def dispatch(
        self, header: Dict[str, Any], body: bytes
    ) -> Tuple[Dict[str, Any], bytes]:
        method = header.get("method")
        request_id = header.get("id")
        if not isinstance(method, str):
            return protocol.error(request_id, "E_INVALID_ARG", "缺少 method"), b""

        try:
            if method == "ping":
                return protocol.response(request_id, pong=True), b""

            if method == "shutdown":
                self._shutdown = True
                return protocol.response(request_id), b""

            if method == "ble.scan":
                timeout_ms = int(header.get("timeout_ms") or 6000)
                devices = await scan_async(
                    timeout=max(0.5, timeout_ms / 1000.0),
                    name_filter=header.get("name_filter"),
                    service_uuid=header.get("service_uuid") or SERVICE_UUID,
                )
                return protocol.response(request_id, devices=devices), b""

            if method == "ble.list_paired":
                devices = await enumerate_paired_async()
                return protocol.response(request_id, devices=devices), b""

            if method == "ble.connect":
                timeout_ms = int(header.get("timeout_ms") or 30000)
                info = await self.link.connect(
                    address=header.get("address"),
                    name=header.get("name"),
                    paired_only=bool(header.get("paired_only")),
                    timeout=max(5.0, timeout_ms / 1000.0),
                )
                header_out = protocol.response(request_id, **info)
                header_out["state"] = "connected"
                return header_out, b""

            if method == "ble.disconnect":
                await self.link.disconnect()
                return protocol.response(request_id), b""

            if method == "ble.write":
                data_b64 = header.get("data_b64") or ""
                try:
                    data = base64.b64decode(data_b64, validate=True)
                except Exception as exc:  # noqa: BLE001
                    return (
                        protocol.error(
                            request_id, "E_INVALID_ARG", f"data_b64 非法: {exc}"
                        ),
                        b"",
                    )
                written = await self.link.write(data)
                return protocol.response(request_id, bytes=written), b""

            if method == "ble.set_notify":
                await self.link.set_notify(bool(header.get("enabled")))
                return protocol.response(request_id), b""

            return (
                protocol.error(
                    request_id, "E_NOT_IMPLEMENTED", f"未知 method: {method}"
                ),
                b"",
            )
        except BleError as exc:
            log(f"{method} failed: {exc.code} {exc.message}")
            return protocol.error(request_id, exc.code, exc.message), b""
        except asyncio.TimeoutError:
            log(f"{method} timeout")
            return (
                protocol.error(request_id, "E_BLE_CONNECT_TIMEOUT", "BLE 操作超时"),
                b"",
            )
        except Exception as exc:  # noqa: BLE001 - 兜底，避免 sidecar 静默退出
            log(f"{method} error: {exc!r}")
            code = "E_WRITE_FAILED" if method == "ble.write" else "E_IO"
            return protocol.error(request_id, code, str(exc)), b""

    # -- 生命周期 ---------------------------------------------------------

    async def run(self) -> int:
        loop = asyncio.get_running_loop()
        queue: asyncio.Queue = asyncio.Queue()
        stop = threading.Event()
        reader = threading.Thread(
            target=_stdin_reader,
            args=(loop, queue, stop),
            name="sidecar-stdin",
            daemon=True,
        )
        reader.start()

        self._write(protocol.hello())
        log("hello sent")
        try:
            while True:
                kind, payload = await queue.get()
                if kind == "eof":
                    log("stdin closed")
                    break
                if kind == "protocol_error":
                    log(f"protocol error: {payload}")
                    break
                header, body = payload
                if header.get("kind") != "request":
                    log(f"ignore kind={header.get('kind')!r}")
                    continue
                response_header, response_body = await self.dispatch(header, body)
                self._write(response_header, response_body)
                if self._shutdown:
                    break
        finally:
            stop.set()
            try:
                await self.link.disconnect()
            except Exception:
                pass
        return 0


def _stdin_reader(
    loop: asyncio.AbstractEventLoop,
    queue: "asyncio.Queue[Tuple[str, Any]]",
    stop: threading.Event,
) -> None:
    while not stop.is_set():
        try:
            frame = protocol.read_frame(sys.stdin.buffer)
        except Exception as exc:  # noqa: BLE001
            loop.call_soon_threadsafe(queue.put_nowait, ("protocol_error", repr(exc)))
            return
        if frame is None:
            loop.call_soon_threadsafe(queue.put_nowait, ("eof", None))
            return
        loop.call_soon_threadsafe(queue.put_nowait, ("frame", frame))


def main() -> int:
    try:
        return asyncio.run(Sidecar().run())
    except KeyboardInterrupt:
        return 130
    except Exception as exc:  # noqa: BLE001
        log(f"fatal: {exc!r}")
        return 1


def run_cli() -> None:
    """入口：退出时绕过解释器 finalize，避免守护读线程与 stdin 锁竞争崩溃。"""
    import os

    code = main()
    try:
        sys.stdout.flush()
        sys.stderr.flush()
    finally:
        os._exit(code)


if __name__ == "__main__":
    run_cli()
