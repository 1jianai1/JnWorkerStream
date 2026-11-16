"""Sidecar 帧协议的单元测试（标准库 unittest）。

运行::

    python -m unittest discover -s sidecars/ble -p "test_*.py"
"""

import io
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import protocol  # noqa: E402


class TestFrameCodec(unittest.TestCase):
    def test_roundtrip_with_body_and_unicode_header(self):
        header = {
            "kind": "response",
            "id": "req-1",
            "ts_ms": 1763200000000,
            "ok": True,
            "note": "已配对",
        }
        body = bytes(range(64))
        raw = protocol.encode_frame(header, body)
        got_header, got_body = protocol.read_frame(io.BytesIO(raw))
        self.assertEqual(got_header, header)
        self.assertEqual(got_body, body)

    def test_empty_body(self):
        raw = protocol.encode_frame({"kind": "event", "ts_ms": 1, "event": "ping"})
        header, body = protocol.read_frame(io.BytesIO(raw))
        self.assertEqual(header["event"], "ping")
        self.assertEqual(body, b"")

    def test_eof_returns_none(self):
        self.assertIsNone(protocol.read_frame(io.BytesIO(b"")))

    def test_half_prefix_raises(self):
        raw = protocol.encode_frame({"kind": "event", "ts_ms": 1})
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_frame(io.BytesIO(raw[:7]))

    def test_bad_magic(self):
        raw = bytearray(protocol.encode_frame({"kind": "event", "ts_ms": 1}))
        raw[0] = 0x00
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_frame(io.BytesIO(bytes(raw)))

    def test_version_mismatch(self):
        raw = bytearray(protocol.encode_frame({"kind": "event", "ts_ms": 1}))
        raw[2] = 2
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_frame(io.BytesIO(bytes(raw)))

    def test_oversized_frame_rejected(self):
        header = {"kind": "event", "ts_ms": 1}
        raw = bytearray(protocol.encode_frame(header))
        raw[4:8] = (protocol.MAX_FRAME).to_bytes(4, "little")
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_frame(io.BytesIO(bytes(raw)))

    def test_header_must_be_object(self):
        # 手工拼一个 header 为 JSON 数组的帧
        header_bytes = b"[]"
        raw = (
            protocol.MAGIC
            + bytes((protocol.PROTO_VERSION, 0))
            + len(header_bytes).to_bytes(4, "little")
            + (0).to_bytes(4, "little")
            + header_bytes
        )
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_frame(io.BytesIO(raw))

    def test_hello_contract(self):
        header = protocol.hello()
        self.assertEqual(header["kind"], "hello")
        self.assertEqual(header["proto_version"], 1)
        self.assertEqual(header["role"], "ble")
        self.assertIn("scan", header["capabilities"])
        json.dumps(header)  # 必须可序列化

    def test_error_shape(self):
        header = protocol.error("req-2", "E_BLE_CONNECT_TIMEOUT", "超时")
        self.assertFalse(header["ok"])
        self.assertEqual(header["error"]["code"], "E_BLE_CONNECT_TIMEOUT")
        self.assertEqual(header["id"], "req-2")


if __name__ == "__main__":
    unittest.main()
