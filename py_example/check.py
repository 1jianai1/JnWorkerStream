"""接收数据校验与录制/对比（来源：docs/上位机通信验证.md §5.3、§5.4、§6.3）。

包含三部分：

- :class:`Checker`：序号连续性、长度、内置测试模式、抖动、整体 sha256；
- :class:`Recorder` / :func:`load_records`：把每帧 ``(seq, sha256(data), 到达时间)``
  写入 CSV，供 USB / BLE 跨链路比较；
- :func:`compare_records`：比较两份记录（除到达时间、dev_ms 外必须一致）。
"""

from __future__ import annotations

import csv
import hashlib
import os
import statistics
import time
from collections import Counter, deque
from dataclasses import dataclass, field
from typing import Deque, Dict, Iterable, List, Optional, Sequence, Tuple

from .frame import TEST_FLAG_ECHO, Frame


# ---------------------------------------------------------------------------
# 测试源模式（与设备端 test_src.c 对齐）
# ---------------------------------------------------------------------------

PATTERNS = ("auto", "any", "inc", "zero", "ascii", "empty")


def classify_pattern(data: bytes, seq: int) -> str:
    """判断一段测试数据属于哪种内置模式。

    - ``inc``:  ``data[i] == (seq*7 + i) & 0xFF``（设备端推荐模式，可抓任何错位）
    - ``zero``: 全 0x00
    - ``ascii``: 可打印 ASCII（设备端 ``CUSTOM_DATA_TEST_%04u|`` 循环）
    """
    if not data:
        return "empty"
    if all(b == 0 for b in data):
        return "zero"
    if all(b == ((seq * 7 + i) & 0xFF) for i, b in enumerate(data)):
        return "inc"
    if all(0x20 <= b < 0x7F or b in (0x09, 0x0A, 0x0D) for b in data):
        return "ascii"
    return "other"


def _pattern_ok(expected: str, data: bytes, seq: int) -> bool:
    if expected == "any" or expected == "other":
        return True
    if expected == "inc":
        return bool(data) and all(
            b == ((seq * 7 + i) & 0xFF) for i, b in enumerate(data)
        )
    if expected == "zero":
        return all(b == 0 for b in data)
    if expected == "ascii":
        return all(0x20 <= b < 0x7F or b in (0x09, 0x0A, 0x0D) for b in data)
    if expected == "empty":
        return len(data) == 0
    return True


# ---------------------------------------------------------------------------
# 单帧信息
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class PayloadInfo:
    seq: int
    dev_ms: int
    flags: int
    data: bytes
    payload_len: int
    sha256: str
    is_echo: bool
    pattern: str
    pattern_ok: bool
    length_ok: bool


# ---------------------------------------------------------------------------
# 校验器
# ---------------------------------------------------------------------------


class Checker:
    """对 ``TEST_DATA`` 帧做序号/长度/模式/抖动/摘要校验。

    统计字段：

    - ``recv``     已接收的 TEST_DATA 帧数（含回显帧）
    - ``lost``     根据 seq 推算的丢失帧数
    - ``dup``      重复 seq 数
    - ``order``    乱序（seq 回退）数
    - ``pattern_err``  与预期测试模式不符的帧数
    - ``length_err``   与预期 data 长度不符的帧数
    - ``malformed``    payload 短于 5 字节的帧数

    ``lost`` 只根据到达顺序推算：如果设备重启导致 seq 归零，可能出现一次较大
    的 lost，这是符合预期的表现，详见 README 的说明。
    """

    def __init__(
        self,
        pattern: str = "auto",
        expected_data_len: Optional[int] = None,
        jitter_window: int = 4096,
        max_gap_records: int = 256,
        ignore_echo_seq: bool = True,
    ) -> None:
        if pattern not in PATTERNS:
            raise ValueError(f"未知 pattern: {pattern!r}，可选 {PATTERNS}")
        self.pattern = pattern
        self.expected_data_len = expected_data_len
        self.ignore_echo_seq = ignore_echo_seq

        self.expect_seq: Optional[int] = None  # 可选：期望的首帧 seq
        self.last_seq: Optional[int] = None
        self.first_seq: Optional[int] = None
        self.max_seq: Optional[int] = None

        self.recv = 0
        self.lost = 0
        self.dup = 0
        self.order = 0
        self.malformed = 0
        self.pattern_err = 0
        self.length_err = 0
        self.length_changes = 0
        self.echo_frames = 0

        self.payload_bytes = 0
        self.digest = hashlib.sha256()
        self.digest_final: Optional[str] = None

        self.lengths: Counter[int] = Counter()
        self.patterns_seen: Counter[str] = Counter()
        self.lost_ranges: List[Tuple[int, int]] = []
        self._max_gap_records = max_gap_records

        self._inferred_pattern: Optional[str] = None
        self._inferred_len: Optional[int] = None
        self._jitter_window = jitter_window
        self.intervals_ms: Deque[float] = deque(maxlen=jitter_window)
        self._last_arrival: Optional[float] = None
        self._started_at = time.monotonic()
        self._finished_at: Optional[float] = None

    # -- 序号 -------------------------------------------------------------

    def _record_gap(self, start: int, count: int) -> None:
        if count <= 0:
            return
        if len(self.lost_ranges) < self._max_gap_records:
            self.lost_ranges.append((start, count))

    def _track_seq(self, seq: int) -> None:
        if self.last_seq is None:
            self.first_seq = seq
            if self.expect_seq is not None and seq != self.expect_seq:
                gap = (seq - self.expect_seq) & 0xFFFF
                if 0 < gap < 0x8000:
                    self.lost += gap
                    self._record_gap(self.expect_seq, gap)
            self.last_seq = seq
            self.max_seq = seq
            return

        delta = (seq - self.last_seq) & 0xFFFF
        if delta == 0:
            self.dup += 1
            return
        if delta < 0x8000:
            if delta > 1:
                self.lost += delta - 1
                self._record_gap((self.last_seq + 1) & 0xFFFF, delta - 1)
            self.last_seq = seq
            if self.max_seq is None or ((seq - self.max_seq) & 0xFFFF) < 0x8000:
                self.max_seq = seq
        else:
            # 回退 / 严重乱序：只计数，不用它推进水位，避免后续被误判为大量丢帧。
            self.order += 1

    # -- 主入口 -----------------------------------------------------------

    def on_frame(self, frame: Frame, arrival: Optional[float] = None) -> PayloadInfo:
        return self.on_payload(frame.seq, frame.payload, arrival=arrival)

    def on_payload(
        self, seq: int, payload: bytes, arrival: Optional[float] = None
    ) -> PayloadInfo:
        now = time.monotonic() if arrival is None else arrival
        if self._last_arrival is not None:
            self.intervals_ms.append((now - self._last_arrival) * 1000.0)
        self._last_arrival = now
        self.recv += 1

        if len(payload) < 5:
            self.malformed += 1
            return PayloadInfo(
                seq=seq,
                dev_ms=0,
                flags=0,
                data=payload,
                payload_len=len(payload),
                sha256=hashlib.sha256(payload).hexdigest(),
                is_echo=False,
                pattern="other",
                pattern_ok=False,
                length_ok=False,
            )

        dev_ms = int.from_bytes(payload[0:4], "little")
        flags = payload[4]
        data = bytes(payload[5:])
        is_echo = bool(flags & TEST_FLAG_ECHO)
        data_len = len(data)

        if is_echo:
            self.echo_frames += 1
        if not (is_echo and self.ignore_echo_seq):
            self._track_seq(seq)

        self.payload_bytes += data_len
        self.digest.update(data)
        self.lengths[data_len] += 1

        if self.expected_data_len is not None:
            length_ok = data_len == self.expected_data_len
            if not length_ok:
                self.length_err += 1
        else:
            if self._inferred_len is None:
                self._inferred_len = data_len
            length_ok = data_len == self._inferred_len
            if not length_ok:
                self.length_changes += 1

        # 回显帧的数据应当逐字节等于主机发送内容，模式检查对它没有意义。
        check_pattern = not is_echo
        pattern = classify_pattern(data, seq)
        self.patterns_seen[pattern] += 1

        pattern_ok = True
        if check_pattern and self.pattern != "any":
            expected: Optional[str] = None
            if self.pattern == "auto":
                if self._inferred_pattern is None and pattern in ("inc", "zero", "ascii"):
                    self._inferred_pattern = pattern
                expected = self._inferred_pattern
            else:
                expected = self.pattern
            if expected is not None:
                pattern_ok = _pattern_ok(expected, data, seq)
                if not pattern_ok:
                    self.pattern_err += 1

        return PayloadInfo(
            seq=seq,
            dev_ms=dev_ms,
            flags=flags,
            data=data,
            payload_len=len(payload),
            sha256=hashlib.sha256(data).hexdigest(),
            is_echo=is_echo,
            pattern=pattern,
            pattern_ok=pattern_ok,
            length_ok=length_ok,
        )

    # -- 汇总 -------------------------------------------------------------

    def finish(self) -> None:
        if self._finished_at is None:
            self._finished_at = time.monotonic()
            self.digest_final = self.digest.hexdigest()

    @property
    def duration_s(self) -> float:
        end = self._finished_at if self._finished_at is not None else time.monotonic()
        return max(0.0, end - self._started_at)

    @property
    def rate_fps(self) -> float:
        d = self.duration_s
        return self.recv / d if d > 0 else 0.0

    @property
    def throughput_bps(self) -> float:
        d = self.duration_s
        return self.payload_bytes / d if d > 0 else 0.0

    @property
    def jitter_ms(self) -> Optional[float]:
        if len(self.intervals_ms) < 2:
            return None
        return statistics.pstdev(self.intervals_ms)

    @property
    def mean_interval_ms(self) -> Optional[float]:
        if not self.intervals_ms:
            return None
        return statistics.fmean(self.intervals_ms)

    @property
    def expected_pattern(self) -> Optional[str]:
        return self._inferred_pattern or (
            self.pattern if self.pattern not in ("auto", "any") else None
        )

    def pass_fail(self) -> Tuple[bool, List[str]]:
        """返回 (是否通过, 失败原因列表)。"""
        reasons: List[str] = []
        if self.recv == 0:
            reasons.append("no frames")
        if self.lost:
            reasons.append(f"lost={self.lost}")
        if self.dup:
            reasons.append(f"dup={self.dup}")
        if self.order:
            reasons.append(f"order={self.order}")
        if self.pattern_err:
            reasons.append(f"pattern_err={self.pattern_err}")
        if self.length_err:
            reasons.append(f"length_err={self.length_err}")
        if self.malformed:
            reasons.append(f"malformed={self.malformed}")
        return (not reasons, reasons)

    def summary_lines(self) -> List[str]:
        self.finish()
        assert self.digest_final is not None
        lines = [
            f"SUMMARY frames={self.recv} payload_bytes={self.payload_bytes} "
            f"lost={self.lost} dup={self.dup} order={self.order} "
            f"pattern_err={self.pattern_err} length_err={self.length_err} "
            f"malformed={self.malformed} echo={self.echo_frames}",
            f"SUMMARY seq first={self.first_seq} last={self.last_seq} "
            f"max={self.max_seq} duration={self.duration_s:.3f}s "
            f"rate={self.rate_fps:.2f}fps payload={self.throughput_bps / 1024.0:.2f}KB/s",
        ]

        if self.intervals_ms:
            jitter = self.jitter_ms
            jitter_txt = f"{jitter:.3f}ms" if jitter is not None else "n/a"
            lines.append(
                f"SUMMARY interval mean={self.mean_interval_ms:.3f}ms "
                f"jitter={jitter_txt} samples={len(self.intervals_ms)}"
            )

        if self.lengths:
            hist = " ".join(f"{k}B:{v}" for k, v in sorted(self.lengths.items())[:8])
            if len(self.lengths) > 8:
                hist += f" (+{len(self.lengths) - 8} more)"
            lines.append(f"SUMMARY sizes {hist}")

        if self.patterns_seen:
            seen = " ".join(f"{k}:{v}" for k, v in self.patterns_seen.items())
            lines.append(f"SUMMARY patterns {seen} expected={self.expected_pattern or 'none'}")

        if self.lost_ranges:
            ranges = ", ".join(
                f"{s}(+{n})" for s, n in self.lost_ranges[:12]
            )
            more = "" if len(self.lost_ranges) <= 12 else f" ... (+{len(self.lost_ranges) - 12})"
            lines.append(f"SUMMARY lost_at {ranges}{more}")

        lines.append(f"SUMMARY digest sha256={self.digest_final}")
        return lines


# ---------------------------------------------------------------------------
# CSV 录制
# ---------------------------------------------------------------------------

CSV_FIELDS = ["seq", "sha256", "arrival_ms", "payload_len", "data_len", "dev_ms", "flags"]


class Recorder:
    """把每帧记录为 CSV，行格式见 :data:`CSV_FIELDS`。"""

    def __init__(self, path: str, flush_every: int = 64) -> None:
        self.path = path
        self.flush_every = max(1, int(flush_every))
        parent = os.path.dirname(os.path.abspath(path))
        if parent and not os.path.isdir(parent):
            os.makedirs(parent, exist_ok=True)
        self._fh = open(path, "w", newline="", encoding="utf-8")
        self._writer = csv.DictWriter(self._fh, fieldnames=CSV_FIELDS)
        self._writer.writeheader()
        self._since_flush = 0
        self.rows = 0

    def write(self, info: PayloadInfo, arrival_ms: float) -> None:
        self._writer.writerow(
            {
                "seq": info.seq,
                "sha256": info.sha256,
                "arrival_ms": f"{arrival_ms:.3f}",
                "payload_len": info.payload_len,
                "data_len": len(info.data),
                "dev_ms": info.dev_ms,
                "flags": info.flags,
            }
        )
        self.rows += 1
        self._since_flush += 1
        if self._since_flush >= self.flush_every:
            self._fh.flush()
            self._since_flush = 0

    def close(self) -> None:
        if not self._fh.closed:
            self._fh.flush()
            self._fh.close()

    def __enter__(self) -> "Recorder":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()


# ---------------------------------------------------------------------------
# 跨链路比较
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class Record:
    seq: int
    sha256: str
    arrival_ms: float
    payload_len: int = 0
    data_len: int = 0
    dev_ms: int = 0
    flags: int = 0


def load_records(path: str) -> List[Record]:
    """读取 :class:`Recorder` 生成的 CSV；兼容只有前三列的精简 CSV。"""
    rows: List[Record] = []
    with open(path, "r", newline="", encoding="utf-8-sig") as fh:
        reader = csv.DictReader(fh)
        if not reader.fieldnames or "seq" not in reader.fieldnames:
            raise ValueError(f"{path}: 缺少 seq 列，不是本程序录制的 CSV")
        for lineno, row in enumerate(reader, start=2):
            try:
                seq = int(row["seq"])
                sha = (row.get("sha256") or "").strip().lower()
                arrival = float(row.get("arrival_ms") or 0.0)
                payload_len = int(row.get("payload_len") or 0)
                data_len = int(row.get("data_len") or 0)
                dev_ms = int(row.get("dev_ms") or 0)
                flags = int(row.get("flags") or 0)
            except (TypeError, ValueError) as exc:
                raise ValueError(f"{path}:{lineno}: 行格式错误: {exc}") from exc
            if not sha:
                raise ValueError(f"{path}:{lineno}: sha256 为空")
            rows.append(
                Record(
                    seq=seq,
                    sha256=sha,
                    arrival_ms=arrival,
                    payload_len=payload_len,
                    data_len=data_len,
                    dev_ms=dev_ms,
                    flags=flags,
                )
            )
    return rows


@dataclass(slots=True)
class CompareResult:
    ok: bool
    lines: List[str] = field(default_factory=list)
    failures: List[str] = field(default_factory=list)


def _first_diff(a: Sequence[object], b: Sequence[object]) -> Optional[int]:
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    if len(a) != len(b):
        return min(len(a), len(b))
    return None


def compare_records(
    path_a: str,
    path_b: str,
    label_a: str = "A",
    label_b: str = "B",
) -> CompareResult:
    """比较两份 CSV：帧数、seq 序列、data 哈希序列、payload 长度分布。"""
    a = load_records(path_a)
    b = load_records(path_b)
    lines = [
        f"{label_a}: {path_a} frames={len(a)}",
        f"{label_b}: {path_b} frames={len(b)}",
    ]
    failures: List[str] = []

    if not a or not b:
        failures.append("至少一份记录为空")
        lines.append("RESULT FAIL (empty record)")
        return CompareResult(False, lines, failures)

    if len(a) != len(b):
        failures.append(f"帧数不同: {len(a)} vs {len(b)}")
    lines.append(f"frames  {'OK' if len(a) == len(b) else 'DIFF'} ({len(a)} vs {len(b)})")

    seq_a = [r.seq for r in a]
    seq_b = [r.seq for r in b]
    diff = _first_diff(seq_a, seq_b)
    if diff is None:
        lines.append("seq     OK")
    else:
        failures.append(
            f"seq 序列在第 {diff} 帧不同: "
            f"{seq_a[diff] if diff < len(seq_a) else '<missing>'} vs "
            f"{seq_b[diff] if diff < len(seq_b) else '<missing>'}"
        )
        lines.append(f"seq     DIFF at index {diff}")
    if set(seq_a) != set(seq_b):
        only_a = sorted(set(seq_a) - set(seq_b))[:10]
        only_b = sorted(set(seq_b) - set(seq_a))[:10]
        failures.append(f"seq 集合不同: only {label_a}={only_a} only {label_b}={only_b}")
        lines.append("seqset  DIFF")

    hash_a = [r.sha256 for r in a]
    hash_b = [r.sha256 for r in b]
    diff = _first_diff(hash_a, hash_b)
    if diff is None:
        lines.append("sha256  OK")
    else:
        failures.append(f"sha256 序列在第 {diff} 帧不同")
        lines.append(f"sha256  DIFF at index {diff}")

    len_a = Counter(r.payload_len for r in a)
    len_b = Counter(r.payload_len for r in b)
    if len_a == len_b:
        desc = " ".join(f"{k}B:{v}" for k, v in sorted(len_a.items())[:8])
        lines.append(f"lengths OK ({desc})")
    else:
        lines.append(f"lengths DIFF {dict(sorted(len_a.items()))} vs {dict(sorted(len_b.items()))}")
        failures.append("payload 长度分布不同")

    ok = not failures
    lines.append("RESULT PASS" if ok else "RESULT FAIL")
    return CompareResult(ok, lines, failures)


def summarize_records(path: str) -> str:
    rows = load_records(path)
    if not rows:
        return f"{path}: empty"
    return (
        f"{path}: frames={len(rows)} seq={rows[0].seq}..{rows[-1].seq} "
        f"digest={hashlib.sha256(''.join(r.sha256 for r in rows).encode()).hexdigest()[:16]}"
    )
