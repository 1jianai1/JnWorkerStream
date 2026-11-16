"""faster-whisper 本地引擎（CPU / int8）。

模型路径优先级：构造参数 → 环境变量 ``HOST_TEST_ASR_MODEL`` →
``models/faster-whisper-small`` → 名称 ``small``（由 faster-whisper 下载）。

``WhisperModel`` 加载较慢，所以真正加载放在 ``warmup()/transcribe()`` 第一次调用，
GUI 可以放到 worker 线程里，避免阻塞 Qt 事件循环。
"""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path
from typing import Any, Optional

from .base import AsrError, AsrResult, AsrUnavailable

ROOT = Path(__file__).resolve().parents[3]
DEFAULT_MODEL_DIR = ROOT / "models" / "faster-whisper-small"


def resolve_model_path(model: Optional[str] = None) -> str:
    """返回本地模型目录；不存在时退化为模型名（交给 faster-whisper 下载）。"""
    candidates = []
    if model:
        candidates.append(Path(model))
    env_model = os.environ.get("HOST_TEST_ASR_MODEL")
    if env_model:
        candidates.append(Path(env_model))
    candidates.append(DEFAULT_MODEL_DIR)
    for candidate in candidates:
        try:
            if candidate.exists():
                return str(candidate)
        except OSError:
            continue
    return model or "small"


class FasterWhisperEngine:
    """faster-whisper 的薄封装；线程内串行调用。"""

    def __init__(
        self,
        model_path: Optional[str] = None,
        device: str = "cpu",
        compute_type: str = "int8",
        language: str = "zh",
        beam_size: int = 1,
        vad_filter: bool = True,
        cpu_threads: int = 0,
        on_log: Optional[Any] = None,
    ) -> None:
        self.model_path_arg = model_path
        self.resolved_model = resolve_model_path(model_path)
        self.device = device
        self.compute_type = compute_type
        self.language = language
        self.beam_size = max(1, int(beam_size))
        self.vad_filter = bool(vad_filter)
        self.cpu_threads = max(0, int(cpu_threads))
        self.on_log = on_log
        self._model: Any = None
        self._lock = threading.Lock()

    @property
    def model_label(self) -> str:
        path = Path(self.resolved_model)
        if path.exists():
            return path.name
        return self.resolved_model

    def _log(self, text: str) -> None:
        if self.on_log is not None:
            try:
                self.on_log(text)
            except Exception:
                pass

    def _ensure_model(self) -> Any:
        if self._model is not None:
            return self._model
        with self._lock:
            if self._model is not None:
                return self._model
            try:
                from faster_whisper import WhisperModel  # type: ignore
            except ImportError as exc:
                raise AsrUnavailable(
                    "未安装 faster-whisper，请执行：\n"
                    "  pip install -r host_test/requirements-voice.txt"
                ) from exc

            self._log(f"加载 ASR 模型：{self.resolved_model}（{self.device}/{self.compute_type}）")
            try:
                self._model = WhisperModel(
                    self.resolved_model,
                    device=self.device,
                    compute_type=self.compute_type,
                    cpu_threads=self.cpu_threads,
                )
            except Exception as exc:
                raise AsrUnavailable(
                    f"ASR 模型加载失败：{exc}\n"
                    f"模型路径/名称：{self.resolved_model}\n"
                    "请把 faster-whisper-small 放到 models/faster-whisper-small，"
                    "或用 --model 指定本地目录。"
                ) from exc
            self._log(f"ASR 模型就绪：{self.model_label}")
            return self._model

    def warmup(self) -> None:
        """加载模型并用 0.5 s 静音做一次前推，压掉首句冷启动。"""
        self._ensure_model()
        silence = b"\x00\x00" * 8000  # 0.5 s @16k/mono/s16
        try:
            self.transcribe(silence, sample_rate=16000)
        except AsrError:
            raise
        except Exception as exc:  # 静音预热失败不致命，但保留日志
            self._log(f"ASR 预热警告：{exc}")

    def transcribe(self, pcm: bytes, sample_rate: int = 16000) -> AsrResult:
        if sample_rate != 16000:
            raise AsrError(f"仅支持 16 kHz PCM，收到 {sample_rate}")
        if not pcm:
            return AsrResult("", self.language, 0, 0)
        if len(pcm) % 2:
            pcm = pcm[:-1]

        try:
            import numpy as np  # type: ignore
        except ImportError as exc:
            raise AsrUnavailable(
                "未安装 numpy，请执行：pip install -r host_test/requirements-voice.txt"
            ) from exc

        model = self._ensure_model()
        started = time.perf_counter()
        pcm_i16 = np.frombuffer(pcm, dtype="<i2")
        pcm_f32 = pcm_i16.astype(np.float32) / 32768.0
        audio_ms = int(len(pcm_i16) * 1000 / sample_rate)

        with self._lock:
            try:
                text, info = self._run_model(pcm_f32, self.vad_filter)
                if not text and self.vad_filter:
                    # VAD 对很短的语音/低电平录音可能整段过滤掉；再给一次机会
                    self._log("VAD 未检测到语音，重试一次（vad_filter=False）")
                    text, info = self._run_model(pcm_f32, False)
            except Exception as exc:
                raise AsrError(f"ASR 识别失败：{exc}") from exc

        latency = int((time.perf_counter() - started) * 1000)
        lang = getattr(info, "language", None) or self.language or ""
        return AsrResult(text=text, lang=lang, audio_ms=audio_ms, latency_ms=latency)

    def _run_model(self, pcm_f32: Any, vad_filter: bool) -> Any:
        segments, info = self._model.transcribe(
            pcm_f32,
            language=self.language or None,
            beam_size=self.beam_size,
            vad_filter=vad_filter,
            condition_on_previous_text=False,
            without_timestamps=True,
        )
        text = "".join(seg.text for seg in segments).strip()
        return text, info
