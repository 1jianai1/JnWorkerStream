"""ASR sidecar（headless）：stdin 收请求，stdout 只输出协议帧，stderr 打日志。

method（`docs/interfaces.md` §4.4）::

    asr.load        {model_path?, device?, compute_type?, language?, beam_size?, vad_filter?, cpu_threads?}
    asr.warmup      {}
    asr.transcribe  body = PCM16 16 kHz 单声道；header {sample_rate:16000, id}
    asr.cancel      {target_id?}        取消未完成的 transcribe（按 id 丢弃迟到结果）
    asr.unload      {}
    ping            {}
    shutdown        {}

环境变量：

    JNWS_ASR_MODEL      模型目录（优先于 models/faster-whisper-small）
    JNWS_ASR_FAKE=1     用 EchoEngine 代替 faster-whisper（自检/集成测试，不需要模型）

启动方式（开发环境使用 kbhost Python）::

    python -m sidecars.asr
    python sidecars/asr/main.py
    python -m sidecars.asr --selftest
"""

from __future__ import annotations

import os
import queue
import sys
import threading
import time
from typing import Any, Dict, List, Optional, Tuple

try:
    from . import protocol
    from .engine import AsrError, EchoEngine, FasterWhisperEngine, SAMPLE_RATE
except ImportError:  # 直接以脚本运行
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    import protocol  # type: ignore[no-redef]
    from engine import (  # type: ignore[no-redef]
        AsrError,
        EchoEngine,
        FasterWhisperEngine,
        SAMPLE_RATE,
    )

CAPABILITIES = ["load", "warmup", "transcribe", "cancel", "unload"]
VERSION = "0.1.0"

Job = Tuple[str, bytes, int]


def log(message: str) -> None:
    print(f"[asr] {message}", file=sys.stderr, flush=True)


class AsrSidecar:
    """请求分发 + 单线程识别队列。

    `asr.transcribe` 会入队并由 worker 串行执行（避免阻塞 `asr.cancel` / `ping`）；
    其余方法在调用线程同步处理。
    """

    def __init__(self, engine: Any = None, logger=log) -> None:
        self.engine = engine if engine is not None else FasterWhisperEngine(logger)
        self._log = logger
        self._out_lock = threading.Lock()
        self._cancel_lock = threading.Lock()
        self._cancelled: set = set()
        self._pending: set = set()
        self._jobs: "queue.Queue[Optional[Job]]" = queue.Queue()
        self._worker: Optional[threading.Thread] = None
        self._shutdown = False

    # -- 输出 -------------------------------------------------------------

    def _write(self, header: Dict[str, Any], body: bytes = b"") -> None:
        with self._out_lock:
            protocol.write_frame(sys.stdout.buffer, header, body)

    # -- 请求处理 ---------------------------------------------------------

    def handle(self, header: Dict[str, Any], body: bytes) -> Optional[Dict[str, Any]]:
        """同步方法返回响应头；`asr.transcribe` 入队后返回 None（响应由 worker 写）。"""
        method = header.get("method")
        request_id = header.get("id")
        try:
            if method == "ping":
                return protocol.response(request_id, pong=True)

            if method == "shutdown":
                self._shutdown = True
                return protocol.response(request_id)

            if method == "asr.load":
                result = self.engine.load(
                    model_path=header.get("model_path"),
                    device=header.get("device") or "cpu",
                    compute_type=header.get("compute_type") or "int8",
                    language=header.get("language") or "zh",
                    beam_size=int(header.get("beam_size") or 1),
                    vad_filter=bool(header.get("vad_filter", True)),
                    cpu_threads=int(header.get("cpu_threads") or 0),
                )
                return protocol.response(request_id, **result)

            if method == "asr.warmup":
                return protocol.response(request_id, **self.engine.warmup())

            if method == "asr.unload":
                return protocol.response(request_id, **self.engine.unload())

            if method == "asr.transcribe":
                sample_rate = int(header.get("sample_rate") or SAMPLE_RATE)
                if sample_rate != SAMPLE_RATE:
                    return protocol.error(
                        request_id,
                        "E_INVALID_ARG",
                        f"仅支持 {SAMPLE_RATE} Hz，收到 {sample_rate}",
                    )
                self._enqueue(request_id, body, sample_rate)
                return None

            if method == "asr.cancel":
                target = header.get("target_id") or header.get("id")
                cancelled = self._cancel(str(target)) if target else False
                return protocol.response(request_id, cancelled=cancelled)

            return protocol.error(
                request_id, "E_NOT_IMPLEMENTED", f"未知 method: {method}"
            )
        except AsrError as exc:
            self._log(f"{method} failed: {exc.code} {exc.message}")
            return protocol.error(request_id, exc.code, exc.message)
        except Exception as exc:  # noqa: BLE001 - 兜底，避免 sidecar 静默退出
            self._log(f"{method} error: {exc!r}")
            return protocol.error(request_id, "E_ASR_FAILED", str(exc))

    # -- 识别队列 ---------------------------------------------------------

    def _ensure_worker(self) -> None:
        if self._worker is not None and self._worker.is_alive():
            return
        self._worker = threading.Thread(
            target=self._worker_loop, name="asr-worker", daemon=True
        )
        self._worker.start()

    def _enqueue(self, request_id: Any, body: bytes, sample_rate: int) -> None:
        self._ensure_worker()
        with self._cancel_lock:
            self._pending.add(str(request_id))
        self._jobs.put((str(request_id), bytes(body), sample_rate))

    def _cancel(self, target: str) -> bool:
        with self._cancel_lock:
            if target not in self._pending:
                return False
            self._cancelled.add(target)
            return True

    def _is_cancelled(self, request_id: str) -> bool:
        with self._cancel_lock:
            return request_id in self._cancelled

    def _settle(self, request_id: str) -> bool:
        """任务收尾：返回是否曾被取消（并从取消集合移除）。"""
        with self._cancel_lock:
            self._pending.discard(request_id)
            was_cancelled = request_id in self._cancelled
            self._cancelled.discard(request_id)
            return was_cancelled

    def _worker_loop(self) -> None:
        while True:
            job = self._jobs.get()
            if job is None:
                return
            request_id, pcm, sample_rate = job
            self._write(self.run_job(request_id, pcm, sample_rate))

    def run_job(self, request_id: str, pcm: bytes, sample_rate: int) -> Dict[str, Any]:
        """执行一次识别并返回响应头（单独暴露便于测试）。"""
        if self._is_cancelled(request_id):
            self._settle(request_id)
            return protocol.response(
                request_id,
                text="",
                language="",
                audio_ms=int(len(pcm) / 2 * 1000 / SAMPLE_RATE),
                latency_ms=0,
                cancelled=True,
            )
        try:
            result = self.engine.transcribe(pcm, sample_rate)
        except AsrError as exc:
            self._log(f"transcribe failed: {exc.code} {exc.message}")
            self._settle(request_id)
            return protocol.error(request_id, exc.code, exc.message)
        except Exception as exc:  # noqa: BLE001
            self._log(f"transcribe error: {exc!r}")
            self._settle(request_id)
            return protocol.error(request_id, "E_ASR_FAILED", str(exc))

        if self._is_cancelled(request_id):
            self._settle(request_id)
            return protocol.response(
                request_id,
                text="",
                language=result.get("language", ""),
                audio_ms=result.get("audio_ms", 0),
                latency_ms=result.get("latency_ms", 0),
                cancelled=True,
            )
        self._settle(request_id)
        return protocol.response(request_id, **result)

    # -- 生命周期 ---------------------------------------------------------

    def run(self) -> int:
        self._write(
            protocol.hello(role="asr", version=VERSION, capabilities=CAPABILITIES)
        )
        self._log("hello sent")
        try:
            while True:
                frame = protocol.read_frame(sys.stdin.buffer)
                if frame is None:
                    self._log("stdin closed")
                    break
                header, body = frame
                if header.get("kind") != "request":
                    self._log(f"ignore kind={header.get('kind')!r}")
                    continue
                response = self.handle(header, body)
                if response is not None:
                    self._write(response)
                if self._shutdown:
                    break
        finally:
            self._stop_worker()
        return 0

    def _stop_worker(self) -> None:
        if self._worker is None:
            return
        self._jobs.put(None)
        self._worker.join(timeout=1.0)
        self._worker = None


def build_engine(logger=log) -> Any:
    if os.environ.get("JNWS_ASR_FAKE") == "1":
        return EchoEngine(logger)
    return FasterWhisperEngine(logger)


def main() -> int:
    try:
        return AsrSidecar(build_engine()).run()
    except KeyboardInterrupt:
        return 130
    except Exception as exc:  # noqa: BLE001
        log(f"fatal: {exc!r}")
        return 1


def run_cli() -> None:
    """入口：退出时绕过解释器 finalize，避免守护线程与 stdin 锁竞争崩溃。"""
    code = main()
    try:
        sys.stdout.flush()
        sys.stderr.flush()
    finally:
        os._exit(code)


# -- 自检（不需要模型） ----------------------------------------------------


def selftest() -> int:
    import io

    failures: List[str] = []

    def check(name: str, ok: bool, detail: str = "") -> None:
        if ok:
            print(f"PASS {name}")
        else:
            failures.append(name)
            print(f"FAIL {name} {detail}")

    buffer = io.BytesIO()
    protocol.write_frame(buffer, protocol.response("r1", text="你好"), b"\x01\x02")
    buffer.seek(0)
    header, body = protocol.read_frame(buffer)
    check("frame roundtrip", header.get("text") == "你好" and body == b"\x01\x02")

    hello = protocol.hello(role="asr", version=VERSION, capabilities=CAPABILITIES)
    check("hello role/caps", hello["role"] == "asr" and "transcribe" in hello["capabilities"])

    sidecar = AsrSidecar(EchoEngine(lambda _text: None), lambda _text: None)
    loaded = sidecar.handle({"kind": "request", "id": "a", "method": "asr.load"}, b"")
    check("asr.load", loaded is not None and loaded["ok"])

    warm = sidecar.handle({"kind": "request", "id": "b", "method": "asr.warmup"}, b"")
    check("asr.warmup", warm is not None and warm["ok"] and warm["ready"])

    pcm = b"\x10\x00" * 1600  # 0.1 s 静音
    transcribe = sidecar.run_job("t1", pcm, SAMPLE_RATE)
    check(
        "asr.transcribe",
        transcribe["ok"] and transcribe["audio_ms"] == 100 and transcribe["text"].startswith("echo"),
    )

    bad = sidecar.handle(
        {"kind": "request", "id": "c", "method": "asr.transcribe", "sample_rate": 8000}, pcm
    )
    check(
        "sample_rate rejected",
        bad is not None and bad["error"]["code"] == "E_INVALID_ARG",
    )

    unknown = sidecar.handle({"kind": "request", "id": "d", "method": "nope"}, b"")
    check(
        "unknown method",
        unknown is not None and unknown["error"]["code"] == "E_NOT_IMPLEMENTED",
    )

    print("---")
    if failures:
        print(f"SELFTEST FAILED: {len(failures)} -> {', '.join(failures)}")
        return 1
    print("SELFTEST OK")
    return 0


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        raise SystemExit(selftest())
    run_cli()
