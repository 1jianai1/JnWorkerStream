"""Sidecar 帧协议（ADR-007）：长度前缀 + 二进制 body + JSON 头。

布局::

    0  2  3      4            8            12
    +--+--+------+------------+------------+----------------+----------+
    |JN|v1|flags |header_len  |body_len    | header (JSON)  | body     |
    +--+--+------+------------+------------+----------------+----------+

- 单帧上限 8 MiB（header + body）；
- stdout 只允许协议帧，日志一律 stderr；
- 必须能处理半帧到达：先读 12 B 头，再按长度读满。

本模块只依赖标准库，Rust 侧 `crates/kb-sidecar` 是最小复刻。
"""

from __future__ import annotations

import json
import struct
import time
from typing import Any, BinaryIO, Dict, Optional, Tuple

MAGIC = b"JN"
PROTO_VERSION = 1
HEADER_PREFIX = 12
MAX_FRAME = 8 * 1024 * 1024

#: 帧头 flags：bit0 = body 用 gzip（V1 恒 0）。
FLAG_BODY_GZIP = 0x01


class ProtocolError(RuntimeError):
    """帧非法 / 版本不匹配。"""


def now_ms() -> int:
    return int(time.time() * 1000)


def encode_frame(header: Dict[str, Any], body: bytes = b"") -> bytes:
    """把 JSON 头与二进制 body 编码为完整帧。"""
    if not isinstance(header, dict):
        raise ProtocolError("header 必须是 dict")
    for key in ("kind", "ts_ms"):
        if key not in header:
            raise ProtocolError(f"header 缺少 {key}")
    header_bytes = json.dumps(
        header, ensure_ascii=False, separators=(",", ":")
    ).encode("utf-8")
    body = bytes(body)
    if HEADER_PREFIX + len(header_bytes) + len(body) > MAX_FRAME:
        raise ProtocolError(f"帧超过 {MAX_FRAME} B 上限")
    return (
        MAGIC
        + bytes((PROTO_VERSION, 0))
        + struct.pack("<II", len(header_bytes), len(body))
        + header_bytes
        + body
    )


def write_frame(stream: BinaryIO, header: Dict[str, Any], body: bytes = b"") -> int:
    data = encode_frame(header, body)
    stream.write(data)
    stream.flush()
    return len(data)


def _read_exact(stream: BinaryIO, size: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < size:
        chunk = stream.read(size - len(chunks))
        if not chunk:
            raise ProtocolError(
                f"半帧结束：期望 {size} B，实际 {len(chunks)} B"
            )
        chunks += chunk
    return bytes(chunks)


def read_frame(
    stream: BinaryIO,
) -> Optional[Tuple[Dict[str, Any], bytes]]:
    """读取一帧；流结束（EOF）返回 None。"""
    prefix = stream.read(HEADER_PREFIX)
    if not prefix:
        return None
    if len(prefix) < HEADER_PREFIX:
        prefix += _read_exact(stream, HEADER_PREFIX - len(prefix))

    if prefix[0:2] != MAGIC:
        raise ProtocolError(f"magic 非法: {prefix[0:2]!r}")
    version = prefix[2]
    if version != PROTO_VERSION:
        raise ProtocolError(f"版本不匹配: {version} != {PROTO_VERSION}")
    flags = prefix[3]
    if flags & ~FLAG_BODY_GZIP:
        raise ProtocolError(f"未知 flags: 0x{flags:02X}")
    header_len, body_len = struct.unpack("<II", prefix[4:12])
    if HEADER_PREFIX + header_len + body_len > MAX_FRAME:
        raise ProtocolError("帧超过 8 MiB 上限")
    header_bytes = _read_exact(stream, header_len) if header_len else b""
    body = _read_exact(stream, body_len) if body_len else b""
    try:
        header = json.loads(header_bytes.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProtocolError(f"header JSON 非法: {exc}") from exc
    if not isinstance(header, dict):
        raise ProtocolError("header 必须是 JSON object")
    if "kind" not in header:
        raise ProtocolError("header 缺少 kind")
    return header, body


def hello(
    role: str = "ble",
    version: str = "0.1.0",
    capabilities: Optional[list] = None,
) -> Dict[str, Any]:
    import platform
    import sys

    return {
        "kind": "hello",
        "id": None,
        "ts_ms": now_ms(),
        "proto_version": PROTO_VERSION,
        "role": role,
        "version": version,
        "capabilities": capabilities
        or ["scan", "paired", "connect_by_address", "mtu", "notify"],
        "python": platform.python_version(),
        "platform": sys.platform,
    }


def response(request_id: Optional[str], **fields: Any) -> Dict[str, Any]:
    header = {"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True}
    header.update(fields)
    return header


def error(
    request_id: Optional[str], code: str, message: str, **fields: Any
) -> Dict[str, Any]:
    header = {
        "kind": "response",
        "id": request_id,
        "ts_ms": now_ms(),
        "ok": False,
        "error": {"code": code, "message": message},
    }
    header.update(fields)
    return header


def event(name: str, body: bytes = b"", **fields: Any) -> Tuple[Dict[str, Any], bytes]:
    header = {"kind": "event", "id": None, "ts_ms": now_ms(), "event": name}
    header.update(fields)
    return header, body
