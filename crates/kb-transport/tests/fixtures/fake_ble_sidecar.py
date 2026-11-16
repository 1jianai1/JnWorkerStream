"""测试用最小 BLE sidecar：不接触真实蓝牙，验证 Rust 传输层的事件/写入/断连/崩溃。"""

import base64
import json
import os
import struct
import sys
import time

MAGIC = b"JN"


def read_frame(stream):
    prefix = stream.read(12)
    if not prefix:
        return None
    while len(prefix) < 12:
        prefix += stream.read(12 - len(prefix))
    header_len, body_len = struct.unpack("<II", prefix[4:12])
    header = json.loads(stream.read(header_len).decode("utf-8"))
    body = stream.read(body_len) if body_len else b""
    return header, body


def write_frame(header, body=b""):
    data = json.dumps(header, ensure_ascii=False).encode("utf-8")
    sys.stdout.buffer.write(
        MAGIC
        + bytes((1, 0))
        + struct.pack("<II", len(data), len(body))
        + data
        + body
    )
    sys.stdout.buffer.flush()


def now_ms():
    return int(time.time() * 1000)


def should_fail_connect():
    """`FAKE_BLE_FAIL` 恒失败；`FAKE_BLE_FAIL_TIMES` 只在首次连接之后失败 N 次。"""
    if os.environ.get("FAKE_BLE_FAIL") == "1":
        return True
    limit = int(os.environ.get("FAKE_BLE_FAIL_TIMES") or 0)
    path = os.environ.get("FAKE_BLE_FAIL_COUNTER")
    if limit <= 0 or not path or not os.path.exists(path):
        return False
    try:
        with open(path, encoding="utf-8") as handle:
            count = int(handle.read().strip() or 0)
    except (OSError, ValueError):
        count = 0
    if count >= limit:
        return False
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(str(count + 1))
    return True


def mark_connected():
    path = os.environ.get("FAKE_BLE_FAIL_COUNTER")
    if path and not os.path.exists(path):
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("0")


def main():
    print("[fake-ble] started", file=sys.stderr, flush=True)
    write_frame(
        {
            "kind": "hello",
            "id": None,
            "ts_ms": now_ms(),
            "proto_version": 1,
            "role": "ble",
            "version": "0.0.1",
            "capabilities": ["scan", "paired", "connect_by_address", "mtu", "notify"],
        }
    )
    while True:
        frame = read_frame(sys.stdin.buffer)
        if frame is None:
            return 0
        header, _body = frame
        method = header.get("method")
        request_id = header.get("id")

        if method == "shutdown":
            write_frame({"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True})
            return 0

        if method == "ble.connect":
            if should_fail_connect():
                write_frame(
                    {
                        "kind": "response",
                        "id": request_id,
                        "ts_ms": now_ms(),
                        "ok": False,
                        "error": {
                            "code": "E_BLE_DEVICE_NOT_FOUND",
                            "message": "fake: not found",
                        },
                    }
                )
                continue
            mark_connected()
            write_frame(
                {
                    "kind": "response",
                    "id": request_id,
                    "ts_ms": now_ms(),
                    "ok": True,
                    "state": "connected",
                    "address": header.get("address") or "5C:FD:52:80:0C:3C",
                    "name": header.get("name") or "SF32-KB-0C3C",
                    "mtu": 247,
                    "address_type": "public",
                }
            )
            continue

        if method == "ble.write":
            data = base64.b64decode(header.get("data_b64") or "")
            if os.environ.get("FAKE_BLE_CRASH_ON_WRITE") == "1":
                marker = os.environ.get("FAKE_BLE_MARKER")
                if not marker or not os.path.exists(marker):
                    if marker:
                        with open(marker, "w", encoding="utf-8") as handle:
                            handle.write("crashed")
                    os._exit(9)
            write_frame(
                {"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True},
            )
            write_frame(
                {
                    "kind": "event",
                    "id": None,
                    "ts_ms": now_ms(),
                    "event": "ble.bytes",
                    "len": len(data),
                },
                data,
            )
            if os.environ.get("FAKE_BLE_DROP_AFTER_WRITE") == "1":
                write_frame(
                    {
                        "kind": "event",
                        "id": None,
                        "ts_ms": now_ms(),
                        "event": "ble.link",
                        "state": "disconnected",
                        "reason": "link_lost",
                        "mtu": 247,
                    }
                )
            continue

        if method == "ble.disconnect":
            write_frame({"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True})
            continue

        if method == "ble.scan":
            write_frame(
                {
                    "kind": "response",
                    "id": request_id,
                    "ts_ms": now_ms(),
                    "ok": True,
                    "devices": [
                        {
                            "id": "fake",
                            "name": "SF32-KB-0C3C",
                            "address": "5C:FD:52:80:0C:3C",
                            "address_type": "public",
                            "paired": True,
                            "connected": False,
                            "rssi": None,
                        }
                    ],
                }
            )
            continue

        write_frame(
            {
                "kind": "response",
                "id": request_id,
                "ts_ms": now_ms(),
                "ok": True,
            }
        )


if __name__ == "__main__":
    raise SystemExit(main())
