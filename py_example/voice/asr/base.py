"""ASR 引擎接口。"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol


class AsrError(RuntimeError):
    """ASR 不可用/识别失败。"""


class AsrUnavailable(AsrError):
    """依赖或模型未准备好。"""


@dataclass(slots=True)
class AsrResult:
    text: str
    lang: str
    audio_ms: int
    latency_ms: int


class AsrEngine(Protocol):
    def warmup(self) -> None: ...

    def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> AsrResult: ...
