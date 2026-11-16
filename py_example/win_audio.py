"""Windows 流式 PCM 播放（waveOut），用于界面里的实时监听。

不引入 QtMultimedia / sounddevice 等依赖：PySide6-Essentials 不含 QtMultimedia，
而 waveOut 是系统自带的，16 kHz 单声道的延迟和抖动都够用。

用法::

    out = WinAudioOut(rate=16000, channels=1, bits=16)
    out.start()
    out.write(pcm_bytes)   # 可以随时喂，内部按 buffer 切片
    out.poll()             # 建议 20 ms 调一次，回收已播完的 buffer
    out.stop()

设计：4 个 40 ms 的循环 buffer。没有数据时不提交 buffer（自然静音），
有数据时立刻补上，因此正常情况下的额外延迟约 40–80 ms。
"""

from __future__ import annotations

import ctypes
from ctypes import wintypes
from typing import List, Optional

WAVE_MAPPER = 0xFFFFFFFF
WAVE_FORMAT_PCM = 1
CALLBACK_NULL = 0

WHDR_DONE = 0x00000001
WHDR_PREPARED = 0x00000002

winmm = ctypes.WinDLL("winmm")


class WAVEFORMATEX(ctypes.Structure):
    _fields_ = [
        ("wFormatTag", wintypes.WORD),
        ("nChannels", wintypes.WORD),
        ("nSamplesPerSec", wintypes.DWORD),
        ("nAvgBytesPerSec", wintypes.DWORD),
        ("nBlockAlign", wintypes.WORD),
        ("wBitsPerSample", wintypes.WORD),
        ("cbSize", wintypes.WORD),
    ]


class WAVEHDR(ctypes.Structure):
    _fields_ = [
        ("lpData", ctypes.c_void_p),
        ("dwBufferLength", wintypes.DWORD),
        ("dwBytesRecorded", wintypes.DWORD),
        ("dwUser", ctypes.c_size_t),
        ("dwFlags", wintypes.DWORD),
        ("dwLoops", wintypes.DWORD),
        ("lpNext", ctypes.c_void_p),
        ("reserved", ctypes.c_size_t),
    ]


winmm.waveOutOpen.argtypes = [
    ctypes.POINTER(wintypes.HANDLE), wintypes.UINT, ctypes.POINTER(WAVEFORMATEX),
    ctypes.c_size_t, ctypes.c_size_t, wintypes.DWORD,
]
winmm.waveOutPrepareHeader.argtypes = [
    wintypes.HANDLE, ctypes.POINTER(WAVEHDR), wintypes.UINT,
]
winmm.waveOutUnprepareHeader.argtypes = [
    wintypes.HANDLE, ctypes.POINTER(WAVEHDR), wintypes.UINT,
]
winmm.waveOutWrite.argtypes = [wintypes.HANDLE, ctypes.POINTER(WAVEHDR), wintypes.UINT]
winmm.waveOutReset.argtypes = [wintypes.HANDLE]
winmm.waveOutClose.argtypes = [wintypes.HANDLE]


class WinAudioOut:
    """16 bit PCM 的流式播放器（Windows waveOut）。"""

    def __init__(
        self,
        rate: int = 16000,
        channels: int = 1,
        bits: int = 16,
        buf_ms: int = 40,
        buffers: int = 4,
    ) -> None:
        self.rate = rate
        self.channels = channels
        self.bits = bits
        self.block_align = channels * bits // 8
        self.buf_bytes = max(self.block_align, rate * self.block_align * buf_ms // 1000)
        self.buffer_count = max(2, buffers)

        self._fmt = WAVEFORMATEX(
            WAVE_FORMAT_PCM,
            channels,
            rate,
            rate * self.block_align,
            self.block_align,
            bits,
            0,
        )
        self._hwo = wintypes.HANDLE()
        self._opened = False
        self._bufs = [ctypes.create_string_buffer(self.buf_bytes) for _ in range(self.buffer_count)]
        self._hdrs = [WAVEHDR() for _ in range(self.buffer_count)]
        self._in_use = [False] * self.buffer_count
        self._pending = bytearray()
        self.underruns = 0

    # -- 生命周期 ---------------------------------------------------------

    def start(self) -> None:
        if self._opened:
            return
        rc = winmm.waveOutOpen(
            ctypes.byref(self._hwo), WAVE_MAPPER, ctypes.byref(self._fmt), 0, 0, CALLBACK_NULL
        )
        if rc != 0:
            raise OSError(f"waveOutOpen 失败: {rc}")
        self._opened = True
        self._pending.clear()
        self._in_use = [False] * self.buffer_count

    def stop(self) -> None:
        if not self._opened:
            return
        winmm.waveOutReset(self._hwo)
        for i in range(self.buffer_count):
            if self._in_use[i]:
                winmm.waveOutUnprepareHeader(
                    self._hwo, ctypes.byref(self._hdrs[i]), ctypes.sizeof(WAVEHDR)
                )
                self._in_use[i] = False
        winmm.waveOutClose(self._hwo)
        self._opened = False
        self._pending.clear()

    # -- 数据 -------------------------------------------------------------

    def write(self, pcm: bytes) -> None:
        if not self._opened or not pcm:
            return
        self._pending += pcm
        self._pump()

    def poll(self) -> None:
        """回收已播完的 buffer 并继续提交。建议 20 ms 调一次。"""
        if not self._opened:
            return

        for i in range(self.buffer_count):
            if not self._in_use[i]:
                continue
            if self._hdrs[i].dwFlags & WHDR_DONE:
                winmm.waveOutUnprepareHeader(
                    self._hwo, ctypes.byref(self._hdrs[i]), ctypes.sizeof(WAVEHDR)
                )
                self._in_use[i] = False
                self._hdrs[i].dwFlags = 0

        self._pump()

    def _pump(self) -> None:
        for i in range(self.buffer_count):
            if len(self._pending) < self.buf_bytes:
                break
            if self._in_use[i]:
                continue

            chunk = bytes(self._pending[: self.buf_bytes])
            del self._pending[: self.buf_bytes]

            ctypes.memmove(self._bufs[i], chunk, len(chunk))
            hdr = self._hdrs[i]
            hdr.lpData = ctypes.cast(self._bufs[i], ctypes.c_void_p)
            hdr.dwBufferLength = len(chunk)
            hdr.dwBytesRecorded = 0
            hdr.dwFlags = 0
            hdr.dwLoops = 0

            if winmm.waveOutPrepareHeader(self._hwo, ctypes.byref(hdr), ctypes.sizeof(WAVEHDR)) != 0:
                continue
            if winmm.waveOutWrite(self._hwo, ctypes.byref(hdr), ctypes.sizeof(WAVEHDR)) != 0:
                winmm.waveOutUnprepareHeader(self._hwo, ctypes.byref(hdr), ctypes.sizeof(WAVEHDR))
                continue
            self._in_use[i] = True

    @property
    def pending_bytes(self) -> int:
        return len(self._pending)
