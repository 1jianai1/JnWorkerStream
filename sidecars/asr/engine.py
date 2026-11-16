"""faster-whisper 引擎封装（CPU / int8，16 kHz 单声道 PCM16）。

与 `py_example/voice/asr/faster_whisper_engine.py` 的判别逻辑保持一致：
模型路径优先级 = 参数 → `JNWS_ASR_MODEL` → `<repo>/models/faster-whisper-small` → 模型名；
VAD 把整段过滤掉时用 `vad_filter=False` 再识别一次。

本模块只在 `load()/warmup()/transcribe()` 首次调用时 import faster-whisper 与 numpy，
这样没装依赖的机器仍然能跑 `python -m sidecars.asr --selftest`。
"""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path
from typing import Any, Callable, Dict, Optional

SAMPLE_RATE = 16000
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MODEL_DIR = ROOT / "models" / "faster-whisper-small"

#: 输入增益上限（约 +18 dB）。设备链路电平偏低时抬一档，但不把底噪也放大到失控。
MAX_GAIN = 8.0
#: 归一化目标峰值。
PEAK_TARGET = 0.9
#: 静音判定：20 ms 帧 RMS 的中位数低于该值即视为没有语音（-38 dBFS）。
SILENCE_DBFS = -38.0
SILENCE_PERCENTILE = 50
#: 幻觉过滤阈值（whisper 默认值，另加「同词重复」判定）。
MAX_COMPRESSION_RATIO = 2.4
MAX_NO_SPEECH = 0.6
MIN_AVG_LOGPROB = -1.0
REPEAT_UNIQUE_RATIO = 0.25


def is_hallucination(segment: Any) -> bool:
    """静音/底噪上 whisper 会编出「Hello Hello …」，按质量指标丢掉这类段。"""
    if getattr(segment, "compression_ratio", 0.0) > MAX_COMPRESSION_RATIO:
        return True
    if getattr(segment, "no_speech_prob", 0.0) > MAX_NO_SPEECH:
        return True
    if getattr(segment, "avg_logprob", 0.0) < MIN_AVG_LOGPROB:
        return True
    return is_repeated_tokens(segment.text)


def is_repeated_tokens(text: str) -> bool:
    """空格分词后几乎全是同一个词，判定为幻觉（中文不分词，不受影响）。"""
    tokens = text.split()
    if len(tokens) < 4:
        return False
    return len(set(tokens)) / len(tokens) <= REPEAT_UNIQUE_RATIO


def has_speech_energy(samples: Any, frame: int = 320) -> bool:
    """20 ms 帧能量的中位数是否过静音线（`samples` 为 -1..1 的 float32）。"""
    import numpy as np  # 延迟 import：selftest / EchoEngine 不需要 numpy

    total = (len(samples) // frame) * frame
    if total <= 0:
        return False
    frames = np.asarray(samples[:total], dtype=np.float32).reshape(-1, frame)
    rms = np.sqrt(np.mean(frames * frames, axis=1))
    level = 20 * np.log10(np.maximum(rms, 1e-9))
    return float(np.percentile(level, SILENCE_PERCENTILE)) >= SILENCE_DBFS


class AsrError(RuntimeError):
    """带 sidecar 错误码的识别异常（`E_ASR_*` / `E_INVALID_ARG`）。"""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


def resolve_model_path(model: Optional[str]) -> str:
    """返回可用的本地模型目录；都不存在时返回模型名（首次使用会触发下载）。"""
    candidates = []
    if model:
        candidates.append(Path(model))
    env_model = os.environ.get("JNWS_ASR_MODEL")
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


class EchoEngine:
    """不依赖模型的假引擎：给自检与 Rust 集成测试用。

    输出文本形如 `echo:<ms>ms:<bytes>B`，便于断言“这段 PCM 确实被识别了”。
    """

    def __init__(self, log: Optional[Callable[[str], None]] = None) -> None:
        self._log = log or (lambda _text: None)
        self._loaded = False
        self._load_ms = 0
        self._transcribes = 0

    def load(self, **kwargs: Any) -> Dict[str, Any]:
        if self._loaded:
            return {
                "loaded": True,
                "already": True,
                "model_path": "echo",
                "load_ms": self._load_ms,
            }
        self._loaded = True
        self._load_ms = 1
        return {"loaded": True, "model_path": "echo", "load_ms": self._load_ms}

    def warmup(self) -> Dict[str, Any]:
        return {"ready": True, "warmup_ms": 0}

    def unload(self) -> Dict[str, Any]:
        self._loaded = False
        return {}

    def transcribe(self, pcm: bytes, sample_rate: int = SAMPLE_RATE) -> Dict[str, Any]:
        if int(sample_rate) != SAMPLE_RATE:
            raise AsrError("E_INVALID_ARG", f"仅支持 {SAMPLE_RATE} Hz，收到 {sample_rate}")
        pcm = bytes(pcm)
        if len(pcm) % 2:
            pcm = pcm[:-1]
        audio_ms = int(len(pcm) / 2 * 1000 / SAMPLE_RATE)
        self._transcribes += 1
        return {
            "text": f"echo:{audio_ms}ms:{len(pcm)}B",
            "language": "zh",
            "audio_ms": audio_ms,
            "latency_ms": 0,
        }

    def status(self) -> Dict[str, Any]:
        return {"ready": self._loaded, "model_path": "echo", "transcribes": self._transcribes}


class FasterWhisperEngine:
    """线程内串行调用的引擎；`load` 幂等。"""

    def __init__(self, log: Optional[Callable[[str], None]] = None) -> None:
        self._log = log or (lambda _text: None)
        self._model: Any = None
        self._lock = threading.Lock()
        self._model_path: Optional[str] = None
        self._device = "cpu"
        self._compute_type = "int8"
        self._language = "zh"
        self._beam_size = 1
        self._vad_filter = True
        self._cpu_threads = 0
        self._load_ms = 0
        self._last_error: Optional[str] = None
        self._transcribes = 0

    # -- 状态 -------------------------------------------------------------

    @property
    def loaded(self) -> bool:
        return self._model is not None

    def status(self) -> Dict[str, Any]:
        return {
            "ready": self.loaded,
            "model_path": self._model_path or "",
            "device": self._device,
            "compute_type": self._compute_type,
            "language": self._language,
            "transcribes": self._transcribes,
            "last_error": self._last_error,
        }

    # -- 生命周期 ---------------------------------------------------------

    def load(
        self,
        model_path: Optional[str] = None,
        device: str = "cpu",
        compute_type: str = "int8",
        language: str = "zh",
        beam_size: int = 1,
        vad_filter: bool = True,
        cpu_threads: int = 0,
    ) -> Dict[str, Any]:
        resolved = resolve_model_path(model_path)
        if self.loaded and resolved == self._model_path and device == self._device:
            return {
                "loaded": True,
                "already": True,
                "model_path": resolved,
                "load_ms": self._load_ms,
            }

        try:
            from faster_whisper import WhisperModel  # type: ignore
        except ImportError as exc:  # pragma: no cover - 依赖缺失路径
            self._last_error = str(exc)
            raise AsrError(
                "E_ASR_FAILED",
                "未安装 faster-whisper：pip install -r sidecars/asr/requirements.txt",
            ) from exc

        self._log(f"load model {resolved} ({device}/{compute_type})")
        started = time.perf_counter()
        try:
            model = WhisperModel(
                resolved,
                device=device,
                compute_type=compute_type,
                cpu_threads=max(0, int(cpu_threads)),
            )
        except Exception as exc:
            self._last_error = str(exc)
            raise AsrError(
                "E_ASR_MODEL_MISSING",
                f"模型加载失败：{exc}（路径/名称：{resolved}）",
            ) from exc

        with self._lock:
            self._model = model
        self._model_path = resolved
        self._device = device
        self._compute_type = compute_type
        self._language = language or "zh"
        self._beam_size = max(1, int(beam_size))
        self._vad_filter = bool(vad_filter)
        self._cpu_threads = max(0, int(cpu_threads))
        self._load_ms = int((time.perf_counter() - started) * 1000)
        self._last_error = None
        return {"loaded": True, "model_path": resolved, "load_ms": self._load_ms}

    def warmup(self) -> Dict[str, Any]:
        """0.5 s 静音前推一次，压掉首句冷启动；失败不致命。

        注意不能走 `transcribe()`：静音会被静音门直接跳过，等于没预热。
        """
        started = time.perf_counter()
        if not self.loaded:
            self.load()
        try:
            import numpy as np  # type: ignore

            silence = np.zeros(SAMPLE_RATE // 2, dtype=np.float32)
            with self._lock:
                self._run(silence, False)
        except Exception as exc:  # noqa: BLE001 - 预热失败不影响后续识别
            self._log(f"warmup warning: {exc!r}")
        return {"ready": True, "warmup_ms": int((time.perf_counter() - started) * 1000)}

    def unload(self) -> Dict[str, Any]:
        with self._lock:
            self._model = None
        self._model_path = None
        return {}

    # -- 识别 -------------------------------------------------------------

    def transcribe(self, pcm: bytes, sample_rate: int = SAMPLE_RATE) -> Dict[str, Any]:
        if int(sample_rate) != SAMPLE_RATE:
            raise AsrError(
                "E_INVALID_ARG", f"仅支持 {SAMPLE_RATE} Hz，收到 {sample_rate}"
            )
        pcm = bytes(pcm)
        if len(pcm) % 2:
            pcm = pcm[:-1]
        audio_ms = int(len(pcm) / 2 * 1000 / SAMPLE_RATE)
        if not pcm:
            return {"text": "", "language": self._language, "audio_ms": 0, "latency_ms": 0}

        if not self.loaded:
            self.load()

        try:
            import numpy as np  # type: ignore
        except ImportError as exc:  # pragma: no cover - 依赖缺失路径
            raise AsrError(
                "E_ASR_FAILED",
                "未安装 numpy：pip install -r sidecars/asr/requirements.txt",
            ) from exc

        samples = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768.0
        started = time.perf_counter()
        if not has_speech_energy(samples):
            # 纯底噪：直接返回空串。否则 whisper 会在这里编出几百个 "Hello"，还慢。
            self._log(f"skip silent segment ({audio_ms} ms)")
            self._transcribes += 1
            return {
                "text": "",
                "language": self._language,
                "audio_ms": audio_ms,
                "latency_ms": int((time.perf_counter() - started) * 1000),
            }

        peak = float(np.max(np.abs(samples))) if samples.size else 0.0
        gain = min(MAX_GAIN, PEAK_TARGET / peak) if peak > 0 else 1.0
        samples = np.clip(samples * gain, -1.0, 1.0)

        with self._lock:
            try:
                text, info, dropped = self._run(samples, self._vad_filter)
                if not text and self._vad_filter:
                    # VAD 可能把轻声短句整段滤掉：能量检查已通过，再给一次机会。
                    self._log("VAD filtered everything, retry with vad_filter=False")
                    text, info, dropped = self._run(samples, False)
            except AsrError:
                raise
            except Exception as exc:
                self._last_error = str(exc)
                raise AsrError("E_ASR_FAILED", f"识别失败：{exc}") from exc

        if dropped:
            self._log(f"dropped {dropped} hallucinated segment(s)")

        latency_ms = int((time.perf_counter() - started) * 1000)
        self._transcribes += 1
        return {
            "text": text,
            "language": getattr(info, "language", None) or self._language,
            "audio_ms": audio_ms,
            "latency_ms": latency_ms,
        }

    def _run(self, samples: Any, vad_filter: bool) -> Any:
        """解码并过滤幻觉段；返回 `(text, info, dropped)`。"""
        segments, info = self._model.transcribe(
            samples,
            language=self._language or None,
            beam_size=self._beam_size,
            vad_filter=vad_filter,
            condition_on_previous_text=False,
            without_timestamps=True,
            temperature=0.0,  # 关闭温度回退：静音上回退会连跑 7 轮，延迟爆炸
        )
        kept = []
        dropped = 0
        for segment in segments:
            if is_hallucination(segment):
                dropped += 1
                continue
            kept.append(segment.text)
        return "".join(kept).strip(), info, dropped
