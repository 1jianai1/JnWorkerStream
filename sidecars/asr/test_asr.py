"""`sidecars/asr` 单测：帧复用、分发、取消语义（不需要模型）。"""

from __future__ import annotations

import io
import os
import sys
import threading
import time
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

from sidecars.asr import main as asr_main  # noqa: E402
from sidecars.asr import protocol  # noqa: E402
from sidecars.asr.engine import AsrError, EchoEngine, SAMPLE_RATE  # noqa: E402


class FrameTests(unittest.TestCase):
    def test_roundtrip_with_chinese_and_body(self):
        buffer = io.BytesIO()
        protocol.write_frame(buffer, protocol.response("r1", text="你好"), b"\x01\x02\x03")
        buffer.seek(0)
        header, body = protocol.read_frame(buffer)
        self.assertEqual(header["text"], "你好")
        self.assertEqual(body, b"\x01\x02\x03")

    def test_hello_is_asr_role(self):
        hello = protocol.hello(
            role="asr", version=asr_main.VERSION, capabilities=asr_main.CAPABILITIES
        )
        self.assertEqual(hello["role"], "asr")
        self.assertEqual(hello["proto_version"], 1)
        self.assertIn("transcribe", hello["capabilities"])

    def test_shares_ble_protocol_implementation(self):
        # 帧格式只有一份实现：ASR 侧加载的必须就是 sidecars/ble/protocol.py。
        self.assertEqual(protocol.SOURCE.parent.name, "ble")
        self.assertEqual(protocol.SOURCE.name, "protocol.py")
        self.assertEqual(protocol.SOURCE_MODULE, "jnws_sidecar_protocol")
        self.assertEqual(protocol.MAGIC, b"JN")
        self.assertEqual(protocol.PROTO_VERSION, 1)
        header = {"kind": "response", "id": "x", "ts_ms": 1, "ok": True, "text": "a"}
        raw = protocol.encode_frame(header)
        self.assertTrue(raw.startswith(b"JN\x01\x00"))


class DispatchTests(unittest.TestCase):
    def setUp(self):
        self.sidecar = asr_main.AsrSidecar(EchoEngine(), lambda _text: None)

    def request(self, method, **fields):
        header = {"kind": "request", "id": fields.pop("id", "req"), "method": method}
        header.update(fields)
        return self.sidecar.handle(header, fields.pop("_body", b""))

    def test_load_warmup_unload(self):
        loaded = self.sidecar.handle({"kind": "request", "id": 1, "method": "asr.load"}, b"")
        self.assertTrue(loaded["ok"])
        self.assertTrue(loaded["loaded"])
        again = self.sidecar.handle({"kind": "request", "id": 2, "method": "asr.load"}, b"")
        self.assertTrue(again["already"])

        warm = self.sidecar.handle({"kind": "request", "id": 3, "method": "asr.warmup"}, b"")
        self.assertTrue(warm["ready"])

        unload = self.sidecar.handle({"kind": "request", "id": 4, "method": "asr.unload"}, b"")
        self.assertTrue(unload["ok"])

    def test_ping(self):
        pong = self.sidecar.handle({"kind": "request", "id": 5, "method": "ping"}, b"")
        self.assertTrue(pong["pong"])

    def test_transcribe_returns_none_and_jobs_run(self):
        written = []
        self.sidecar._write = lambda header, body=b"": written.append(header)  # type: ignore[method-assign]
        pcm = b"\x00\x00" * 1600  # 100 ms
        self.assertIsNone(
            self.sidecar.handle(
                {"kind": "request", "id": "t1", "method": "asr.transcribe"}, pcm
            )
        )
        deadline = time.time() + 2
        while "t1" in self.sidecar._pending and time.time() < deadline:
            time.sleep(0.01)
        self.assertNotIn("t1", self.sidecar._pending)
        self.assertEqual(len(written), 1)
        self.assertEqual(written[0]["audio_ms"], 100)
        self.assertEqual(written[0]["text"], "echo:100ms:3200B")

    def test_invalid_sample_rate(self):
        response = self.sidecar.handle(
            {"kind": "request", "id": "t2", "method": "asr.transcribe", "sample_rate": 48000},
            b"\x00\x00" * 160,
        )
        self.assertFalse(response["ok"])
        self.assertEqual(response["error"]["code"], "E_INVALID_ARG")

    def test_unknown_method(self):
        response = self.sidecar.handle({"kind": "request", "id": "t3", "method": "nope"}, b"")
        self.assertEqual(response["error"]["code"], "E_NOT_IMPLEMENTED")

    def test_engine_error_is_mapped(self):
        class Broken(EchoEngine):
            def load(self, **kwargs):
                raise AsrError("E_ASR_MODEL_MISSING", "no model")

        sidecar = asr_main.AsrSidecar(Broken(), lambda _text: None)
        response = sidecar.handle({"kind": "request", "id": "t4", "method": "asr.load"}, b"")
        self.assertFalse(response["ok"])
        self.assertEqual(response["error"]["code"], "E_ASR_MODEL_MISSING")


class CancelTests(unittest.TestCase):
    def _collecting_sidecar(self, engine):
        written = []
        sidecar = asr_main.AsrSidecar(engine, lambda _text: None)
        sidecar._write = lambda header, body=b"": written.append(header)  # type: ignore[method-assign]
        return sidecar, written

    def test_cancel_unknown_id_returns_false(self):
        sidecar, _ = self._collecting_sidecar(EchoEngine())
        response = sidecar.handle(
            {"kind": "request", "id": "c1", "method": "asr.cancel", "target_id": "nope"}, b""
        )
        self.assertFalse(response["cancelled"])

    def test_cancel_discards_late_result(self):
        gate = threading.Event()

        class Slow(EchoEngine):
            def transcribe(self, pcm, sample_rate=SAMPLE_RATE):
                gate.wait(2.0)
                return super().transcribe(pcm, sample_rate)

        sidecar, written = self._collecting_sidecar(Slow())
        pcm = b"\x00\x00" * 160
        sidecar._enqueue("t9", pcm, SAMPLE_RATE)
        time.sleep(0.05)  # 让 worker 进入 transcribe
        self.assertTrue(sidecar._cancel("t9"))
        gate.set()

        deadline = time.time() + 2
        while not written and time.time() < deadline:
            time.sleep(0.01)
        self.assertEqual(len(written), 1)
        self.assertTrue(written[0]["ok"])
        self.assertTrue(written[0]["cancelled"])
        self.assertEqual(written[0]["text"], "")
        self.assertNotIn("t9", sidecar._pending)

    def test_cancel_before_worker_picks_up(self):
        sidecar, written = self._collecting_sidecar(EchoEngine())
        pcm = b"\x00\x00" * 160
        sidecar._enqueue("t10", pcm, SAMPLE_RATE)
        sidecar._cancel("t10")
        deadline = time.time() + 2
        while not written and time.time() < deadline:
            time.sleep(0.01)
        self.assertTrue(written[0]["cancelled"])


if __name__ == "__main__":
    unittest.main()
