"""端到端手验（带对照）：Tk Text 窗口，先由 Rust 注入，再由 ctypes 注入对照字符。

用法（必须在**交互式桌面会话**里跑，否则 Windows 不允许抢占前台窗口）::

    cargo build -p kb-voice --example voice_e2e
    python crates/kb-voice/examples/inject_e2e.py

预期：窗口里出现识别文本（如「他怎么卡住了」），末尾还会多一个对照字符 "Q"；
脚本最后把 Text 内容打印出来，两者都出现才算注入真正送达。
"""

import ctypes
import subprocess
import sys
import threading
import time
import tkinter as tk
from ctypes import wintypes
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
MARKER = ROOT / ".cache" / "inject_ready.marker"
EXE = ROOT / "target" / "debug" / "examples" / "voice_e2e.exe"
WAV = ROOT / "target" / "hw" / "seg5_05.wav"
MODEL = ROOT / "models" / "faster-whisper-small"

user32 = ctypes.WinDLL("user32", use_last_error=True)
ULONG_PTR = ctypes.c_size_t
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004
INPUT_KEYBOARD = 1


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", wintypes.DWORD), ("wParamL", wintypes.WORD), ("wParamH", wintypes.WORD)]


class _U(ctypes.Union):
    _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", wintypes.DWORD), ("u", _U)]


def type_text(text: str) -> None:
    units = []
    for ch in text:
        encoded = ch.encode("utf-16-le")
        units.extend(encoded[i] | (encoded[i + 1] << 8) for i in range(0, len(encoded), 2))
    inputs = (INPUT * (len(units) * 2))()
    index = 0
    for unit in units:
        for up in (False, True):
            item = inputs[index]
            item.type = INPUT_KEYBOARD
            item.ki.wVk = 0
            item.ki.wScan = unit & 0xFFFF
            item.ki.dwFlags = KEYEVENTF_UNICODE | (KEYEVENTF_KEYUP if up else 0)
            index += 1
    sent = user32.SendInput(len(inputs), inputs, ctypes.sizeof(INPUT))
    print(f"control_sent={sent}/{len(inputs)}", flush=True)


def main() -> int:
    if MARKER.exists():
        MARKER.unlink()

    root = tk.Tk()
    root.title("kb-inject e2e")
    root.geometry("640x200+120+120")
    text = tk.Text(root, width=80, height=8, font=("Consolas", 14))
    text.pack(fill="both", expand=True)
    root.attributes("-topmost", True)
    root.update()
    text.focus_set()
    text.bind("<Key>", lambda event: print(f"key {event.char!r}", flush=True))

    hwnd = root.winfo_id()
    parent = ctypes.windll.user32.GetParent(hwnd)

    def focus() -> None:
        root.lift()
        text.focus_set()
        root.focus_force()
        for target in (parent, hwnd):
            if target:
                ctypes.windll.user32.SetForegroundWindow(target)
        print(f"foreground={ctypes.windll.user32.GetForegroundWindow()} hwnd={hwnd}", flush=True)

    child = subprocess.Popen(
        [str(EXE), str(WAV), str(MODEL), str(MARKER)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        cwd=str(ROOT),
    )

    def pump() -> None:
        for line in child.stdout:  # type: ignore[union-attr]
            print("child| " + line.decode("utf-8", "replace").rstrip(), flush=True)

    threading.Thread(target=pump, daemon=True).start()

    state = {"done": False}

    def check() -> None:
        if child.poll() is not None:
            state["done"] = True
            return
        root.after(200, check)

    root.after(600, focus)
    root.after(1400, focus)
    root.after(1600, lambda: MARKER.write_text("go", encoding="utf-8"))
    # 对照：Rust 注入之后，再由本进程直接 SendInput 一个 "Q"。
    root.after(4200, lambda: type_text("Q"))
    root.after(200, check)

    def finish() -> None:
        if not state["done"]:
            root.after(200, finish)
            return
        time.sleep(1.0)
        content = text.get("1.0", "end-1c")
        print("CONTENT_BEGIN", flush=True)
        print(repr(content), flush=True)
        print("CONTENT_END", flush=True)
        root.destroy()

    root.after(1500, finish)
    root.mainloop()
    child.wait(timeout=60)
    print("child_exit=", child.returncode, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
