"""``python -m host_test`` 的命令行入口。

对应文档 §5.4 / §5.5：USB、BLE 共用同一个 FrameParser 和同一套输出格式。

典型用法::

    python -m host_test --transport usb --port COM5
    python -m host_test --transport ble --name SF32
    python -m host_test --transport usb --port COM5 --echo "hello"
    python -m host_test --transport usb --port COM5 --record run_usb.csv --duration 60
    python -m host_test --compare run_usb.csv run_ble.csv
    python -m host_test --transport sim --duration 3
    python -m host_test --selftest
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import threading
import time
from datetime import datetime
from typing import Callable, Dict, List, Optional, Sequence, Tuple

from . import __version__
from .check import (
    PATTERNS,
    Checker,
    CompareResult,
    PayloadInfo,
    Recorder,
    compare_records,
)
from .frame import (
    MAX_PAYLOAD,
    TYPE_TEST_DATA,
    Frame,
    FrameParser,
    build_frame,
    build_test_payload,
)
from .transport import (
    BleTransport,
    SerialTransport,
    SimTransport,
    Transport,
    TransportError,
    list_paired_ble_devices,
    list_serial_ports,
    scan_ble_devices,
)
from .transport.ble_tx import DATA_RX_UUID, DATA_TX_UUID, SERVICE_UUID
from .transport.sim_tx import ANOMALIES as SIM_ANOMALIES


# ---------------------------------------------------------------------------
# 输出
# ---------------------------------------------------------------------------


def _configure_stdout() -> None:
    """尽量让 Windows 控制台按 UTF-8 输出中文。"""
    try:
        if hasattr(sys.stdout, "reconfigure"):
            sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
    except Exception:
        pass


def _timestamp() -> str:
    return datetime.now().strftime("%H:%M:%S.%f")[:-3]


def _short_data(data: bytes, max_hex: int = 32, max_text: int = 96) -> str:
    """把 TEST_DATA 的 data 变成 DATA 行里的可读形式（UTF-8 或 HEX）。"""
    if not data:
        return '"" (0B)'
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        text = None

    if text is not None and all(ch.isprintable() or ch in "\r\n\t" for ch in text):
        shown = text if len(text) <= max_text else text[:max_text] + "…"
        return f"{json.dumps(shown, ensure_ascii=False)} ({len(data)}B)"

    shown_bytes = data[:max_hex]
    hex_text = " ".join(f"{b:02X}" for b in shown_bytes)
    if len(data) > len(shown_bytes):
        hex_text += " …"
    return f"hex={hex_text} ({len(data)}B)"


class Printer:
    """固定格式输出，所有链路保持一致。"""

    def __init__(self, tag: str, quiet: bool = False) -> None:
        self.tag = tag
        self.quiet = quiet
        self._lock = threading.Lock()

    def raw(self, line: str) -> None:
        with self._lock:
            print(line, flush=True)

    def line(self, message: str) -> None:
        self.raw(f"[{_timestamp()}] [{self.tag}] {message}")

    # 传输层日志回调
    def event(self, message: str) -> None:
        self.line(message)

    def frame(self, frame: Frame) -> None:
        if self.quiet:
            return
        self.line(
            f"FRAME seq={frame.seq} len={len(frame.payload)} "
            f"type=0x{frame.type:02X} flags=0x{frame.flags:02X} crc=OK"
        )

    def data(self, info: PayloadInfo) -> None:
        if self.quiet:
            return
        self.line(f"DATA  {_short_data(info.data)} sha={info.sha256[:8]}…")

    def echo(self, message: str) -> None:
        self.line(f"ECHO  {message}")

    def stat(self, checker: Checker, parser: FrameParser) -> None:
        ps = parser.stats_snapshot()
        self.line(
            f"STAT  rx={checker.recv} lost={checker.lost} dup={checker.dup} "
            f"crc_err={ps['crc_err']} oversize={ps['oversize']}"
        )

    def result(self, text: str) -> None:
        self.raw(f"[{_timestamp()}] [{self.tag}] RESULT {text}")


# ---------------------------------------------------------------------------
# 参数
# ---------------------------------------------------------------------------


def _hex_int(value: str) -> int:
    text = value.strip().lower()
    if text.startswith("0x"):
        text = text[2:]
    try:
        return int(text, 16)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"需要十六进制数，例如 0x1234 / 38F3: {value!r}") from exc


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python -m host_test",
        description="数字键盘上位机通信验证程序（协议基准：docs/上位机通信验证.md）",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "示例:\n"
            "  python -m host_test --gui\n"
            "  python -m host_test --transport usb --port COM5\n"
            "  python -m host_test --transport ble --name SF32\n"
            "  python -m host_test --transport ble --name SF32 --paired\n"
            "  python -m host_test --list-paired\n"
            "  python -m host_test --transport usb --port COM5 --echo \"hello\"\n"
            "  python -m host_test --transport usb --record run_usb.csv --duration 60\n"
            "  python -m host_test --compare run_usb.csv run_ble.csv\n"
            "  python -m host_test --transport sim --sim-anomaly all --duration 5\n"
            "  python -m host_test --selftest\n"
        ),
    )
    parser.add_argument("--version", action="version", version=f"host_test {__version__}")

    # 传输选择
    g = parser.add_argument_group("传输")
    g.add_argument("--transport", choices=("usb", "ble", "sim"), default="usb",
                   help="usb=USB CDC 串口（默认），ble=私有 GATT，sim=内置模拟设备")
    g.add_argument("--port", help="串口号，如 COM5；省略时按 --vid/--pid/--serial 匹配")
    g.add_argument("--baud", type=int, default=115200, help="波特率（CDC 忽略，默认 115200）")
    g.add_argument("--vid", type=_hex_int, help="按 USB VID 自动选口（十六进制）")
    g.add_argument("--pid", type=_hex_int, help="按 USB PID 自动选口（十六进制）")
    g.add_argument("--serial", dest="serial_number", help="按 USB 序列号自动选口")
    g.add_argument("--desc", dest="desc", help="按串口描述/设备名子串自动选口")
    g.add_argument("--reconnect", dest="reconnect", action="store_true", default=True,
                   help="断连后自动重连（默认开启）")
    g.add_argument("--no-reconnect", dest="reconnect", action="store_false",
                   help="断连后直接退出")
    g.add_argument("--connect-timeout", type=float, default=15.0,
                   help="首次连接总超时，秒（默认 15）")
    g.add_argument("--wait", action="store_true", help="一直等待设备出现，不因超时退出")

    # BLE
    gb = parser.add_argument_group("BLE")
    gb.add_argument("--service", default=SERVICE_UUID, help=f"私有服务 UUID（默认 {SERVICE_UUID}）")
    gb.add_argument("--tx-char", default=DATA_TX_UUID, help="DATA_TX 特征 UUID（设备->主机）")
    gb.add_argument("--rx-char", default=DATA_RX_UUID, help="DATA_RX 特征 UUID（主机->设备）")
    gb.add_argument("--name", default=None,
                    help="设备名子串，如 SF32；扫描不到时会自动查 Windows 配对记录")
    gb.add_argument("--address", default=None,
                    help="BLE 地址/MAC，指定后直接连接（不依赖广播），适合已被系统 HID 占用的设备")
    gb.add_argument("--paired", action="store_true",
                    help="跳过广播扫描，直接使用 Windows 已配对设备（需 --name 或 --address）")
    gb.add_argument("--scan-timeout", type=float, default=6.0, help="单次 BLE 扫描时间，秒（默认 6）")
    gb.add_argument("--scan-ble", action="store_true", help="扫描周围 BLE 设备（含已配对）后退出")
    gb.add_argument("--list-paired", action="store_true",
                    help="列出 Windows 已配对 BLE 设备后退出（含当前被 HID 占用、不广播的设备）")

    # 模拟链路
    gs = parser.add_argument_group("模拟链路")
    gs.add_argument("--sim-rate", type=float, default=20.0, help="模拟发送速率 Hz（默认 20）")
    gs.add_argument("--sim-size", type=int, default=64, help="模拟 data 长度（默认 64）")
    gs.add_argument("--sim-pattern", choices=("inc", "ascii", "zero"), default="inc",
                    help="模拟测试模式（默认 inc）")
    gs.add_argument("--sim-anomaly", choices=SIM_ANOMALIES, default="none",
                    help="模拟链路异常：拆包/粘包/脏字节/CRC 错")
    gs.add_argument("--sim-burst", type=int, default=0, help="只发 N 帧后停（0=持续发送）")
    gs.add_argument("--sim-no-echo", dest="sim_echo", action="store_false", default=True,
                    help="模拟设备不回显主机数据")

    # 回环
    ge = parser.add_argument_group("回环")
    ge.add_argument("--echo", metavar="TEXT", help="连接后发送一条 TEST_DATA 文本并校验回显")
    ge.add_argument("--echo-hex", metavar="HEX", help="同上，但发送十六进制字节，如 00 01 FF")
    ge.add_argument("--echo-count", type=int, default=1, help="发送条数（默认 1）")
    ge.add_argument("--echo-interval", type=float, default=0.2, help="多条间隔秒数（默认 0.2）")
    ge.add_argument("--echo-seq", type=int, default=0, help="起始 seq（默认 0）")
    ge.add_argument("--echo-timeout", type=float, default=5.0, help="等待全部回显的超时秒数（默认 5）")

    # 校验/录制
    gc = parser.add_argument_group("校验与录制")
    gc.add_argument("--record", metavar="CSV", help="把每帧 (seq, sha256(data), 到达时间) 写入 CSV")
    gc.add_argument("--pattern", choices=PATTERNS, default="auto",
                    help="期望的测试数据模式（默认 auto：首帧推断后校验）")
    gc.add_argument("--expect-data-len", type=int, default=None, help="期望每帧 data 长度")
    gc.add_argument("--allow-errors", action="store_true", help="有 lost/dup/crc_err 也返回 0")
    gc.add_argument("--duration", type=float, default=0.0, help="运行秒数，0=不限")
    gc.add_argument("--max-frames", type=int, default=0, help="收到 N 帧后退出，0=不限")
    gc.add_argument("--stat-every", type=float, default=1.0, help="STAT 周期秒数，0=关闭（默认 1）")
    gc.add_argument("--quiet", action="store_true", help="不打印 FRAME/DATA，只打印 STAT/结果")

    # 其他模式
    go = parser.add_argument_group("其他")
    go.add_argument("--gui", action="store_true", help="启动图形界面（PySide6）")
    go.add_argument("--compare", nargs=2, metavar=("A.csv", "B.csv"),
                    help="比较两份记录（帧数/seq/哈希/长度），不再连接设备")
    go.add_argument("--list", action="store_true", help="列出串口后退出")
    go.add_argument("--selftest", action="store_true", help="运行内置协议/解析器自检后退出")
    return parser


# ---------------------------------------------------------------------------
# 传输工厂 / 连接
# ---------------------------------------------------------------------------


def make_transport(args: argparse.Namespace) -> Transport:
    if args.transport == "usb":
        return SerialTransport(
            port=args.port,
            baudrate=args.baud,
            vid=args.vid,
            pid=args.pid,
            serial_number=args.serial_number,
            description=args.desc,
        )
    if args.transport == "ble":
        return BleTransport(
            service_uuid=args.service,
            tx_uuid=args.tx_char,
            rx_uuid=args.rx_char,
            name_filter=args.name,
            address=args.address,
            paired=args.paired,
            scan_timeout=args.scan_timeout,
            connect_timeout=max(5.0, args.connect_timeout),
        )
    return SimTransport(
        rate_hz=args.sim_rate,
        size=args.sim_size,
        pattern=args.sim_pattern,
        anomaly=args.sim_anomaly,
        burst=args.sim_burst,
        echo=args.sim_echo,
    )


def attach_transport(
    transport: Transport,
    parser: FrameParser,
    printer: Printer,
    disconnected: threading.Event,
) -> None:
    transport.set_rx(parser.feed)
    transport.set_event(printer.event)
    transport.set_disconnect(lambda _reason: disconnected.set())


def open_with_retry(
    factory: Callable[[], Transport],
    parser: FrameParser,
    printer: Printer,
    disconnected: threading.Event,
    args: argparse.Namespace,
) -> Transport:
    """首次连接：在 --connect-timeout（或 --wait）内反复尝试。"""
    deadline = None if args.wait else time.monotonic() + max(0.0, args.connect_timeout)
    delay = 1.0
    last_error: Optional[TransportError] = None
    while True:
        transport = factory()
        attach_transport(transport, parser, printer, disconnected)
        try:
            transport.open()
            return transport
        except TransportError as exc:
            last_error = exc
            transport.close()
            remaining = None if deadline is None else deadline - time.monotonic()
            if remaining is not None and remaining <= 0:
                raise last_error
            printer.event(f"LINK  not connected: {exc}")
            sleep_s = delay if remaining is None else max(0.0, min(delay, remaining))
            if sleep_s > 0:
                time.sleep(sleep_s)
            delay = min(delay * 1.6, 5.0)


def reconnect_transport(
    factory: Callable[[], Transport],
    parser: FrameParser,
    printer: Printer,
    disconnected: threading.Event,
    args: argparse.Namespace,
    deadline: Optional[float],
) -> Optional[Transport]:
    """串口/SIM 断连后重建传输对象；BLE 由传输内部重连，不走这里。"""
    delay = 1.0
    while True:
        if deadline is not None and time.monotonic() >= deadline:
            return None
        printer.event(f"LINK  reconnecting in {delay:.1f}s ...")
        time.sleep(delay)
        transport = factory()
        attach_transport(transport, parser, printer, disconnected)
        try:
            transport.open()
            parser.reset()  # 丢掉断连前残留的半帧
            printer.event("LINK  reconnected")
            return transport
        except TransportError as exc:
            transport.close()
            printer.event(f"LINK  reconnect failed: {exc}")
            delay = min(delay * 1.7, 10.0)


# ---------------------------------------------------------------------------
# 回环
# ---------------------------------------------------------------------------


def _build_echo_frames(
    args: argparse.Namespace, transport: Transport
) -> Tuple[List[bytes], Dict[int, bytes]]:
    if args.echo is not None and args.echo_hex is not None:
        raise TransportError("--echo 与 --echo-hex 只能二选一")
    if args.echo_hex is not None:
        try:
            data = bytes.fromhex(args.echo_hex.replace(",", " ").replace("0x", ""))
        except ValueError as exc:
            raise TransportError(f"--echo-hex 不是合法十六进制: {exc}") from exc
    elif args.echo is not None:
        data = args.echo.encode("utf-8")
    else:
        return [], {}

    count = max(1, args.echo_count)
    frames: List[bytes] = []
    pending: Dict[int, bytes] = {}
    host_ms = int(time.monotonic() * 1000.0) & 0xFFFFFFFF
    for i in range(count):
        seq = (args.echo_seq + i) & 0xFFFF
        payload = build_test_payload(host_ms, data, 0)
        if len(payload) > min(transport.max_payload, MAX_PAYLOAD):
            raise TransportError(
                f"回环数据 {len(data)} B 超过本链路上限 "
                f"{min(transport.max_payload, MAX_PAYLOAD)} B"
            )
        frame = build_frame(TYPE_TEST_DATA, 0, seq, payload)
        if transport.name.startswith("BLE") and len(frame) > transport.mtu - 3:
            raise TransportError(
                f"回环帧 {len(frame)} B 超过 ATT MTU-3={transport.mtu - 3} B"
            )
        frames.append(frame)
        pending[seq] = data
    return frames, pending


def send_echo_frames(
    transport: Transport,
    printer: Printer,
    args: argparse.Namespace,
    frames: List[bytes],
    pending: Dict[int, bytes],
) -> int:
    if not frames:
        return 0
    text_preview = (args.echo if args.echo is not None else args.echo_hex) or ""
    printer.event(
        f"ECHO  sending {len(frames)} frame(s) data={text_preview!r} "
        f"bytes={len(next(iter(pending.values())))}"
    )
    for i, frame in enumerate(frames):
        n = transport.write(frame)
        if n != len(frame):
            raise TransportError(f"回环帧写入不完整: {n}/{len(frame)} B")
        if i + 1 < len(frames):
            time.sleep(max(0.0, args.echo_interval))
    return len(frames)


# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------


def _drain_queue(
    arrivals: "queue.Queue[Tuple[float, Frame]]",
    limit: int = 1024,
) -> List[Tuple[float, Frame]]:
    items: List[Tuple[float, Frame]] = []
    for _ in range(limit):
        try:
            items.append(arrivals.get_nowait())
        except queue.Empty:
            break
    return items


def run_capture(args: argparse.Namespace, printer: Printer) -> int:
    run_start = time.monotonic()
    arrivals: "queue.Queue[Tuple[float, Frame]]" = queue.Queue()
    parser = FrameParser(on_frame=lambda f: arrivals.put((time.monotonic(), f)))
    disconnected = threading.Event()

    try:
        transport = open_with_retry(
            lambda: make_transport(args), parser, printer, disconnected, args
        )
    except TransportError as exc:
        printer.raw(f"[{_timestamp()}] [{printer.tag}] RESULT NO-LINK ({exc})")
        return 2

    # --duration 从连接成功、开始收数时计时（不含等待设备的时间）
    deadline: Optional[float] = None
    if args.duration and args.duration > 0:
        deadline = time.monotonic() + args.duration

    checker = Checker(
        pattern=args.pattern,
        expected_data_len=args.expect_data_len,
    )

    recorder: Optional[Recorder] = None
    if args.record:
        try:
            recorder = Recorder(args.record)
        except OSError as exc:
            printer.event(f"REC   cannot open {args.record}: {exc}")
            transport.close()
            return 1

    echo_pending: Dict[int, bytes] = {}
    echo_frames: List[bytes] = []
    echo_ok = 0
    echo_fail = 0
    echo_sent = 0
    echo_deadline: Optional[float] = None
    stop_reason = "eof"

    # 回环参数错误属于用法错误，不能伪装成"链路错误但数据正常"。
    if args.echo is not None or args.echo_hex is not None:
        try:
            echo_frames, echo_pending = _build_echo_frames(args, transport)
        except TransportError as exc:
            printer.event(f"ECHO  {exc}")
            transport.close()
            if recorder is not None:
                recorder.close()
            return 1

    try:
        printer.event(
            f"LINK  ready mtu={transport.mtu} max_payload={transport.max_payload}B "
            f"parser={args.pattern}"
        )

        if echo_frames:
            send_echo_frames(transport, printer, args, echo_frames, echo_pending)
            echo_sent = len(echo_frames)
            echo_deadline = time.monotonic() + max(0.5, args.echo_timeout)

        next_stat = time.monotonic() + args.stat_every if args.stat_every > 0 else float("inf")

        while True:
            now = time.monotonic()
            if deadline is not None and now >= deadline:
                stop_reason = "duration"
                break
            if args.max_frames and checker.recv >= args.max_frames:
                stop_reason = "max-frames"
                break

            if disconnected.is_set():
                disconnected.clear()
                if not args.reconnect:
                    stop_reason = "disconnected"
                    break
                if transport.auto_reconnect:
                    printer.event("LINK  waiting for auto-reconnect ...")
                else:
                    new_transport = reconnect_transport(
                        lambda: make_transport(args), parser, printer, disconnected, args, deadline
                    )
                    if new_transport is None:
                        stop_reason = "reconnect-timeout"
                        break
                    transport = new_transport

            try:
                arrival, frame = arrivals.get(timeout=0.02)
            except queue.Empty:
                pass
            else:
                batch = [(arrival, frame)]
                batch.extend(_drain_queue(arrivals))
                for ts, item in batch:
                    info = checker.on_frame(item, ts)
                    if recorder is not None:
                        recorder.write(info, (ts - run_start) * 1000.0)
                    printer.frame(item)
                    printer.data(info)

                    if info.is_echo:
                        expected = echo_pending.pop(info.seq, None)
                        if expected is None:
                            printer.echo(f"seq={info.seq} UNMATCHED")
                        elif info.data == expected:
                            echo_ok += 1
                            printer.echo(
                                f"seq={info.seq} OK ({len(info.data)}B) sha={info.sha256[:8]}…"
                            )
                        else:
                            echo_fail += 1
                            printer.echo(
                                f"seq={info.seq} DIFF ({len(info.data)}B != {len(expected)}B)"
                            )
                    if args.max_frames and checker.recv >= args.max_frames:
                        break

            if echo_sent and not echo_pending:
                stop_reason = "echo-done"
                break
            if echo_deadline is not None and time.monotonic() >= echo_deadline and echo_pending:
                stop_reason = "echo-timeout"
                break
            if (
                getattr(transport, "exhausted", False)
                and arrivals.empty()
                and not echo_pending
            ):
                stop_reason = "sim-done"
                break

            now = time.monotonic()
            if now >= next_stat:
                printer.stat(checker, parser)
                next_stat += args.stat_every
                if next_stat <= now:  # 落后太多就不补打印
                    next_stat = now + args.stat_every

    except KeyboardInterrupt:
        stop_reason = "interrupted"
        printer.raw("")
        printer.event("INTERRUPTED by user")
    except TransportError as exc:
        stop_reason = "link-error"
        printer.event(f"LINK  error: {exc}")
    finally:
        # 先关链路，再处理残留队列，最后打印汇总
        try:
            transport.close()
        except Exception:
            pass
        for ts, item in _drain_queue(arrivals):
            info = checker.on_frame(item, ts)
            if recorder is not None:
                recorder.write(info, (ts - run_start) * 1000.0)
            printer.frame(item)
            printer.data(info)
        if recorder is not None:
            recorder.close()

        checker.finish()
        printer.raw("")
        for line in checker.summary_lines():
            printer.line(line)
        ps = parser.stats_snapshot()
        printer.line(
            f"LINK  parser frames={ps['frames']} bytes={ps['bytes']} "
            f"crc_err={ps['crc_err']} oversize={ps['oversize']} "
            f"unknown_type={ps['unknown_type']} resync={ps['resync']} "
            f"overflow={ps['overflow']}"
        )
        if echo_sent or echo_pending or echo_fail:
            printer.line(
                f"ECHO  sent={echo_sent} ok={echo_ok} diff={echo_fail} "
                f"unmatched={len(echo_pending)}"
            )
        if recorder is not None:
            printer.line(f"REC   {recorder.rows} rows -> {os.path.abspath(args.record)}")

    # ---- 判定 ----
    intentional_errors = args.transport == "sim" and args.sim_anomaly != "none"
    ok, reasons = checker.pass_fail()
    hard_errors: List[str] = []
    if ps["crc_err"] and not intentional_errors:
        hard_errors.append(f"crc_err={ps['crc_err']}")
    if ps["oversize"] and not intentional_errors:
        hard_errors.append(f"oversize={ps['oversize']}")
    if ps["overflow"]:
        hard_errors.append(f"overflow={ps['overflow']}")
    if echo_pending and not (args.echo is None and args.echo_hex is None):
        hard_errors.append(f"echo pending={len(echo_pending)}")
    if echo_fail:
        hard_errors.append(f"echo diff={echo_fail}")
    if stop_reason == "link-error":
        hard_errors.append("link-error")
    if intentional_errors:
        # 注入异常时，丢帧/CRC 错是预期结果，只要求还有帧被正确解析
        reasons = [r for r in reasons if r not in ("lost=1",)]
        if checker.recv > 0:
            reasons = [r for r in reasons if not r.startswith(("lost=", "dup=", "order="))]
        printer.line("NOTE  sim anomaly injected, link error counters are expected")

    all_reasons = reasons + hard_errors
    if checker.recv == 0:
        detail = " ".join(all_reasons)
        printer.result(f"NO-DATA ({stop_reason})" + (f" {detail}" if detail else ""))
        return 2
    if all_reasons and not args.allow_errors:
        printer.result(f"FAIL ({stop_reason}) " + " ".join(all_reasons))
        return 1
    printer.result(f"PASS ({stop_reason}) frames={checker.recv} digest={checker.digest_final[:16]}")
    return 0


# ---------------------------------------------------------------------------
# 其他模式
# ---------------------------------------------------------------------------


def cmd_list(printer: Printer) -> int:
    try:
        ports = list_serial_ports()
    except TransportError as exc:
        printer.raw(f"LIST  {exc}")
        return 1
    if not ports:
        printer.raw("LIST  (no serial ports)")
        return 0
    for p in ports:
        printer.raw(f"LIST  {p.pretty()}")
    return 0


def cmd_scan_ble(printer: Printer, timeout: float) -> int:
    try:
        devices = scan_ble_devices(timeout=timeout)
    except TransportError as exc:
        printer.raw(f"BLE   {exc}")
        return 1
    if not devices:
        printer.raw("BLE   (no advertising devices)")
    for d in devices:
        services = ",".join(d["services"]) if d["services"] else "-"
        printer.raw(
            f"BLE   {d['address']}  rssi={d['rssi']}  name={d['name'] or '?'}  services={services}"
        )
    # 已配对设备可能正在被系统 HID 占用，不会出现在广播列表里。
    try:
        paired = list_paired_ble_devices()
    except TransportError as exc:
        printer.raw(f"PAIRED {exc}")
        return 0
    if paired:
        printer.raw("PAIRED (Windows 已配对，可直接用 --paired)")
        for d in paired:
            state = "connected" if d["connected"] else "paired"
            printer.raw(
                f"PAIRED {d['address']}  name={d['name'] or '?'}  "
                f"type={d['address_type']}  {state}"
            )
    return 0


def cmd_list_paired(printer: Printer) -> int:
    try:
        devices = list_paired_ble_devices()
    except TransportError as exc:
        printer.raw(f"PAIRED {exc}")
        return 1
    if not devices:
        printer.raw("PAIRED (no paired BLE devices)")
        return 0
    for d in devices:
        state = "connected" if d["connected"] else "paired"
        printer.raw(
            f"PAIRED {d['address']}  name={d['name'] or '?'}  "
            f"type={d['address_type']}  {state}"
        )
    return 0


def cmd_compare(printer: Printer, path_a: str, path_b: str) -> int:
    try:
        result: CompareResult = compare_records(path_a, path_b)
    except (OSError, ValueError) as exc:
        printer.raw(f"COMPARE error: {exc}")
        return 1
    for line in result.lines:
        printer.raw(f"COMPARE {line}")
    if not result.ok:
        for failure in result.failures:
            printer.raw(f"COMPARE   - {failure}")
    return 0 if result.ok else 1


def cmd_selftest() -> int:
    from .selftest import run_selftest

    return run_selftest()


# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------


def main(argv: Optional[Sequence[str]] = None) -> int:
    _configure_stdout()
    args = build_arg_parser().parse_args(argv)

    if args.selftest:
        return cmd_selftest()

    if args.gui:
        from .gui import main as gui_main

        return int(gui_main())

    if args.compare:
        return cmd_compare(Printer("COMPARE"), args.compare[0], args.compare[1])

    if args.list:
        return cmd_list(Printer("LIST"))

    if args.list_paired:
        return cmd_list_paired(Printer("PAIRED"))

    if args.scan_ble:
        return cmd_scan_ble(Printer("BLE"), args.scan_timeout)

    if args.paired and not (args.name or args.address):
        print("--paired 需要同时指定 --name 或 --address", file=sys.stderr)
        return 1
    if args.paired and args.transport != "ble":
        print("--paired 只适用于 --transport ble", file=sys.stderr)
        return 1

    if args.echo_count < 1:
        print("--echo-count 必须 >= 1", file=sys.stderr)
        return 1
    if args.echo_seq < 0 or args.echo_seq > 0xFFFF:
        print("--echo-seq 必须在 0..65535", file=sys.stderr)
        return 1
    if args.max_frames < 0 or args.duration < 0:
        print("--max-frames/--duration 不能为负", file=sys.stderr)
        return 1

    transport_name = {"usb": "USB-CDC", "ble": "BLE-GATT", "sim": "SIM"}[args.transport]
    printer = Printer(transport_name, quiet=args.quiet)
    try:
        return run_capture(args, printer)
    except KeyboardInterrupt:
        printer.raw("")
        printer.raw(f"[{_timestamp()}] [{transport_name}] RESULT ABORTED")
        return 130
    except Exception as exc:  # 兜底：任何异常都不能表现为"崩栈"
        import traceback

        printer.raw(f"[{_timestamp()}] [{transport_name}] ERROR {exc}")
        if os.environ.get("HOST_TEST_DEBUG"):
            traceback.print_exc()
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
