"""语音分段会话：把 ``AUDIO_CTL + AUDIO_DATA`` 还原成一段段录音。

核心不变量（见 docs/语音输入实施计划.md §1.4）：

    一次 KEY1 按压
      └─ AUDIO_CTL(ev=0, seg_id=N)  开新段（重置 ADPCM）
      └─ AUDIO_DATA(fmt=1, seq=0..) 段内块
      └─ AUDIO_CTL(ev=1, seg_id=N)  封段 → 交给 ASR

异常规则：
- 新段开始时上一段还没收到 ev=1 → 上一段丢弃，不和新区拼；
- 链路断开时未封段 → 丢弃；
- 没有 AUDIO_CTL 的旧 `audio save/send` 数据仍按 seq=0 自动切段，保持兼容。
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from ..audio_rx import (
    BLOCK_BYTES,
    BLOCK_SAMPLES,
    SAMPLE_RATE,
    AdpcmDecoder,
    parse_audio_payload,
)
from ..frame import Frame

MAX_SEGMENTS = 8


@dataclass
class AudioSegment:
    """一次按压对应的录音段。"""

    seg_id: int = -1
    blocks: Dict[int, bytes] = field(default_factory=dict)
    started_at: float = field(default_factory=time.monotonic)
    ended_at: Optional[float] = None
    complete: bool = False
    interrupted: bool = False

    @property
    def received(self) -> int:
        return len(self.blocks)

    @property
    def total(self) -> int:
        return (max(self.blocks) + 1) if self.blocks else 0

    @property
    def missing(self) -> List[int]:
        if not self.blocks:
            return []
        return [i for i in range(max(self.blocks) + 1) if i not in self.blocks]

    @property
    def duration_ms(self) -> int:
        return self.total * 20

    @property
    def missing_ratio(self) -> float:
        total = self.total
        return (len(self.missing) / total) if total else 0.0


def decode_blocks(blocks: Dict[int, bytes]) -> Tuple[bytes, List[int]]:
    """从 seq=0 顺序解码一段 ADPCM，缺块用静音占位并返回缺失列表。"""
    if not blocks:
        return b"", []
    end = max(blocks) + 1
    missing = [i for i in range(end) if i not in blocks]
    decoder = AdpcmDecoder()
    pcm = bytearray()
    for i in range(end):
        chunk = blocks.get(i)
        if chunk is None:
            pcm += b"\x00\x00" * BLOCK_SAMPLES
        else:
            pcm += decoder.decode_block(chunk)
    return bytes(pcm), missing


def decode_segment(segment: AudioSegment) -> Tuple[bytes, List[int]]:
    return decode_blocks(segment.blocks)


class VoiceSession:
    """按设备控制帧管理录音段，并提供实时 PCM 给扬声器。"""

    def __init__(self) -> None:
        self.segments: List[AudioSegment] = []
        self.current: Optional[AudioSegment] = None
        self.bad = 0
        self.talking = False
        self.last_rx = 0.0
        self.last_ctl = 0.0
        self.last_ctl_event: Optional[int] = None
        self.seg_id = -1
        self.interrupted = 0
        self.decoder = AdpcmDecoder()
        self.next_seq = 0
        self._ptt_mode = False

    # -- 生命周期 ---------------------------------------------------------

    def reset(self) -> None:
        self.segments = []
        self.current = None
        self.bad = 0
        self.talking = False
        self.last_rx = 0.0
        self.last_ctl = 0.0
        self.last_ctl_event = None
        self.seg_id = -1
        self.interrupted = 0
        self.decoder = AdpcmDecoder()
        self.next_seq = 0
        self._ptt_mode = False

    def _trim(self) -> None:
        while len(self.segments) > MAX_SEGMENTS:
            # current 永远是最新段，不会被这里删掉
            if self.segments[0] is self.current:
                break
            self.segments.pop(0)

    def _start_segment(self, seg_id: int) -> AudioSegment:
        segment = AudioSegment(seg_id=seg_id)
        self.segments.append(segment)
        self.current = segment
        self.decoder = AdpcmDecoder()
        self.next_seq = 0
        self.talking = True
        self.seg_id = seg_id
        self._trim()
        return segment

    def _remove_current(self) -> None:
        if self.current is not None and self.current in self.segments:
            self.segments.remove(self.current)
        self.current = None
        self.talking = False
        self.decoder = AdpcmDecoder()
        self.next_seq = 0

    def on_ctl(self, event: int, seg_id: int) -> Optional[AudioSegment]:
        """处理 AUDIO_CTL。

        Returns:
            ``event == 1`` 时返回刚封好的段；``event == 0`` 返回 None。
        """
        self.last_ctl = time.monotonic()
        self.last_ctl_event = event
        self._ptt_mode = True

        if event == 0:
            # 新的按压：上一段若未封段，按规则丢弃
            if self.current is not None:
                self.interrupted += 1
                self._remove_current()
            self._start_segment(seg_id)
            return None

        if event == 1:
            if self.current is None:
                return None
            self.talking = False
            self.current.ended_at = time.monotonic()
            self.current.complete = True
            finished = self.current
            self.current = None
            self.decoder = AdpcmDecoder()
            self.next_seq = 0
            return finished

        return None

    def discard_open(self, reason: str = "") -> Optional[AudioSegment]:
        """链路断开/停止收听时丢弃未封段，返回被丢弃的段（可能为 None）。"""
        segment = self.current
        if segment is not None:
            segment.interrupted = True
            self._remove_current()
            self.interrupted += 1
        return segment

    def finalize_current(self) -> Optional[AudioSegment]:
        """没有 ev=1 时（旧 audio save/send）把当前段封段。"""
        if self.current is None:
            return None
        self.talking = False
        self.current.ended_at = time.monotonic()
        self.current.complete = True
        finished = self.current
        self.current = None
        self.decoder = AdpcmDecoder()
        self.next_seq = 0
        return finished

    # -- 数据 -------------------------------------------------------------

    def feed(self, frame: Frame) -> bytes:
        """收一个 AUDIO_DATA 帧，返回实时播放 PCM（重复/坏帧返回空）。"""
        parsed = parse_audio_payload(frame.payload)
        if parsed is None:
            self.bad += 1
            return b""
        fmt, seq, data = parsed
        if fmt != 1 or len(data) != BLOCK_BYTES:
            self.bad += 1
            return b""

        if self.current is None:
            # PTT 模式下 ev=1 后的迟到块不应再开一段
            if self._ptt_mode:
                return b""
            # 旧 audio save/send 没有 CTL：首块自动开段
            self._start_segment(-1)

        segment = self.current
        assert segment is not None

        if seq < self.next_seq:
            if seq != 0:
                return b""
            if self._ptt_mode:
                return b""
            # 旧模式的第二遍录音：封上一段，开新段
            self.finalize_current()
            self._start_segment(-1)
            segment = self.current
            assert segment is not None

        if seq > self.next_seq:
            # 缺块：解码状态会漂移，但仍保留后续块；缺块由 stats()/decode 报告
            pass

        segment.blocks[seq] = data
        self.last_rx = time.monotonic()
        self.next_seq = seq + 1
        return self.decoder.decode_block(data)

    # -- 汇总 -------------------------------------------------------------

    def stats(self) -> Tuple[int, int, List[int]]:
        """返回 (已收块数, 应有块数, 缺失 seq 列表)：跨段汇总。"""
        received = 0
        total = 0
        missing: List[int] = []
        for segment in self.segments:
            if not segment.blocks:
                continue
            received += len(segment.blocks)
            end = max(segment.blocks) + 1
            total += end
            missing.extend(i for i in range(end) if i not in segment.blocks)
        return received, total, missing

    def pcm(self, include_current: bool = True) -> bytes:
        out = bytearray()
        for segment in self.segments:
            if not include_current and segment is self.current:
                continue
            if not segment.blocks:
                continue
            pcm, _missing = decode_segment(segment)
            out += pcm
        return bytes(out)

    def duration_s(self) -> float:
        _received, total, _missing = self.stats()
        return total * 0.02

    def last_segment(self, complete_only: bool = False) -> Optional[AudioSegment]:
        for segment in reversed(self.segments):
            if complete_only and not segment.complete:
                continue
            if segment.blocks:
                return segment
        return None


# 兼容旧 GUI 名称
AudioCapture = VoiceSession


def segment_filename(base: str, seg_id: int, multi: bool) -> str:
    """按段生成 WAV 文件名：out.wav -> out_seg3.wav。"""
    if not multi:
        return base
    path = Path(base)
    return str(path.with_name(f"{path.stem}_seg{seg_id}{path.suffix or '.wav'}"))
