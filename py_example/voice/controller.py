"""语音控制器：段结束 → 解码 → ASR → 注入，全部在 worker 线程执行。

GUI 只负责把 ``AUDIO_CTL(ev=1)`` 的块字典交给 :meth:`VoiceController.submit_segment`，
然后通过回调更新状态；ASR 不阻塞 Qt 事件循环。
"""

from __future__ import annotations

import queue
import threading
import time
from typing import Any, Callable, Dict, Optional

from .asr.base import AsrEngine, AsrResult
from .inject import InjectResult, Injector
from .postprocess import clean_text
from .session import decode_blocks

StateCallback = Callable[[str, str], None]
ResultCallback = Callable[[AsrResult, Optional[InjectResult], Dict[str, Any]], None]
ErrorCallback = Callable[[str], None]
LogCallback = Callable[[str], None]


class VoiceController:
    def __init__(
        self,
        engine: AsrEngine,
        injector: Injector,
        *,
        inject_enabled: bool = False,
        inject_mode: str = "auto",
        on_state: Optional[StateCallback] = None,
        on_result: Optional[ResultCallback] = None,
        on_error: Optional[ErrorCallback] = None,
        on_log: Optional[LogCallback] = None,
    ) -> None:
        self.engine = engine
        self.injector = injector
        self.inject_enabled = bool(inject_enabled)
        self.inject_mode = inject_mode
        self.on_state = on_state
        self.on_result = on_result
        self.on_error = on_error
        self.on_log = on_log

        self._queue: "queue.Queue[Optional[tuple]]" = queue.Queue()
        self._thread: Optional[threading.Thread] = None
        self._running = False
        self._pending = 0
        self._cond = threading.Condition()
        self.last_result: Optional[AsrResult] = None
        self.last_inject: Optional[InjectResult] = None
        self.segments_done = 0
        self.failures = 0
        self.ready = False

    # -- 生命周期 ---------------------------------------------------------

    def start(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            return
        self._running = True
        self._thread = threading.Thread(target=self._run, name="voice-worker", daemon=True)
        self._thread.start()

    def close(self, timeout: float = 5.0) -> None:
        if not self._running:
            return
        self._running = False
        self._queue.put(None)
        thread = self._thread
        if thread is not None:
            thread.join(timeout=timeout)
        self._thread = None

    def _emit_state(self, state: str, detail: str = "") -> None:
        if self.on_state is not None:
            try:
                self.on_state(state, detail)
            except Exception:
                pass

    def _emit_error(self, message: str) -> None:
        self.failures += 1
        if self.on_error is not None:
            try:
                self.on_error(message)
            except Exception:
                pass

    def _emit_log(self, message: str) -> None:
        if self.on_log is not None:
            try:
                self.on_log(message)
            except Exception:
                pass

    # -- 对外接口 ---------------------------------------------------------

    def set_inject_enabled(self, on: bool) -> None:
        self.inject_enabled = bool(on)

    def set_inject_mode(self, mode: str) -> None:
        self.inject_mode = mode

    def warmup_async(self) -> None:
        self._enqueue(("warmup", None))

    def submit_segment(
        self,
        blocks: Dict[int, bytes],
        *,
        seg_id: int = -1,
        source: str = "ptt",
    ) -> None:
        if not blocks:
            return
        self._enqueue(("segment", (seg_id, dict(blocks), source)))

    def _enqueue(self, task: tuple) -> None:
        self.start()
        with self._cond:
            self._pending += 1
        self._queue.put(task)

    def wait_idle(self, timeout: Optional[float] = None) -> bool:
        deadline = None if timeout is None else time.monotonic() + timeout
        with self._cond:
            while self._pending > 0:
                remain = None if deadline is None else deadline - time.monotonic()
                if remain is not None and remain <= 0:
                    return False
                self._cond.wait(remain if remain is not None else 0.5)
        return True

    # -- worker -----------------------------------------------------------

    def _run(self) -> None:
        while True:
            task = self._queue.get()
            try:
                if task is None:
                    return
                kind, payload = task
                if kind == "warmup":
                    self._handle_warmup()
                elif kind == "segment":
                    seg_id, blocks, source = payload
                    self._handle_segment(seg_id, blocks, source)
            except Exception as exc:  # worker 不能死
                self._emit_state("error", str(exc))
                self._emit_error(f"语音处理异常：{exc}")
            finally:
                with self._cond:
                    self._pending = max(0, self._pending - 1)
                    self._cond.notify_all()
                self._queue.task_done()

    def _handle_warmup(self) -> None:
        self._emit_state("loading", "正在加载模型…")
        self._emit_log("加载 ASR 模型…")
        started = time.perf_counter()
        try:
            self.engine.warmup()
        except Exception as exc:
            self._emit_state("error", str(exc))
            self._emit_error(str(exc))
            return
        elapsed = int((time.perf_counter() - started) * 1000)
        label = getattr(self.engine, "model_label", "")
        self.ready = True
        self._emit_state("ready", label)
        self._emit_log(f"ASR 模型就绪{'：' + label if label else ''}（{elapsed} ms）")

    def _handle_segment(self, seg_id: int, blocks: Dict[int, bytes], source: str) -> None:
        self._emit_state("transcribing", f"seg={seg_id}")
        pcm, missing = decode_blocks(blocks)
        audio_ms = len(pcm) // 2 * 1000 // 16000
        if not pcm:
            self._emit_state("ready", "空音频")
            return
        if missing:
            self._emit_log(
                f"段 {seg_id}: 缺块 {len(missing)}（{len(missing) / max(1, max(blocks) + 1) * 100:.1f}%）"
            )

        try:
            result = self.engine.transcribe(pcm, 16000)
        except Exception as exc:
            self._emit_state("error", str(exc))
            self._emit_error(f"识别失败：{exc}")
            self._emit_log(f"段 {seg_id}: 识别失败，音频 {audio_ms / 1000:.2f}s 已保留在会话中")
            return

        result.text = clean_text(result.text)
        self.last_result = result
        self.segments_done += 1
        inject_result: Optional[InjectResult] = None
        if result.text and self.inject_enabled:
            self._emit_state("injecting", result.text[:32])
            inject_result = self.injector.inject(result.text, self.inject_mode)
            self.last_inject = inject_result
            if not inject_result.ok:
                self._emit_error(f"文本注入失败：{inject_result.error}")
            else:
                self._emit_log(
                    f"注入成功（{inject_result.method}, {inject_result.chars} 字）"
                )

        meta = {
            "seg_id": seg_id,
            "source": source,
            "missing": missing,
            "audio_ms": audio_ms,
            "model": getattr(self.engine, "model_label", ""),
        }
        # 先回到 ready，再投递结果；GUI 的“已注入/注入失败”不会被就绪状态覆盖
        self._emit_state("ready", "")
        if self.on_result is not None:
            try:
                self.on_result(result, inject_result, meta)
            except Exception:
                pass
        self._emit_log(
            f"识别 {audio_ms / 1000:.2f}s：{result.text!r} "
            f"（ASR {result.latency_ms} ms）"
        )
