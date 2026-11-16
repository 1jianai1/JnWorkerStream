"""语音输入子系统。"""

from __future__ import annotations

from .session import AudioSegment, VoiceSession, decode_blocks, decode_segment

__all__ = [
    "AudioSegment",
    "VoiceSession",
    "decode_blocks",
    "decode_segment",
]
