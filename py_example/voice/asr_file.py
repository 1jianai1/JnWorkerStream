"""对已有 WAV 做 ASR，便于不接设备时验证模型/准确率。

    python -m host_test.voice.asr_file record.wav --model models/faster-whisper-small
    python -m host_test.voice.asr_file record.wav --inject
"""

from __future__ import annotations

import argparse
import json
import sys
import wave
from pathlib import Path
from typing import List, Optional

from .asr.faster_whisper_engine import FasterWhisperEngine
from .inject import Injector


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="host_test.voice.asr_file", description="WAV → 文本")
    parser.add_argument("wav", help="WAV 路径（要求 16 kHz / 16 bit / 单声道）")
    parser.add_argument("--model", default=None, help="本地模型目录（默认 models/faster-whisper-small）")
    parser.add_argument("--lang", default="zh")
    parser.add_argument("--beam-size", type=int, default=1)
    parser.add_argument("--inject", nargs="?", const="auto",
                        choices=["auto", "sendinput", "clipboard", "off"], default="off")
    args = parser.parse_args(argv)

    path = Path(args.wav)
    if not path.exists():
        print(f"文件不存在: {path}", file=sys.stderr)
        return 1
    try:
        with wave.open(str(path), "rb") as w:
            channels = w.getnchannels()
            rate = w.getframerate()
            width = w.getsampwidth()
            pcm = w.readframes(w.getnframes())
    except (wave.Error, OSError) as exc:
        print(f"读取 WAV 失败: {exc}", file=sys.stderr)
        return 1
    if channels != 1 or rate != 16000 or width != 2:
        print(
            f"需要 16 kHz / 16 bit / mono，当前 channels={channels} rate={rate} width={width}",
            file=sys.stderr,
        )
        return 1

    engine = FasterWhisperEngine(
        model_path=args.model,
        language=args.lang,
        beam_size=args.beam_size,
        on_log=lambda text: print(f"[asr] {text}", flush=True),
    )
    try:
        engine.warmup()
        result = engine.transcribe(pcm, 16000)
    except Exception as exc:
        print(f"ASR 失败: {exc}", file=sys.stderr)
        return 1

    payload = {
        "text": result.text,
        "lang": result.lang,
        "audio_ms": result.audio_ms,
        "latency_ms": result.latency_ms,
    }
    print(json.dumps(payload, ensure_ascii=False, indent=2))

    if args.inject != "off" and result.text:
        injected = Injector().inject(result.text, args.inject)
        print(
            json.dumps(
                {
                    "inject_ok": injected.ok,
                    "method": injected.method,
                    "chars": injected.chars,
                    "error": injected.error,
                },
                ensure_ascii=False,
            )
        )
        if not injected.ok:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
