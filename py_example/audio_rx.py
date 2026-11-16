"""接收 AUDIO_CTL/AUDIO_DATA → 按段解码 WAV → 可选 ASR + 文本注入。

设备端 PTT（KEY1）行为::

    ev=0, seg_id=N  开始一段（ADPCM 状态重置）
    AUDIO_DATA      每 20 ms 一块：fmt(1) + block_seq(2 LE) + 160 B ADPCM
    ev=1, seg_id=N  结束该段

本程序按 ``AUDIO_CTL`` 切段，每段写 ``<out>_seg<N>.wav``；也兼容旧的
``audio save/send``（无 CTL，按 block_seq=0 自动切段，最终写 ``--out``）。
加上 ``--asr`` 后，每段结束后解码并识别；``--inject`` 可注入当前焦点窗口。

用法::

    python -m host_test.audio_rx --address 5C:FD:52:80:0C:3C --out out.wav
    python -m host_test.audio_rx --name SF32 --out seg.wav --asr --inject
    python -m host_test.audio_rx --name SF32 --asr --model models/faster-whisper-small

解码器与设备端 ``src/audio/adpcm.c`` 使用同一套 step/index 表，块间状态连续，
所以必须从 block_seq 0 开始按序解码；缺块会在报告里列出。
"""

from __future__ import annotations

import argparse
import struct
import sys
import time
import wave
from pathlib import Path
from typing import Dict, List, Optional

from .frame import TYPE_AUDIO_CTL, TYPE_AUDIO_DATA, FrameParser

SAMPLE_RATE = 16000
CHANNELS = 1
BLOCK_SAMPLES = 320
BLOCK_BYTES = 160

AUDIO_FMT_PCM16 = 0
AUDIO_FMT_ADPCM = 1

STEP_TABLE = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31,
    34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
    724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499,
    2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630,
    9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623,
    27086, 29794, 32767,
]
INDEX_TABLE = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8]


class AdpcmDecoder:
    """与设备端 adpcm.c 对称的解码器，状态跨块连续。"""

    def __init__(self) -> None:
        self.predictor = 0
        self.index = 0

    def _nibble(self, code: int) -> int:
        step = STEP_TABLE[self.index]
        delta = step >> 3
        if code & 0x4:
            delta += step
        if code & 0x2:
            delta += step >> 1
        if code & 0x1:
            delta += step >> 2

        pred = self.predictor - delta if code & 0x8 else self.predictor + delta
        self.predictor = max(-32768, min(32767, pred))
        self.index = max(0, min(88, self.index + INDEX_TABLE[code & 0xF]))
        return self.predictor

    def decode_block(self, data: bytes) -> bytes:
        out = bytearray()
        for b in data:
            out += struct.pack("<h", self._nibble(b & 0x0F))
            out += struct.pack("<h", self._nibble(b >> 4))
        return bytes(out)


def parse_audio_payload(payload: bytes):
    """fmt(1) + block_seq(2 LE) + data[]"""
    if len(payload) < 3:
        return None
    fmt = payload[0]
    seq = int.from_bytes(payload[1:3], "little")
    return fmt, seq, payload[3:]


def write_wav(path: str, pcm: bytes) -> None:
    w = wave.open(path, "wb")
    try:
        w.setnchannels(CHANNELS)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(pcm)
    finally:
        w.close()


def build_arg_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="host_test.audio_rx",
        description="接收 AUDIO_DATA/AUDIO_CTL，按段保存 WAV，可选 ASR + 文本注入",
    )
    g = p.add_argument_group("链路")
    g.add_argument("--address", default="5C:FD:52:80:0C:3C", help="BLE 设备地址（默认本板）")
    g.add_argument("--name", default=None, help="按名字连接（未给地址时使用）")
    g.add_argument("--paired", action="store_true", help="只用系统配对记录直连")
    g.add_argument("--scan-timeout", type=float, default=6.0)
    g.add_argument("--connect-timeout", type=float, default=30.0)

    o = p.add_argument_group("输出")
    o.add_argument("--out", default="out.wav", help="WAV 输出基准路径（默认 out.wav）")
    o.add_argument("--seconds", type=float, default=30.0, help="最多等待秒数（默认 30）")
    o.add_argument("--idle", type=float, default=8.0, help="最后一帧后再等多久结束（默认 8 s）")
    o.add_argument("--quiet", action="store_true", help="不打印每块")

    a = p.add_argument_group("语音识别/注入（可选）")
    a.add_argument("--asr", nargs="?", const="faster-whisper", choices=["faster-whisper"],
                   default=None, help="启用 ASR；默认 faster-whisper")
    a.add_argument("--model", default=None,
                   help="本地模型目录（默认 models/faster-whisper-small，找不到则按 small 下载）")
    a.add_argument("--lang", default="zh", help="识别语言（默认 zh）")
    a.add_argument("--beam-size", type=int, default=1, help="beam size（默认 1，快）")
    a.add_argument("--device", default="cpu", choices=["cpu", "cuda"], help="推理设备")
    a.add_argument("--compute-type", default="int8", help="int8 / float16 / float32")
    a.add_argument("--inject", nargs="?", const="auto",
                   choices=["auto", "sendinput", "clipboard", "off"], default="off",
                   help="识别后注入当前焦点窗口；默认 off")
    return p


def main(argv: Optional[List[str]] = None) -> int:
    args = build_arg_parser().parse_args(argv)

    # 这些导入放在函数内，避免 voice.session <-> audio_rx 的循环导入
    from .voice.session import VoiceSession, decode_segment, segment_filename

    if args.inject != "off" and not args.asr:
        print("WARN --inject 需要同时启用 --asr，已忽略注入", file=sys.stderr)
        args.inject = "off"

    try:
        from .transport.ble_tx import BleTransport
    except Exception as exc:  # pragma: no cover - 环境缺依赖时给出可读提示
        print(f"BLE 传输不可用: {exc}", file=sys.stderr)
        return 1

    session = VoiceSession()
    saw_ctl = False
    activity_seen = False
    bad_fmt = 0
    bad_ctl = 0
    last_activity = 0.0
    segments_done: List[int] = []
    missing_total = 0
    controller = None
    asr_requested = bool(args.asr)
    asr_ready = False

    def write_segment(seg, multi: bool) -> None:
        nonlocal missing_total
        if seg is None or not seg.blocks:
            return
        pcm, missing = decode_segment(seg)
        path = segment_filename(args.out, seg.seg_id, multi)
        try:
            Path(path).parent.mkdir(parents=True, exist_ok=True)
            write_wav(path, pcm)
        except OSError as exc:
            print(f"SEG 写入失败: {exc}", file=sys.stderr, flush=True)
            return
        missing_total += len(missing)
        segments_done.append(seg.seg_id)
        print(
            f"SEG seg={seg.seg_id} blocks={seg.received}/{seg.total} "
            f"missing={len(missing)} duration={seg.duration_ms / 1000:.2f}s -> {path}",
            flush=True,
        )
        if controller is not None and asr_ready:
            controller.submit_segment(seg.blocks, seg_id=seg.seg_id)

    # -- ASR 控制器（先加载模型，再开始收音频） ---------------------------
    if asr_requested:
        try:
            from .voice.asr.faster_whisper_engine import FasterWhisperEngine
            from .voice.controller import VoiceController
            from .voice.inject import Injector
        except Exception as exc:
            print(f"语音模块不可用: {exc}", file=sys.stderr)
            return 1

        def on_state(state: str, detail: str) -> None:
            if state in ("loading", "transcribing", "injecting"):
                print(f"VOICE {state} {detail}".rstrip(), flush=True)
            elif state == "ready":
                print(f"VOICE ready {detail}".rstrip(), flush=True)
            elif state == "error":
                print(f"VOICE error {detail}", file=sys.stderr, flush=True)

        def on_result(result, inject_result, meta) -> None:
            print(f"TEXT {result.text}", flush=True)
            print(
                f"ASR lang={result.lang} audio={result.audio_ms / 1000:.2f}s "
                f"latency={result.latency_ms}ms model={meta.get('model', '')}",
                flush=True,
            )
            if inject_result is not None:
                status = "OK" if inject_result.ok else "FAIL"
                print(
                    f"INJECT {status} method={inject_result.method} "
                    f"chars={inject_result.chars} {inject_result.error}",
                    flush=True,
                )

        def on_error(message: str) -> None:
            print(f"VOICE ERROR: {message}", file=sys.stderr, flush=True)

        def on_log(message: str) -> None:
            print(f"[voice] {message}", flush=True)

        engine = FasterWhisperEngine(
            model_path=args.model,
            device=args.device,
            compute_type=args.compute_type,
            language=args.lang,
            beam_size=args.beam_size,
            on_log=on_log,
        )
        controller = VoiceController(
            engine,
            Injector(),
            inject_enabled=(args.inject != "off"),
            inject_mode=args.inject,
            on_state=on_state,
            on_result=on_result,
            on_error=on_error,
            on_log=on_log,
        )
        print("VOICE 正在加载模型 / 预热…", flush=True)
        controller.warmup_async()
        if not controller.wait_idle(180.0):
            print("VOICE 模型加载超时", file=sys.stderr, flush=True)
        asr_ready = controller.ready
        if not asr_ready:
            print("VOICE 模型不可用；仍会接收音频并保存 WAV", file=sys.stderr, flush=True)

    def on_frame(frame) -> None:
        nonlocal saw_ctl, activity_seen, bad_fmt, bad_ctl, last_activity
        if frame.type == TYPE_AUDIO_CTL:
            if len(frame.payload) < 3:
                bad_ctl += 1
                return
            event = frame.payload[0]
            seg_id = int.from_bytes(frame.payload[1:3], "little")
            saw_ctl = True
            activity_seen = True
            last_activity = time.monotonic()
            finished = session.on_ctl(event, seg_id)
            print(
                f"CTL {'START' if event == 0 else 'STOP' if event == 1 else '?'} seg={seg_id}",
                flush=True,
            )
            if finished is not None:
                write_segment(finished, multi=True)
            return

        if frame.type != TYPE_AUDIO_DATA:
            return
        parsed = parse_audio_payload(frame.payload)
        if parsed is None:
            bad_fmt += 1
            return
        fmt, seq, data = parsed
        if fmt != AUDIO_FMT_ADPCM:
            bad_fmt += 1
            return
        if len(data) != BLOCK_BYTES:
            bad_fmt += 1
            return
        before = len(session.segments)
        pcm = session.feed(frame)
        activity_seen = True
        last_activity = time.monotonic()
        if not args.quiet and len(session.segments) != before:
            print(f"SEG new (total {len(session.segments)})", flush=True)
        if not args.quiet and seq % 10 == 0:
            print(f"BLOCK seq={seq} {len(data)}B", flush=True)

    parser = FrameParser(on_frame)

    transport = BleTransport(
        name_filter=args.name,
        address=args.address,
        paired=args.paired,
        scan_timeout=args.scan_timeout,
        connect_timeout=args.connect_timeout,
    )
    transport.set_event(lambda text: print(text, flush=True))
    transport.set_rx(parser.feed)

    try:
        transport.open()
    except Exception as exc:
        print(f"连接失败: {exc}", file=sys.stderr)
        if controller is not None:
            controller.close()
        return 2

    print(f"LINK ready mtu={transport.mtu} max_payload={transport.max_payload}B", flush=True)
    print("等待 AUDIO_CTL/AUDIO_DATA …（按住 KEY1 说话，松开完成一段）", flush=True)

    start = time.monotonic()
    last_activity = start
    try:
        while time.monotonic() - start < args.seconds:
            time.sleep(0.05)
            if activity_seen and (time.monotonic() - last_activity) > args.idle:
                break
    except KeyboardInterrupt:
        pass
    finally:
        transport.close()

    # 没有 ev=1 的旧 audio save/send，或断链时未封段
    if session.current is not None:
        if saw_ctl:
            discarded = session.discard_open("link closed")
            if discarded is not None:
                print(
                    f"WARN 未封段 seg={discarded.seg_id} blocks={discarded.received} 已丢弃",
                    file=sys.stderr,
                    flush=True,
                )
        else:
            legacy = session.finalize_current()
            if legacy is not None:
                write_segment(legacy, multi=False)

    if controller is not None:
        controller.wait_idle(timeout=60.0)
        controller.close()
        asr_ready = asr_ready and controller.ready

    if not session.segments:
        print("RESULT NO-DATA：没有收到 AUDIO_DATA", flush=True)
        return 2

    received, total, missing = session.stats()
    if received == 0:
        print("RESULT NO-DATA：没有收到有效 AUDIO_DATA", flush=True)
        return 2
    print(
        f"RESULT segments={len(segments_done)} blocks={received}/{total} "
        f"missing={len(missing)} bad={bad_fmt} bad_ctl={bad_ctl}",
        flush=True,
    )
    if missing:
        print(f"丢失的 block_seq: {missing[:32]}{' …' if len(missing) > 32 else ''}", flush=True)

    if args.asr and not asr_ready:
        print("RESULT ASR 未就绪（WAV 已保留）", file=sys.stderr, flush=True)
        return 1
    if controller is not None and controller.failures:
        print(f"RESULT ASR/注入失败 {controller.failures} 次", file=sys.stderr, flush=True)
        return 1
    return 0 if not missing and bad_fmt == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
