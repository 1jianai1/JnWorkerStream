"""测试用最小 sidecar：只依赖标准库，验证 Rust 侧握手/关联/事件/崩溃/版本。"""

import json
import os
import struct
import sys
import time

MAGIC = b"JN"
VERSION = 1


def read_frame(stream):
    prefix = stream.read(12)
    if not prefix:
        return None
    while len(prefix) < 12:
        chunk = stream.read(12 - len(prefix))
        if not chunk:
            return None
        prefix += chunk
    assert prefix[0:2] == MAGIC, prefix[0:2]
    version = prefix[2]
    assert version == 1, version
    header_len, body_len = struct.unpack("<II", prefix[4:12])
    header = json.loads(stream.read(header_len).decode("utf-8"))
    body = stream.read(body_len) if body_len else b""
    return header, body


def write_frame(header, body=b""):
    data = json.dumps(header, ensure_ascii=False).encode("utf-8")
    sys.stdout.buffer.write(
        MAGIC
        + bytes((VERSION, 0))
        + struct.pack("<II", len(data), len(body))
        + data
        + body
    )
    sys.stdout.buffer.flush()


def now_ms():
    return int(time.time() * 1000)


def main():
    print("[fake] started", file=sys.stderr, flush=True)
    write_frame(
        {
            "kind": "hello",
            "id": None,
            "ts_ms": now_ms(),
            "proto_version": int(os.environ.get("FAKE_PROTO", "1")),
            "role": os.environ.get("FAKE_ROLE", "ble"),
            "version": "0.0.1",
            "capabilities": ["echo"],
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
        if method == "crash":
            os._exit(7)
        if method == "sleep_ms":
            time.sleep(int(header.get("ms", 1000)) / 1000.0)
        if method == "emit":
            write_frame(
                {"kind": "event", "id": None, "ts_ms": now_ms(), "event": "tick", "n": 1},
                b"hello",
            )
        if method == "echo":
            write_frame(
                {"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True},
                str(header.get("text", "")).encode("utf-8"),
            )
            continue
        write_frame(
            {
                "kind": "response",
                "id": request_id,
                "ts_ms": now_ms(),
                "ok": True,
                "pong": True,
            }
        )


if __name__ == "__main__":
    raise SystemExit(main())
