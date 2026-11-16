"""内置自检：不接硬件即可验证协议与解析器（文档 §7 的 H5–H9、X1 基础部分）。

运行::

    python -m host_test --selftest
    python -m host_test.selftest
"""

from __future__ import annotations

import hashlib
import os
import sys
import time
import unittest
from contextlib import contextmanager
from typing import Iterator, List, Tuple

from .check import Checker, Recorder, compare_records, load_records
from .frame import (
    TEST_FLAG_ECHO,
    TYPE_AUDIO_CTL,
    TYPE_AUDIO_DATA,
    TYPE_TEST_DATA,
    Frame,
    FrameParser,
    build_frame,
    build_test_payload,
    crc16,
)
from .transport.ble_tx import _format_bt_address
from .transport.sim_tx import SimTransport, make_test_data
from .voice.asr.base import AsrResult
from .voice.controller import VoiceController
from .voice.inject import InjectResult
from .voice.session import VoiceSession, decode_segment


def _frame(seq: int, size: int = 64, pattern: str = "inc", dev_ms: int = 0) -> bytes:
    return build_frame(
        TYPE_TEST_DATA,
        0,
        seq,
        build_test_payload(dev_ms, make_test_data(seq, size, pattern), 0),
    )


@contextmanager
def _csv_pair(prefix: str) -> Iterator[Tuple[str, str]]:
    """在 host_test/ 下创建一对临时 CSV，结束后删除（避免系统临时目录权限问题）。"""
    base = os.path.dirname(os.path.abspath(__file__))
    paths = (
        os.path.join(base, f".selftest_{prefix}_a.csv"),
        os.path.join(base, f".selftest_{prefix}_b.csv"),
    )
    try:
        yield paths
    finally:
        for path in paths:
            try:
                os.unlink(path)
            except OSError:
                pass


class TestCrc(unittest.TestCase):
    def test_reference_vectors(self) -> None:
        self.assertEqual(crc16(b"123456789"), 0x29B1)
        self.assertEqual(crc16(b""), 0xFFFF)
        self.assertEqual(crc16(bytes(range(256))), crc16(bytes(range(256))))


class TestBleHelpers(unittest.TestCase):
    def test_format_address(self) -> None:
        self.assertEqual(_format_bt_address(0x5CFD52800C3C), "5C:FD:52:80:0C:3C")
        self.assertEqual(_format_bt_address(0x000000000001), "00:00:00:00:00:01")


class TestFrameBuild(unittest.TestCase):
    def test_roundtrip(self) -> None:
        payload = build_test_payload(123456, "你好, 键盘".encode("utf-8"), 0)
        raw = build_frame(TYPE_TEST_DATA, 0, 0x1234, payload)
        self.assertEqual(len(raw), 11 + len(payload))
        self.assertEqual(raw[:2], b"\xA5\x5A")
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(raw)
        self.assertEqual(len(got), 1)
        frame = got[0]
        self.assertEqual(frame.seq, 0x1234)
        self.assertEqual(frame.type, TYPE_TEST_DATA)
        self.assertEqual(frame.payload, payload)
        self.assertEqual(frame.data.decode("utf-8"), "你好, 键盘")
        self.assertEqual(frame.dev_ms, 123456)
        self.assertFalse(frame.is_echo)

    def test_reject_oversize_payload(self) -> None:
        with self.assertRaises(ValueError):
            build_frame(TYPE_TEST_DATA, 0, 0, bytes(1025))

    def test_echo_flag(self) -> None:
        raw = build_frame(
            TYPE_TEST_DATA, 0, 5, build_test_payload(1, b"abc", TEST_FLAG_ECHO)
        )
        got: List[Frame] = []
        FrameParser(got.append).feed(raw)
        self.assertTrue(got[0].is_echo)
        self.assertEqual(got[0].data, b"abc")


class TestParserStream(unittest.TestCase):
    def test_h5_split_into_three(self) -> None:
        raw = _frame(1)
        got: List[Frame] = []
        parser = FrameParser(got.append)
        cuts = [0, len(raw) // 3, (2 * len(raw)) // 3, len(raw)]
        for a, b in zip(cuts, cuts[1:]):
            parser.feed(raw[a:b])
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0].seq, 1)
        self.assertEqual(parser.stats["frames"], 1)
        self.assertEqual(parser.stats["crc_err"], 0)

    def test_h6_three_frames_one_feed(self) -> None:
        raw = b"".join(_frame(i) for i in range(3))
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(raw)
        self.assertEqual([f.seq for f in got], [0, 1, 2])
        self.assertEqual(parser.stats["frames"], 3)

    def test_h7_one_dirty_byte(self) -> None:
        raw = b"\x00" + _frame(7) + _frame(8)
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(raw)
        self.assertEqual([f.seq for f in got], [7, 8])
        self.assertEqual(parser.stats["resync"], 1)

    def test_h8_corrupt_payload_byte(self) -> None:
        bad = bytearray(_frame(3, pattern="ascii"))
        bad[10] ^= 0xFF  # payload 第 1 字节（避开 CRC 与帧头）
        raw = bytes(bad) + _frame(4)
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(raw)
        self.assertEqual([f.seq for f in got], [4])
        self.assertEqual(parser.stats["crc_err"], 1)
        self.assertEqual(parser.stats["oversize"], 0)

    def test_corrupt_crc(self) -> None:
        bad = bytearray(_frame(9, pattern="ascii"))
        bad[-1] ^= 0xFF
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(bytes(bad) + _frame(10))
        self.assertEqual([f.seq for f in got], [10])
        self.assertEqual(parser.stats["crc_err"], 1)

    def test_oversize_len(self) -> None:
        header = b"\xA5\x5A" + bytes((0x01, 0x60, 0x00)) + b"\x00\x00" + b"\x01\x04"
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(header + _frame(11))
        self.assertEqual([f.seq for f in got], [11])
        self.assertEqual(parser.stats["oversize"], 1)

    def test_unknown_type_skipped(self) -> None:
        unknown = build_frame(0x61, 0, 1, b"xyz")
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(unknown + _frame(2))
        self.assertEqual([f.seq for f in got], [2])
        self.assertEqual(parser.stats["unknown_type"], 1)

    def test_garbage_only(self) -> None:
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(b"\x00\x11\x22\x33" * 100)
        self.assertEqual(parser.stats["frames"], 0)
        self.assertEqual(len(got), 0)
        self.assertGreater(parser.stats["resync"], 0)

    def test_partial_header_kept(self) -> None:
        raw = _frame(1)
        got: List[Frame] = []
        parser = FrameParser(got.append)
        parser.feed(raw[:8])
        self.assertEqual(parser.stats["frames"], 0)
        parser.feed(raw[8:])
        self.assertEqual(len(got), 1)


class TestChecker(unittest.TestCase):
    def _run(self, frames: List[Frame], checker: Checker, t0: float = 0.0) -> None:
        for i, frame in enumerate(frames):
            checker.on_frame(frame, t0 + i * 0.01)

    def _frames(self, count: int = 1000, size: int = 64) -> List[Frame]:
        got: List[Frame] = []
        parser = FrameParser(got.append)
        # 随机分片投喂，顺便验证拆包/粘包
        raw = b"".join(_frame(i, size=size) for i in range(count))
        step = 137
        for i in range(0, len(raw), step):
            parser.feed(raw[i : i + step])
        return got

    def test_h9_1000_frames(self) -> None:
        frames = self._frames()
        self.assertEqual(len(frames), 1000)
        checker = Checker(pattern="auto")
        self._run(frames, checker)
        checker.finish()
        self.assertEqual(checker.lost, 0)
        self.assertEqual(checker.dup, 0)
        self.assertEqual(checker.order, 0)
        self.assertEqual(checker.pattern_err, 0)
        expected = hashlib.sha256(
            b"".join(make_test_data(i, 64, "inc") for i in range(1000))
        ).hexdigest()
        self.assertEqual(checker.digest_final, expected)
        ok, reasons = checker.pass_fail()
        self.assertTrue(ok, reasons)

    def test_detects_lost(self) -> None:
        frames = self._frames(count=20)
        del frames[5]
        checker = Checker(pattern="auto")
        self._run(frames, checker)
        self.assertEqual(checker.lost, 1)
        self.assertEqual(checker.lost_ranges, [(5, 1)])
        self.assertFalse(checker.pass_fail()[0])

    def test_detects_dup(self) -> None:
        frames = self._frames(count=20)
        frames.insert(5, frames[4])
        checker = Checker(pattern="auto")
        self._run(frames, checker)
        self.assertEqual(checker.dup, 1)

    def test_pattern_mismatch(self) -> None:
        frames = [
            Frame(1, TYPE_TEST_DATA, 0, i, build_test_payload(0, b"\x00" * 8))
            for i in range(3)
        ]
        checker = Checker(pattern="inc")
        self._run(frames, checker)
        self.assertEqual(checker.pattern_err, 3)

    def test_echo_ignored_for_seq(self) -> None:
        frames: List[Frame] = []
        parser = FrameParser(frames.append)
        parser.feed(_frame(1))
        parser.feed(
            build_frame(
                TYPE_TEST_DATA,
                0,
                0x7777,
                build_test_payload(0, b"hello", TEST_FLAG_ECHO),
            )
        )
        parser.feed(_frame(2))
        checker = Checker(pattern="auto")
        for i, f in enumerate(frames):
            checker.on_frame(f, i * 0.01)
        self.assertEqual(checker.echo_frames, 1)
        self.assertEqual(checker.lost, 0)
        self.assertEqual(checker.dup, 0)


class TestRecordCompare(unittest.TestCase):
    def _record(self, path: str, count: int = 50) -> None:
        frames: List[Frame] = []
        parser = FrameParser(frames.append)
        parser.feed(b"".join(_frame(i) for i in range(count)))
        checker = Checker()
        with Recorder(path) as rec:
            for i, frame in enumerate(frames):
                info = checker.on_frame(frame, i * 0.01)
                rec.write(info, i * 10.0)

    def test_roundtrip_and_compare(self) -> None:
        with _csv_pair("roundtrip") as (a, b):
            self._record(a)
            self._record(b)
            rows = load_records(a)
            self.assertEqual(len(rows), 50)
            self.assertEqual(rows[0].seq, 0)
            result = compare_records(a, b)
            self.assertTrue(result.ok, result.failures)
            self.assertTrue(any("RESULT PASS" in line for line in result.lines))

    def test_compare_detects_difference(self) -> None:
        with _csv_pair("diff") as (a, b):
            self._record(a, count=10)
            self._record(b, count=10)
            with open(b, "r", encoding="utf-8") as fh:
                lines = fh.readlines()
            # 直接改第 2 条记录的 hash
            parts = lines[2].strip().split(",")
            parts[1] = "0" * 64
            lines[2] = ",".join(parts) + "\n"
            with open(b, "w", encoding="utf-8") as fh:
                fh.writelines(lines)
            result = compare_records(a, b)
            self.assertFalse(result.ok)

    def test_compare_empty(self) -> None:
        with _csv_pair("empty") as (a, b):
            with Recorder(a):
                pass
            with Recorder(b):
                pass
            result = compare_records(a, b)
            self.assertFalse(result.ok)


class _FakeEngine:
    def warmup(self) -> None:
        pass

    def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> AsrResult:
        return AsrResult("测试文本", "zh", len(pcm) * 1000 // 2 // sample_rate, 1)


class _FakeInjector:
    def __init__(self) -> None:
        self.calls: List[Tuple[str, str]] = []

    def inject(self, text: str, mode: str = "auto") -> InjectResult:
        self.calls.append((text, mode))
        return InjectResult(True, "sendinput", len(text))


def _audio_frame(seq: int, data: bytes = b"") -> Frame:
    payload = bytes((1,)) + seq.to_bytes(2, "little") + (data or bytes(160))
    return Frame(1, TYPE_AUDIO_DATA, 0, seq, payload)


def _ctl_frame(event: int, seg_id: int) -> Frame:
    return Frame(1, TYPE_AUDIO_CTL, 0, 0, bytes((event,)) + seg_id.to_bytes(2, "little"))


class TestVoiceSession(unittest.TestCase):
    def test_ptt_segments_by_ctl(self) -> None:
        session = VoiceSession()
        session.on_ctl(0, 7)
        for seq in (0, 1, 2):
            session.feed(_audio_frame(seq))
        finished = session.on_ctl(1, 7)
        self.assertIsNotNone(finished)
        assert finished is not None
        self.assertEqual(finished.seg_id, 7)
        self.assertEqual(finished.received, 3)
        self.assertEqual(finished.missing, [])
        self.assertEqual(session.stats(), (3, 3, []))
        pcm, missing = decode_segment(finished)
        self.assertEqual(len(pcm), 3 * 320 * 2)
        self.assertEqual(missing, [])

    def test_new_start_discards_open_segment(self) -> None:
        session = VoiceSession()
        session.on_ctl(0, 1)
        session.feed(_audio_frame(0))
        session.on_ctl(0, 2)  # 上一段没有 ev=1，应丢弃
        self.assertEqual(session.interrupted, 1)
        session.feed(_audio_frame(0))
        finished = session.on_ctl(1, 2)
        self.assertIsNotNone(finished)
        self.assertEqual(session.stats()[0], 1)

    def test_discard_open_on_disconnect(self) -> None:
        session = VoiceSession()
        session.on_ctl(0, 3)
        session.feed(_audio_frame(0))
        discarded = session.discard_open("link lost")
        self.assertIsNotNone(discarded)
        self.assertEqual(session.current, None)
        self.assertEqual(session.stats()[0], 0)

    def test_legacy_without_ctl(self) -> None:
        session = VoiceSession()
        session.feed(_audio_frame(0))
        session.feed(_audio_frame(1))
        legacy = session.finalize_current()
        self.assertIsNotNone(legacy)
        self.assertEqual(session.stats(), (2, 2, []))


class TestVoiceController(unittest.TestCase):
    def test_warmup_submit_inject(self) -> None:
        states: List[str] = []
        results: List[Tuple[AsrResult, object, dict]] = []
        errors: List[str] = []
        injector = _FakeInjector()
        controller = VoiceController(
            _FakeEngine(),
            injector,
            inject_enabled=True,
            inject_mode="auto",
            on_state=lambda state, _detail: states.append(state),
            on_result=lambda result, inject_result, meta: results.append(
                (result, inject_result, meta)
            ),
            on_error=errors.append,
        )
        try:
            controller.warmup_async()
            self.assertTrue(controller.wait_idle(5.0))
            self.assertTrue(controller.ready)
            controller.submit_segment({0: bytes(160), 1: bytes(160)}, seg_id=9)
            self.assertTrue(controller.wait_idle(5.0))
            self.assertEqual(controller.failures, 0, errors)
            self.assertEqual(len(results), 1)
            result, inject_result, meta = results[0]
            self.assertEqual(result.text, "测试文本")
            self.assertTrue(inject_result.ok)
            self.assertEqual(meta["seg_id"], 9)
            self.assertEqual(injector.calls, [("测试文本", "auto")])
            self.assertIn("transcribing", states)
            self.assertIn("injecting", states)
        finally:
            controller.close()


class TestSimTransport(unittest.TestCase):
    def test_echo(self) -> None:
        sim = SimTransport(rate_hz=0, echo=True)
        frames: List[Frame] = []
        parser = FrameParser(frames.append)
        sim.set_rx(parser.feed)
        sim.open()
        try:
            raw = build_frame(
                TYPE_TEST_DATA, 0, 7, build_test_payload(0, b"hello echo", 0)
            )
            self.assertEqual(sim.write(raw), len(raw))
            deadline = time.monotonic() + 2.0
            while not frames and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertEqual(len(frames), 1)
            self.assertTrue(frames[0].is_echo)
            self.assertEqual(frames[0].seq, 7)
            self.assertEqual(frames[0].data, b"hello echo")
        finally:
            sim.close()

    def test_anomalies_still_parse(self) -> None:
        for anomaly in ("split", "sticky", "dirty", "badcrc", "all"):
            with self.subTest(anomaly=anomaly):
                sim = SimTransport(
                    rate_hz=400,
                    size=8,
                    pattern="inc",
                    anomaly=anomaly,
                    burst=45,
                    echo=False,
                )
                frames: List[Frame] = []
                parser = FrameParser(frames.append)
                sim.set_rx(parser.feed)
                sim.open()
                try:
                    deadline = time.monotonic() + 3.0
                    while time.monotonic() < deadline:
                        if sim._sent >= 45 and parser.pending == 0:
                            time.sleep(0.05)
                            break
                        time.sleep(0.01)
                finally:
                    sim.close()
                self.assertGreaterEqual(
                    len(frames), 25, f"{anomaly}: frames={len(frames)} stats={parser.stats}"
                )
                if anomaly in ("badcrc", "all"):
                    self.assertGreaterEqual(parser.stats["crc_err"], 1)
                if anomaly in ("dirty", "all"):
                    self.assertGreaterEqual(parser.stats["resync"], 1)


def run_selftest(verbosity: int = 2) -> int:
    """运行全部自检，返回进程退出码。"""
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    result = unittest.TextTestRunner(verbosity=verbosity).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(run_selftest())
