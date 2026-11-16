"""测试用最小 ASR sidecar：不加载模型，只验证协议与 body 传递。

环境变量：
    FAKE_ASR_EMPTY=1      transcribe 恒返回空文本（触发 empty_text 分支）
    FAKE_ASR_TEXT=...     固定返回的文本
"""

import json
import os
import struct
import sys
import time

MAGIC = b"JN"


def read_frame(stream):
    prefix = stream.read(12)
    if not prefix:
        return None
    while len(prefix) < 12:
        prefix += stream.read(12 - len(prefix))
    header_len, body_len = struct.unpack("<II", prefix[4:12])
    header = json.loads(stream.read(header_len).decode("utf-8"))
    body = stream.read(body_len) if body_len else b""
    return header, body


def write_frame(header, body=b""):
    data = json.dumps(header, ensure_ascii=False).encode("utf-8")
    sys.stdout.buffer.write(
        MAGIC + bytes((1, 0)) + struct.pack("<II", len(data), len(body)) + data + body
    )
    sys.stdout.buffer.flush()


def now_ms():
    return int(time.time() * 1000)


def main():
    print("[fake-asr] started", file=sys.stderr, flush=True)
    write_frame(
        {
            "kind": "hello",
            "id": None,
            "ts_ms": now_ms(),
            "proto_version": 1,
            "role": "asr",
            "version": "0.0.1",
            "capabilities": ["load", "warmup", "transcribe", "cancel", "unload"],
        }
    )
    while True:
        frame = read_frame(sys.stdin.buffer)
        if frame is None:
            return 0
        header, body = frame
        method = header.get("method")
        request_id = header.get("id")

        if method == "shutdown":
            write_frame({"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True})
            return 0

        if method == "asr.load":
            write_frame(
                {
                    "kind": "response",
                    "id": request_id,
                    "ts_ms": now_ms(),
                    "ok": True,
                    "loaded": True,
                    "model_path": header.get("model_path") or "fake",
                    "load_ms": 1,
                }
            )
            continue

        if method == "asr.warmup":
            write_frame(
                {
                    "kind": "response",
                    "id": request_id,
                    "ts_ms": now_ms(),
                    "ok": True,
                    "ready": True,
                    "warmup_ms": 1,
                }
            )
            continue

        if method == "asr.transcribe":
            if header.get("sample_rate") != 16000:
                write_frame(
                    {
                        "kind": "response",
                        "id": request_id,
                        "ts_ms": now_ms(),
                        "ok": False,
                        "error": {"code": "E_INVALID_ARG", "message": "need 16000"},
                    }
                )
                continue
            text = os.environ.get("FAKE_ASR_TEXT")
            if text is None:
                text = "" if os.environ.get("FAKE_ASR_EMPTY") == "1" else f"len={len(body)}"
            audio_ms = len(body) // 32
            write_frame(
                {
                    "kind": "response",
                    "id": request_id,
                    "ts_ms": now_ms(),
                    "ok": True,
                    "text": text,
                    "language": "zh",
                    "audio_ms": audio_ms,
                    "latency_ms": 1,
                }
            )
            continue

        if method == "asr.unload":
            write_frame({"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True})
            continue

        if method == "ping":
            write_frame(
                {"kind": "response", "id": request_id, "ts_ms": now_ms(), "ok": True, "pong": True}
            )
            continue

        write_frame(
            {
                "kind": "response",
                "id": request_id,
                "ts_ms": now_ms(),
                "ok": False,
                "error": {"code": "E_NOT_IMPLEMENTED", "message": f"unknown {method}"},
            }
        )


if __name__ == "__main__":
    raise SystemExit(main())
