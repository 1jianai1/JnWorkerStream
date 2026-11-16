"""把识别文本注入当前焦点窗口。

优先级（docs/语音输入方案 §4.4）：

1. ``SendInput + KEYEVENTF_UNICODE``：短文本、通用性最好；
2. 剪贴板 + Ctrl+V：SendInput 失败或长文本时兜底，尽量保存/恢复原剪贴板文本；
3. 目标窗口以管理员权限运行时，UIPI 会拦截注入，必须给出明确提示，不能静默失败。
"""

from __future__ import annotations

import ctypes
import sys
import time
from dataclasses import dataclass
from typing import Optional, TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover
    from ctypes import wintypes

IS_WINDOWS = sys.platform == "win32"

if IS_WINDOWS:  # pragma: no cover - Windows 专用
    from ctypes import wintypes

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    ULONG_PTR = ctypes.c_size_t
    KEYEVENTF_KEYUP = 0x0002
    KEYEVENTF_UNICODE = 0x0004
    INPUT_KEYBOARD = 1
    VK_CONTROL = 0x11
    VK_V = 0x56
    VK_RETURN = 0x0D
    CF_UNICODETEXT = 13
    GMEM_MOVEABLE = 0x0002

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
        _fields_ = [
            ("uMsg", wintypes.DWORD),
            ("wParamL", wintypes.WORD),
            ("wParamH", wintypes.WORD),
        ]

    class _INPUTUNION(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT), ("hi", HARDWAREINPUT)]

    class INPUT(ctypes.Structure):
        _anonymous_ = ("u",)
        _fields_ = [("type", wintypes.DWORD), ("u", _INPUTUNION)]

    user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
    user32.SendInput.restype = wintypes.UINT
    user32.GetForegroundWindow.restype = wintypes.HWND
    user32.OpenClipboard.argtypes = [wintypes.HWND]
    user32.OpenClipboard.restype = wintypes.BOOL
    user32.CloseClipboard.restype = wintypes.BOOL
    user32.EmptyClipboard.restype = wintypes.BOOL
    user32.IsClipboardFormatAvailable.argtypes = [wintypes.UINT]
    user32.IsClipboardFormatAvailable.restype = wintypes.BOOL
    user32.GetClipboardData.argtypes = [wintypes.UINT]
    user32.GetClipboardData.restype = wintypes.HANDLE
    user32.SetClipboardData.argtypes = [wintypes.UINT, wintypes.HANDLE]
    user32.SetClipboardData.restype = wintypes.HANDLE
    kernel32.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
    kernel32.GlobalAlloc.restype = wintypes.HGLOBAL
    kernel32.GlobalLock.argtypes = [wintypes.HGLOBAL]
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
    kernel32.GlobalUnlock.restype = wintypes.BOOL


@dataclass(slots=True)
class InjectResult:
    ok: bool
    method: str      # "sendinput" | "clipboard" | "none"
    chars: int
    error: str = ""


class Injector:
    """Windows 文本注入。非 Windows 平台返回明确错误。"""

    def __init__(self, per_char_ms: float = 6.0, focus_guard: bool = True) -> None:
        self.per_char_ms = max(0.0, float(per_char_ms))
        self.focus_guard = focus_guard

    def inject(self, text: str, mode: str = "auto") -> InjectResult:
        text = text or ""
        if not text:
            return InjectResult(True, "none", 0)
        if mode in ("off", "none"):
            return InjectResult(False, "none", 0, "注入已关闭")
        if not IS_WINDOWS:
            return InjectResult(False, "none", len(text), "文本注入仅支持 Windows")
        if mode not in ("auto", "sendinput", "clipboard"):
            return InjectResult(False, "none", len(text), f"未知注入模式: {mode}")

        if mode == "sendinput":
            return self._sendinput(text)
        if mode == "clipboard":
            return self._clipboard(text)

        # auto：短文本 SendInput；长文本优先剪贴板；失败互相兜底
        if len(text) > 40:
            result = self._clipboard(text)
            if result.ok:
                return result
            fallback = self._sendinput(text)
            if fallback.ok:
                fallback.error = result.error
                return fallback
            fallback.error = f"{result.error}；SendInput 兜底也失败：{fallback.error}"
            return fallback

        result = self._sendinput(text)
        if result.ok:
            return result
        fallback = self._clipboard(text)
        if fallback.ok:
            fallback.error = result.error
            return fallback
        fallback.error = f"{result.error}；剪贴板兜底也失败：{fallback.error}"
        return fallback

    # -- SendInput --------------------------------------------------------

    def _foreground(self) -> int:
        if not IS_WINDOWS:
            return 0
        return int(user32.GetForegroundWindow() or 0)

    def _send_units(self, units: list[int]) -> bool:
        """发送一组 UTF-16 码元（每个码元 down+up）。"""
        if not IS_WINDOWS:
            return False
        if not units:
            return True
        inputs = (INPUT * (len(units) * 2))()
        index = 0
        for unit in units:
            for up in (False, True):
                item = inputs[index]
                item.type = INPUT_KEYBOARD
                item.ki.wVk = 0
                item.ki.wScan = unit & 0xFFFF
                item.ki.dwFlags = KEYEVENTF_UNICODE | (KEYEVENTF_KEYUP if up else 0)
                item.ki.time = 0
                item.ki.dwExtraInfo = 0
                index += 1
        sent = user32.SendInput(len(inputs), inputs, ctypes.sizeof(INPUT))
        return sent == len(inputs)

    def _send_key(self, vk: int, up: bool = False) -> bool:
        if not IS_WINDOWS:
            return False
        item = INPUT()
        item.type = INPUT_KEYBOARD
        item.ki.wVk = vk
        item.ki.wScan = 0
        item.ki.dwFlags = KEYEVENTF_KEYUP if up else 0
        item.ki.time = 0
        item.ki.dwExtraInfo = 0
        return user32.SendInput(1, ctypes.byref(item), ctypes.sizeof(INPUT)) == 1

    def _sendinput(self, text: str) -> InjectResult:
        hwnd = self._foreground()
        if hwnd == 0:
            return InjectResult(False, "sendinput", 0, "没有可注入的前台窗口")
        inserted = 0
        try:
            chars = list(text)
            index = 0
            while index < len(chars):
                ch = chars[index]
                if ch == "\r" and index + 1 < len(chars) and chars[index + 1] == "\n":
                    index += 1
                    continue
                if ch in ("\r", "\n"):
                    if self.focus_guard and self._foreground() != hwnd:
                        raise OSError("注入过程中焦点窗口发生变化，已中止")
                    if not self._send_key(VK_RETURN, False):
                        raise OSError(self._sendinput_error())
                    if not self._send_key(VK_RETURN, True):
                        raise OSError(self._sendinput_error())
                else:
                    encoded = ch.encode("utf-16-le")
                    units = [
                        encoded[i] | (encoded[i + 1] << 8)
                        for i in range(0, len(encoded), 2)
                    ]
                    if not self._send_units(units):
                        raise OSError(self._sendinput_error())
                inserted += 1
                index += 1
                if self.per_char_ms > 0:
                    time.sleep(self.per_char_ms / 1000.0)
                if self.focus_guard and inserted % 8 == 0 and self._foreground() != hwnd:
                    raise OSError("注入过程中焦点窗口发生变化，已中止")
        except OSError as exc:
            return InjectResult(
                False, "sendinput", inserted,
                f"{exc}；若目标窗口以管理员运行，请以管理员身份启动上位机",
            )

        if self.focus_guard and self._foreground() != hwnd:
            return InjectResult(
                False, "sendinput", inserted,
                "注入后焦点窗口发生变化，文字可能没有完整进入目标窗口",
            )
        return InjectResult(True, "sendinput", len(text))

    @staticmethod
    def _sendinput_error() -> str:
        if not IS_WINDOWS:
            return "非 Windows 平台"
        code = ctypes.get_last_error()
        return f"SendInput 失败（WinError={code}），可能被 UIPI/权限拦截"

    # -- 剪贴板 -----------------------------------------------------------

    @staticmethod
    def _clipboard_text() -> Optional[str]:
        if not IS_WINDOWS:
            return None
        for _ in range(5):
            if not user32.OpenClipboard(None):
                time.sleep(0.02)
                continue
            try:
                if not user32.IsClipboardFormatAvailable(CF_UNICODETEXT):
                    return None
                handle = user32.GetClipboardData(CF_UNICODETEXT)
                if not handle:
                    return None
                ptr = kernel32.GlobalLock(handle)
                if not ptr:
                    return None
                try:
                    return ctypes.wstring_at(ptr)
                finally:
                    kernel32.GlobalUnlock(handle)
            finally:
                user32.CloseClipboard()
        return None

    @staticmethod
    def _set_clipboard_text(text: Optional[str]) -> bool:
        if not IS_WINDOWS:
            return False
        for _ in range(5):
            if not user32.OpenClipboard(None):
                time.sleep(0.02)
                continue
            try:
                if not user32.EmptyClipboard():
                    return False
                if text is None:
                    return True
                encoded = (text + "\x00").encode("utf-16-le")
                handle = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(encoded))
                if not handle:
                    return False
                ptr = kernel32.GlobalLock(handle)
                if not ptr:
                    return False
                try:
                    ctypes.memmove(ptr, encoded, len(encoded))
                finally:
                    kernel32.GlobalUnlock(handle)
                if not user32.SetClipboardData(CF_UNICODETEXT, handle):
                    return False
                return True
            finally:
                user32.CloseClipboard()
        return False

    def _clipboard(self, text: str) -> InjectResult:
        old_text = self._clipboard_text()
        hwnd = self._foreground()
        if hwnd == 0:
            return InjectResult(False, "clipboard", 0, "没有可注入的前台窗口")
        if not self._set_clipboard_text(text):
            return InjectResult(False, "clipboard", 0, "写入剪贴板失败（可能被其他程序占用）")
        try:
            # Qt 等框架会缓存剪贴板，留出处理 WM_CLIPBOARDUPDATE 的时间
            time.sleep(0.25)
            if self.focus_guard and self._foreground() != hwnd:
                return InjectResult(False, "clipboard", 0, "粘贴前焦点窗口发生变化，已中止")
            if not self._send_key(VK_CONTROL, False):
                return InjectResult(False, "clipboard", 0, "发送 Ctrl 失败")
            if not self._send_key(VK_V, False):
                self._send_key(VK_CONTROL, True)
                return InjectResult(False, "clipboard", 0, "发送 V 失败")
            self._send_key(VK_V, True)
            self._send_key(VK_CONTROL, True)
            time.sleep(0.6)
            if self.focus_guard and self._foreground() != hwnd:
                return InjectResult(
                    False, "clipboard", 0,
                    "粘贴后焦点窗口发生变化，文字可能进入了错误窗口",
                )
            return InjectResult(True, "clipboard", len(text))
        finally:
            # 给目标程序留出读取剪贴板的时间，然后尽量恢复。
            # 原来没有文本（例如剪贴板里是图片）时不主动清空，避免破坏用户内容。
            try:
                time.sleep(0.3)
                if old_text is not None:
                    self._set_clipboard_text(old_text)
            except Exception:
                pass
