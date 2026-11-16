"""PySide6 图形界面：浅色主题的上位机验证台。

复用 ``frame.FrameParser``、``check.Check`` 与 ``transport``，不改变协议层。
启动::

    python -m host_test --gui
    python -m host_test.gui

依赖：``pip install PySide6-Essentials``。
"""

from __future__ import annotations

import os
import queue
import sys
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Deque, Dict, List, Optional, Tuple

try:
    from PySide6.QtCore import Qt, QThread, QTimer, Signal
    from PySide6.QtGui import (
        QColor,
        QFont,
        QFontDatabase,
        QIcon,
        QPainter,
        QPalette,
        QPixmap,
        QTextBlockFormat,
        QTextCharFormat,
        QTextCursor,
        QTextOption,
    )
    from PySide6.QtWidgets import (
        QApplication,
        QCheckBox,
        QComboBox,
        QFileDialog,
        QFormLayout,
        QFrame,
        QGridLayout,
        QHBoxLayout,
        QLabel,
        QLineEdit,
        QMainWindow,
        QMessageBox,
        QPlainTextEdit,
        QPushButton,
        QScrollArea,
        QSizePolicy,
        QSpinBox,
        QStackedWidget,
        QStatusBar,
        QToolButton,
        QVBoxLayout,
        QWidget,
    )
except ImportError as exc:  # pragma: no cover - 环境相关
    raise SystemExit(
        "未安装 PySide6，无法启动图形界面。请执行：\n"
        "  D:\\Miniconda\\envs\\kbhost\\python.exe -m pip install PySide6-Essentials\n"
        "然后重新运行 python -m host_test --gui"
    ) from exc

from . import __version__
from .check import Checker, Recorder, compare_records
from .cli import _short_data
from .frame import (
    TEST_FLAG_ECHO,
    TYPE_AUDIO_CTL,
    TYPE_AUDIO_DATA,
    TYPE_TEST_DATA,
    Frame,
    FrameParser,
    build_frame,
    build_test_payload,
)
from .audio_rx import SAMPLE_RATE, write_wav
from .voice.session import AudioSegment, VoiceSession, decode_segment
from .voice.asr.faster_whisper_engine import FasterWhisperEngine
from .voice.controller import VoiceController
from .voice.inject import Injector

try:  # 实时播放走 Windows waveOut，非 Windows 环境降级为只录制
    from .win_audio import WinAudioOut
except Exception:  # pragma: no cover
    WinAudioOut = None
from .transport import (
    BleTransport,
    SerialTransport,
    SimTransport,
    Transport,
    TransportError,
    list_paired_ble_devices,
    list_serial_ports,
)


# ---------------------------------------------------------------------------
# 颜色 / 主题
# ---------------------------------------------------------------------------

BG = "#f3f6fb"
CARD = "#ffffff"
BORDER = "#e4eaf3"
TEXT = "#0f172a"
MUTED = "#64748b"
FAINT = "#94a3b8"
PRIMARY = "#2563eb"
PRIMARY_DARK = "#1d4ed8"
GREEN = "#059669"
TEAL = "#0f766e"
AMBER = "#d97706"
RED = "#dc2626"
VIOLET = "#7c3aed"
INDIGO = "#4f46e5"

LEVEL_META: Dict[str, Tuple[str, str]] = {
    "link": ("LINK", PRIMARY),
    "frame": ("FRAME", "#64748b"),
    "data": ("DATA", TEAL),
    "stat": ("STAT", FAINT),
    "echo": ("ECHO", VIOLET),
    "voice": ("VOICE", "#c026d3"),
    "warn": ("WARN", AMBER),
    "error": ("ERROR", RED),
    "rec": ("REC", GREEN),
    "info": ("INFO", "#475569"),
}

LEVEL_FILTER = {
    "frame": "data",
    "data": "data",
    "echo": "echo",
    "stat": "stat",
    "link": "link",
    "rec": "link",
    "info": "link",
    "voice": "voice",
    "warn": "error",
    "error": "error",
}


def _now_ts() -> str:
    return datetime.now().strftime("%H:%M:%S.%f")[:-3]


def _repolish(widget: QWidget) -> None:
    widget.style().unpolish(widget)
    widget.style().polish(widget)
    widget.update()


def _fmt_int(value: int) -> str:
    return f"{int(value):,}"


def _fmt_bytes(value: float) -> str:
    value = float(value)
    if value < 1024:
        return f"{value:.0f} B"
    if value < 1024 * 1024:
        return f"{value / 1024.0:.1f} KB"
    if value < 1024 * 1024 * 1024:
        return f"{value / (1024.0 * 1024.0):.2f} MB"
    return f"{value / (1024.0 ** 3):.2f} GB"


def _mono_font(size: float = 10.0) -> QFont:
    families = set(QFontDatabase.families())
    for name in ("Cascadia Mono", "JetBrains Mono", "Consolas", "DejaVu Sans Mono"):
        if name in families:
            font = QFont(name)
            font.setPointSizeF(size)
            return font
    font = QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont)
    font.setPointSizeF(size)
    return font


def _make_icon() -> QIcon:
    pix = QPixmap(64, 64)
    pix.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pix)
    try:
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        painter.setPen(Qt.PenStyle.NoPen)
        painter.setBrush(QColor(PRIMARY))
        painter.drawRoundedRect(3, 3, 58, 58, 15, 15)
        painter.setPen(QColor("#ffffff"))
        font = QFont("Microsoft YaHei UI")
        font.setBold(True)
        font.setPixelSize(25)
        painter.setFont(font)
        painter.drawText(pix.rect(), Qt.AlignmentFlag.AlignCenter, "KB")
    finally:
        painter.end()
    return QIcon(pix)


QSS = f"""
* {{
    font-family: "Microsoft YaHei UI", "Segoe UI", sans-serif;
    font-size: 13px;
    color: {TEXT};
}}
QMainWindow, QWidget#Root {{
    background: {BG};
}}
QWidget#SidebarContent {{
    background: transparent;
}}
QScrollArea {{
    background: transparent;
    border: none;
}}
QScrollArea > QWidget > QWidget {{
    background: transparent;
}}
QFrame#Card {{
    background: {CARD};
    border: 1px solid {BORDER};
    border-radius: 14px;
}}
QLabel#AppTitle {{
    font-size: 23px;
    font-weight: 700;
    color: {TEXT};
}}
QLabel#AppSubtitle {{
    font-size: 12px;
    color: {MUTED};
}}
QLabel#AppIcon {{
    background: {PRIMARY};
    color: white;
    border-radius: 12px;
    font-size: 16px;
    font-weight: 700;
}}
QLabel#CardHeading {{
    font-size: 15px;
    font-weight: 700;
    color: {TEXT};
}}
QLabel#SectionHeading {{
    font-size: 12px;
    font-weight: 600;
    color: #475569;
}}
QLabel#FormLabel {{
    color: {MUTED};
    font-size: 12px;
}}
QLabel#Hint {{
    color: {FAINT};
    font-size: 11px;
}}
QLabel#VoiceStatus {{
    color: #7c3aed;
    font-size: 11px;
}}
QLabel#VoiceResult {{
    background: #faf7ff;
    border: 1px solid #ece3fb;
    border-radius: 8px;
    padding: 7px 9px;
    color: #3b0764;
    font-size: 12px;
}}
QLabel#Muted {{
    color: {FAINT};
    font-size: 11px;
}}
QLabel#LinkInfo {{
    font-size: 12px;
    font-weight: 600;
    color: #334155;
}}
QLabel#MtuInfo {{
    font-size: 11px;
    color: {FAINT};
}}
QFrame#Divider {{
    background: #edf2f8;
    border: none;
    min-height: 1px;
    max-height: 1px;
}}
QFrame#StatusPill {{
    background: #f1f5f9;
    border: 1px solid #e2e8f0;
    border-radius: 14px;
}}
QFrame#StatusPill[state="connected"] {{
    background: #ecfdf5;
    border-color: #a7f3d0;
}}
QFrame#StatusPill[state="connecting"] {{
    background: #fffbeb;
    border-color: #fde68a;
}}
QFrame#StatusPill[state="reconnect"] {{
    background: #fff7ed;
    border-color: #fed7aa;
}}
QFrame#StatusPill[state="error"] {{
    background: #fef2f2;
    border-color: #fecaca;
}}
QLabel#StatusText {{
    font-size: 11px;
    font-weight: 600;
    color: #475569;
}}
QFrame#StatCard {{
    background: {CARD};
    border: 1px solid {BORDER};
    border-radius: 12px;
}}
QLabel#StatTitle {{
    color: {MUTED};
    font-size: 12px;
}}
QLabel#StatValue {{
    color: {TEXT};
    font-size: 23px;
    font-weight: 700;
}}
QLabel#StatValue[tone="bad"] {{
    color: {RED};
}}
QLabel#StatValue[tone="warn"] {{
    color: {AMBER};
}}
QLabel#StatValue[tone="good"] {{
    color: {GREEN};
}}
QLabel#StatHint {{
    color: {FAINT};
    font-size: 11px;
}}
QPushButton {{
    background: #eef3fa;
    color: #334155;
    border: 1px solid transparent;
    border-radius: 8px;
    padding: 7px 14px;
    min-height: 18px;
}}
QPushButton:hover {{
    background: #e4ecf8;
}}
QPushButton:pressed {{
    background: #dbe6f6;
}}
QPushButton:disabled {{
    color: #a8b3c4;
    background: #f3f6fa;
}}
QPushButton#Primary {{
    background: {PRIMARY};
    color: white;
    border: none;
    font-weight: 600;
    padding: 9px 18px;
}}
QPushButton#Primary:hover {{
    background: {PRIMARY_DARK};
}}
QPushButton#Primary:disabled {{
    background: #c7d7f3;
    color: #f8fbff;
}}
QPushButton#DangerGhost {{
    background: white;
    color: {RED};
    border: 1px solid #fecaca;
    padding: 9px 16px;
}}
QPushButton#DangerGhost:hover {{
    background: #fef2f2;
}}
QPushButton#DangerGhost:disabled {{
    color: #e5a6a6;
    border-color: #f4dddd;
    background: white;
}}
QPushButton#RecordButton:checked {{
    background: #fef2f2;
    color: {RED};
    border: 1px solid #fecaca;
    font-weight: 600;
}}
QPushButton#Ghost {{
    background: transparent;
    color: #52627a;
    border: 1px solid {BORDER};
    padding: 7px 12px;
}}
QPushButton#Ghost:hover {{
    background: #f5f8fd;
}}
QToolButton {{
    background: transparent;
    color: #52627a;
    border: 1px solid transparent;
    border-radius: 7px;
    padding: 5px 9px;
}}
QToolButton:hover {{
    background: #eef3fa;
    border-color: {BORDER};
}}
QToolButton:checked {{
    background: #e8f0fe;
    color: {PRIMARY};
}}
QLineEdit, QComboBox, QSpinBox {{
    background: #f8fafd;
    border: 1px solid #dce5f1;
    border-radius: 8px;
    padding: 6px 9px;
    min-height: 20px;
    selection-background-color: #dbeafe;
    selection-color: {TEXT};
}}
QLineEdit:focus, QComboBox:focus, QSpinBox:focus {{
    border-color: #93c5fd;
    background: white;
}}
QLineEdit:disabled, QComboBox:disabled, QSpinBox:disabled {{
    color: #9aa8ba;
    background: #f4f7fb;
}}
QComboBox::drop-down {{
    border: none;
    width: 22px;
}}
QComboBox QAbstractItemView {{
    background: white;
    border: 1px solid {BORDER};
    border-radius: 8px;
    padding: 4px;
    selection-background-color: #eaf1fe;
    selection-color: {TEXT};
    outline: none;
}}
QCheckBox {{
    spacing: 7px;
    color: #475569;
    font-size: 12px;
}}
QPlainTextEdit#Log {{
    background: #fbfdff;
    border: 1px solid #edf2f8;
    border-radius: 10px;
    padding: 9px;
    selection-background-color: #dbeafe;
    selection-color: {TEXT};
}}
QComboBox#Filter {{
    min-width: 116px;
}}
QLineEdit#Search {{
    min-width: 150px;
}}
QStatusBar {{
    background: #eef3fa;
    color: {MUTED};
    border-top: 1px solid #e2e9f3;
    font-size: 11px;
}}
QStatusBar::item {{
    border: none;
}}
QScrollBar:vertical {{
    background: transparent;
    width: 10px;
    margin: 3px 2px 3px 0;
}}
QScrollBar::handle:vertical {{
    background: #cbd8e9;
    min-height: 30px;
    border-radius: 5px;
}}
QScrollBar::handle:vertical:hover {{
    background: #aebfd6;
}}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {{
    height: 0;
}}
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {{
    background: transparent;
}}
QScrollBar:horizontal {{
    background: transparent;
    height: 10px;
}}
QScrollBar::handle:horizontal {{
    background: #cbd8e9;
    min-width: 30px;
    border-radius: 5px;
}}
QToolTip {{
    background: #0f172a;
    color: white;
    border: none;
    padding: 6px 8px;
    border-radius: 6px;
}}
"""


# ---------------------------------------------------------------------------
# 日志视图
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class LogEntry:
    ts: str
    level: str
    text: str


class LogView(QFrame):
    """可暂停、筛选、搜索、导出的彩色日志面板。"""

    MAX_ENTRIES = 5000

    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self.setObjectName("Card")
        self.entries: Deque[LogEntry] = deque(maxlen=self.MAX_ENTRIES)
        self._filter = "all"
        self._search = ""
        self._paused = False

        self._fmt_time = QTextCharFormat()
        self._fmt_time.setForeground(QColor(FAINT))
        self._fmt_level: Dict[str, QTextCharFormat] = {}
        self._fmt_text: Dict[str, QTextCharFormat] = {}
        self._fmt_block: Dict[str, QTextBlockFormat] = {}
        for level, (_label, color) in LEVEL_META.items():
            fmt = QTextCharFormat()
            fmt.setForeground(QColor(color))
            fmt.setFontWeight(
                QFont.Weight.Bold if level in ("link", "echo", "warn", "error") else QFont.Weight.Normal
            )
            self._fmt_level[level] = fmt

            text_fmt = QTextCharFormat()
            if level == "error":
                text_fmt.setForeground(QColor("#b91c1c"))
                text_fmt.setFontWeight(QFont.Weight.DemiBold)
            elif level == "warn":
                text_fmt.setForeground(QColor("#92400e"))
            elif level == "stat":
                text_fmt.setForeground(QColor("#7b8aa0"))
            elif level == "data":
                text_fmt.setForeground(QColor("#334155"))
            else:
                text_fmt.setForeground(QColor("#1e293b"))
            self._fmt_text[level] = text_fmt

            block_fmt = QTextBlockFormat()
            block_fmt.setLineHeight(
                125.0, QTextBlockFormat.LineHeightTypes.ProportionalHeight.value
            )
            block_fmt.setTopMargin(1)
            block_fmt.setBottomMargin(1)
            if level == "error":
                block_fmt.setBackground(QColor("#fef2f2"))
            elif level == "warn":
                block_fmt.setBackground(QColor("#fffbeb"))
            self._fmt_block[level] = block_fmt

        self._build_ui()

    def _build_ui(self) -> None:
        layout = QVBoxLayout(self)
        layout.setContentsMargins(15, 13, 15, 13)
        layout.setSpacing(10)

        top = QHBoxLayout()
        top.setSpacing(8)
        title = QLabel("运行日志")
        title.setObjectName("CardHeading")
        top.addWidget(title)
        self.count_label = QLabel("0 条")
        self.count_label.setObjectName("Muted")
        top.addWidget(self.count_label)
        top.addStretch(1)

        self.filter_combo = QComboBox()
        self.filter_combo.setObjectName("Filter")
        for text, key in (
            ("全部日志", "all"),
            ("帧 / 数据", "data"),
            ("回显", "echo"),
            ("语音", "voice"),
            ("统计", "stat"),
            ("链路 / 录制", "link"),
            ("警告 / 错误", "error"),
        ):
            self.filter_combo.addItem(text, key)
        self.filter_combo.currentIndexChanged.connect(self._on_filter_changed)
        top.addWidget(self.filter_combo)

        self.search_edit = QLineEdit()
        self.search_edit.setObjectName("Search")
        self.search_edit.setPlaceholderText("搜索关键字…")
        self.search_edit.setClearButtonEnabled(True)
        self.search_edit.setFixedWidth(180)
        self.search_edit.textChanged.connect(self._on_search_changed)
        top.addWidget(self.search_edit)
        layout.addLayout(top)

        tools = QHBoxLayout()
        tools.setSpacing(6)
        self.pause_btn = QToolButton()
        self.pause_btn.setText("暂停")
        self.pause_btn.setCheckable(True)
        self.pause_btn.setToolTip("暂停刷新日志（后台仍继续统计）")
        self.pause_btn.toggled.connect(self.set_paused)
        tools.addWidget(self.pause_btn)

        self.auto_scroll = QCheckBox("自动滚动")
        self.auto_scroll.setChecked(True)
        tools.addWidget(self.auto_scroll)

        self.frames_check = QCheckBox("逐帧")
        self.frames_check.setChecked(True)
        self.frames_check.setToolTip("显示每帧 FRAME / DATA；高频时可关闭")
        tools.addWidget(self.frames_check)

        self.stats_check = QCheckBox("STAT")
        self.stats_check.setChecked(True)
        self.stats_check.setToolTip("每秒输出一行统计")
        tools.addWidget(self.stats_check)

        tools.addStretch(1)

        self.clear_btn = QToolButton()
        self.clear_btn.setText("清空")
        self.clear_btn.clicked.connect(self.clear)
        tools.addWidget(self.clear_btn)

        self.save_btn = QToolButton()
        self.save_btn.setText("导出")
        self.save_btn.clicked.connect(self._save)
        tools.addWidget(self.save_btn)
        layout.addLayout(tools)

        self.edit = QPlainTextEdit()
        self.edit.setObjectName("Log")
        self.edit.setReadOnly(True)
        self.edit.setUndoRedoEnabled(False)
        self.edit.setMaximumBlockCount(self.MAX_ENTRIES)
        self.edit.setFont(_mono_font(10.5))
        self.edit.setLineWrapMode(QPlainTextEdit.LineWrapMode.WidgetWidth)
        self.edit.setWordWrapMode(QTextOption.WrapMode.WrapAtWordBoundaryOrAnywhere)
        self.edit.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.edit.setPlaceholderText("连接设备后，FRAME / DATA / STAT / ECHO 日志会显示在这里…")
        layout.addWidget(self.edit, 1)

        self._search_timer = QTimer(self)
        self._search_timer.setSingleShot(True)
        self._search_timer.setInterval(250)
        self._search_timer.timeout.connect(self.rebuild)

    # -- 数据 -------------------------------------------------------------

    def _matches(self, entry: LogEntry) -> bool:
        if self._filter != "all" and LEVEL_FILTER.get(entry.level, "link") != self._filter:
            return False
        if self._search:
            haystack = f"{entry.ts} {entry.level} {entry.text}".lower()
            if self._search not in haystack:
                return False
        return True

    def append(self, level: str, text: str) -> None:
        entry = LogEntry(_now_ts(), level, text)
        self.entries.append(entry)
        self.count_label.setText(f"{len(self.entries)} 条")
        if self._paused or not self._matches(entry):
            return
        self._insert([entry], scroll=True)

    def append_many(self, items: List[Tuple[str, str]]) -> None:
        if not items:
            return
        entries = [LogEntry(_now_ts(), level, text) for level, text in items]
        for entry in entries:
            self.entries.append(entry)
        self.count_label.setText(f"{len(self.entries)} 条")
        if self._paused:
            return
        visible = [e for e in entries if self._matches(e)]
        if visible:
            self._insert(visible, scroll=True)

    def _insert(self, entries: List[LogEntry], scroll: bool) -> None:
        if not entries:
            return
        edit = self.edit
        edit.setUpdatesEnabled(False)
        try:
            cursor = edit.textCursor()
            cursor.movePosition(QTextCursor.MoveOperation.End)
            for entry in entries:
                label, _color = LEVEL_META.get(entry.level, ("INFO", TEXT))
                cursor.setBlockFormat(
                    self._fmt_block.get(entry.level, self._fmt_block["info"])
                )
                cursor.insertText(entry.ts + "  ", self._fmt_time)
                cursor.insertText(f"{label:<5}  ", self._fmt_level.get(entry.level, self._fmt_text["info"]))
                text = entry.text.replace("\r", " ").replace("\n", " ").replace("\t", "    ")
                cursor.insertText(text + "\n", self._fmt_text.get(entry.level, self._fmt_text["info"]))
            if scroll and self.auto_scroll.isChecked():
                sb = edit.verticalScrollBar()
                sb.setValue(sb.maximum())
        finally:
            edit.setUpdatesEnabled(True)

    def rebuild(self) -> None:
        self.edit.clear()
        visible = [entry for entry in self.entries if self._matches(entry)]
        if visible:
            self._insert(visible, scroll=False)
        self.edit.setPlaceholderText(
            "当前筛选条件下没有日志"
            if self.entries and not visible
            else "连接设备后，FRAME / DATA / STAT / ECHO 日志会显示在这里…"
        )
        self.count_label.setText(f"{len(self.entries)} 条")

    # -- 操作 -------------------------------------------------------------

    def _on_filter_changed(self) -> None:
        self._filter = self.filter_combo.currentData() or "all"
        self.rebuild()

    def _on_search_changed(self, text: str) -> None:
        self._search = text.strip().lower()
        self._search_timer.start()

    def set_paused(self, paused: bool) -> None:
        self._paused = paused
        self.pause_btn.setText("已暂停" if paused else "暂停")
        if not paused:
            self.rebuild()

    def clear(self) -> None:
        self.entries.clear()
        self.edit.clear()
        self.count_label.setText("0 条")

    def _save(self) -> None:
        default = str(Path.cwd() / f"host_test_log_{datetime.now():%Y%m%d_%H%M%S}.txt")
        path, _ = QFileDialog.getSaveFileName(self, "导出日志", default, "Text (*.txt);;All files (*)")
        if not path:
            return
        try:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(f"# DeskPilot host_test log exported {datetime.now():%Y-%m-%d %H:%M:%S}\n")
                for entry in self.entries:
                    label, _color = LEVEL_META.get(entry.level, ("INFO", TEXT))
                    fh.write(f"{entry.ts}  {label:<5}  {entry.text}\n")
        except OSError as exc:
            QMessageBox.warning(self, "导出失败", str(exc))
            return
        self.append("info", f"日志已导出：{path}")


# ---------------------------------------------------------------------------
# 统计卡片
# ---------------------------------------------------------------------------


class StatCard(QFrame):
    def __init__(self, title: str, accent: str, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self.setObjectName("StatCard")
        self.setMinimumHeight(88)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(14, 10, 14, 9)
        layout.setSpacing(2)

        top = QHBoxLayout()
        top.setSpacing(7)
        dot = QLabel()
        dot.setFixedSize(8, 8)
        dot.setStyleSheet(f"background:{accent}; border-radius:4px;")
        top.addWidget(dot)
        title_label = QLabel(title)
        title_label.setObjectName("StatTitle")
        top.addWidget(title_label)
        top.addStretch(1)
        layout.addLayout(top)

        self.value_label = QLabel("—")
        self.value_label.setObjectName("StatValue")
        layout.addWidget(self.value_label)

        self.hint_label = QLabel("")
        self.hint_label.setObjectName("StatHint")
        layout.addWidget(self.hint_label)

    def set_value(self, value: str, hint: str = "", tone: str = "normal") -> None:
        self.value_label.setText(value)
        if self.value_label.property("tone") != tone:
            self.value_label.setProperty("tone", tone)
            _repolish(self.value_label)
        self.hint_label.setText(hint)


class StatusPill(QFrame):
    def __init__(self, parent: Optional[QWidget] = None) -> None:
        super().__init__(parent)
        self.setObjectName("StatusPill")
        self.setFixedHeight(30)
        layout = QHBoxLayout(self)
        layout.setContentsMargins(11, 0, 13, 0)
        layout.setSpacing(7)
        self.dot = QLabel()
        self.dot.setFixedSize(8, 8)
        layout.addWidget(self.dot)
        self.text = QLabel("未连接")
        self.text.setObjectName("StatusText")
        layout.addWidget(self.text)
        self.set_state("idle", "未连接")

    def set_state(self, state: str, text: str) -> None:
        color = {
            "idle": "#94a3b8",
            "connecting": AMBER,
            "connected": GREEN,
            "reconnect": AMBER,
            "error": RED,
        }.get(state, "#94a3b8")
        self.setProperty("state", state)
        _repolish(self)
        self.dot.setStyleSheet(f"background:{color}; border-radius:4px;")
        self.text.setText(text)


# ---------------------------------------------------------------------------
# 后台连接/断开线程
# ---------------------------------------------------------------------------


class ConnectThread(QThread):
    success = Signal(int, object)
    failed = Signal(int, str)

    def __init__(self, transport: Transport, epoch: int, parent: Optional[Any] = None) -> None:
        super().__init__(parent)
        self.transport = transport
        self.epoch = epoch

    def run(self) -> None:
        try:
            self.transport.open()
        except Exception as exc:
            try:
                self.transport.close()
            except Exception:
                pass
            self.failed.emit(self.epoch, str(exc))
            return
        self.success.emit(self.epoch, self.transport)


class CloseThread(QThread):
    done = Signal(int)

    def __init__(self, transport: Transport, epoch: int, parent: Optional[Any] = None) -> None:
        super().__init__(parent)
        self.transport = transport
        self.epoch = epoch

    def run(self) -> None:
        try:
            self.transport.close()
        except Exception:
            pass
        self.done.emit(self.epoch)


# ---------------------------------------------------------------------------
# 主窗口
# ---------------------------------------------------------------------------


class MainWindow(QMainWindow):
    sig_event = Signal(str)
    sig_disconnected = Signal(str)
    sig_voice_state = Signal(str, str)
    sig_voice_result = Signal(object, object, object)
    sig_voice_error = Signal(str)
    sig_voice_log = Signal(str)

    def __init__(self) -> None:
        super().__init__()
        self.setWindowTitle("DeskPilot · SF32LB52 上位机验证台")
        self.setWindowIcon(_make_icon())
        self.resize(1280, 920)
        self.setMinimumSize(1080, 720)

        self.parser = FrameParser(self._on_frame_bg)
        self.checker = Checker(pattern="auto")
        self._parser_baseline = self.parser.stats_snapshot()
        self._err_seen = {
            "crc_err": self._parser_baseline["crc_err"],
            "oversize": self._parser_baseline["oversize"],
            "resync": self._parser_baseline["resync"],
        }
        self.frame_queue: "queue.Queue[Tuple[float, Frame]]" = queue.Queue()

        self.transport: Optional[Transport] = None
        self._connect_thread: Optional[ConnectThread] = None
        self._close_thread: Optional[CloseThread] = None
        self._connect_epoch = 0
        self._state = "idle"
        self._intentional_close = False
        self._closing = False
        self._last_config: Optional[Dict[str, Any]] = None
        self._reconnecting = False

        self.recorder: Optional[Recorder] = None
        self.record_path = ""
        self._record_rows = 0
        self._auto_record_path = True
        self.echo_pending: Dict[int, Tuple[float, bytes]] = {}
        self.echo_seq = 0
        self.echo_ok = 0
        self.echo_fail = 0

        self._audio = VoiceSession()
        self._audio_dirty = False
        self._last_wav = ""
        self._voice_controller: Optional[VoiceController] = None
        self._voice_ready = False
        self._last_voice_wav = ""
        self._player = None
        self._player_timer = QTimer(self)
        self._player_timer.setInterval(10)
        self._player_timer.timeout.connect(self._poll_player)

        self._last_stat_at = time.monotonic()
        self._capture_started_at = time.monotonic()

        self._build_ui()
        self._connect_signals()

        self._drain_timer = QTimer(self)
        self._drain_timer.setInterval(40)
        self._drain_timer.timeout.connect(self._drain_frames)

        self._ui_timer = QTimer(self)
        self._ui_timer.setInterval(200)
        self._ui_timer.timeout.connect(self._tick_ui)
        self._ui_timer.start()

        self._refresh_ports(silent=True)
        QTimer.singleShot(350, lambda: self._refresh_paired(silent=True))

        self.log.append("info", "欢迎使用 DeskPilot 上位机验证台。选择传输方式后点击「连接」。")
        self.log.append("info", "BLE 已配对设备会优先直连，设备被系统 HID 占用、不广播时也能连接。")
        self._set_state("idle", "未连接")
        self._update_cards()

    # -- 构建界面 ---------------------------------------------------------

    def _build_ui(self) -> None:
        central = QWidget()
        central.setObjectName("Root")
        self.setCentralWidget(central)
        root = QVBoxLayout(central)
        root.setContentsMargins(22, 18, 22, 12)
        root.setSpacing(14)

        root.addWidget(self._build_header())

        body = QHBoxLayout()
        body.setSpacing(14)
        body.addWidget(self._build_sidebar(), 0)
        body.addLayout(self._build_right(), 1)
        root.addLayout(body, 1)

        status = QStatusBar()
        status.setSizeGripEnabled(False)
        self.status = status
        self.setStatusBar(status)
        self.parser_status = QLabel("解析器  frames=0  bytes=0  crc_err=0  resync=0")
        self.parser_status.setObjectName("Muted")
        status.addWidget(self.parser_status)
        self.record_status = QLabel("未录制")
        self.record_status.setObjectName("Muted")
        status.addPermanentWidget(self.record_status)
        self.voice_bar = QLabel("语音：未启用")
        self.voice_bar.setObjectName("Muted")
        status.addPermanentWidget(self.voice_bar)
        version = QLabel(f"v{__version__}")
        version.setObjectName("Muted")
        status.addPermanentWidget(version)

    def _build_header(self) -> QWidget:
        header = QWidget()
        layout = QHBoxLayout(header)
        layout.setContentsMargins(2, 0, 2, 0)
        layout.setSpacing(13)

        icon = QLabel("KB")
        icon.setObjectName("AppIcon")
        icon.setFixedSize(46, 46)
        icon.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(icon)

        titles = QVBoxLayout()
        titles.setSpacing(1)
        title = QLabel("DeskPilot 上位机验证台")
        title.setObjectName("AppTitle")
        subtitle = QLabel("SF32LB52 数字键盘 · USB CDC / BLE 私有 GATT · A5 5A 帧解析")
        subtitle.setObjectName("AppSubtitle")
        titles.addWidget(title)
        titles.addWidget(subtitle)
        layout.addLayout(titles)
        layout.addStretch(1)

        info = QVBoxLayout()
        info.setSpacing(2)
        info.setAlignment(Qt.AlignmentFlag.AlignRight)
        self.link_info = QLabel("尚未连接")
        self.link_info.setObjectName("LinkInfo")
        self.link_info.setAlignment(Qt.AlignmentFlag.AlignRight)
        self.mtu_info = QLabel("MTU — · 单帧 payload —")
        self.mtu_info.setObjectName("MtuInfo")
        self.mtu_info.setAlignment(Qt.AlignmentFlag.AlignRight)
        info.addWidget(self.link_info)
        info.addWidget(self.mtu_info)
        layout.addLayout(info)

        self.status_pill = StatusPill()
        layout.addWidget(self.status_pill)
        return header

    def _build_sidebar(self) -> QWidget:
        card = QFrame()
        card.setObjectName("Card")
        card.setFixedWidth(352)
        card_layout = QVBoxLayout(card)
        card_layout.setContentsMargins(0, 0, 0, 0)
        card_layout.setSpacing(0)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        content = QWidget()
        content.setObjectName("SidebarContent")
        layout = QVBoxLayout(content)
        layout.setContentsMargins(17, 15, 17, 15)
        layout.setSpacing(9)

        heading = QLabel("连接设置")
        heading.setObjectName("CardHeading")
        layout.addWidget(heading)

        self.transport_combo = QComboBox()
        self.transport_combo.addItem("BLE GATT · 无线", "ble")
        self.transport_combo.addItem("USB CDC · 有线", "usb")
        self.transport_combo.addItem("模拟设备 · SIM", "sim")
        layout.addWidget(self.transport_combo)

        self.settings_stack = QStackedWidget()
        self.settings_stack.addWidget(self._build_ble_page())
        self.settings_stack.addWidget(self._build_usb_page())
        self.settings_stack.addWidget(self._build_sim_page())
        layout.addWidget(self.settings_stack)
        self.transport_combo.currentIndexChanged.connect(self._on_transport_changed)

        self.auto_reconnect = QCheckBox("断线后自动重连")
        self.auto_reconnect.setChecked(True)
        layout.addWidget(self.auto_reconnect)

        buttons = QHBoxLayout()
        buttons.setSpacing(8)
        self.connect_btn = QPushButton("连接")
        self.connect_btn.setObjectName("Primary")
        self.connect_btn.clicked.connect(self._on_connect_clicked)
        buttons.addWidget(self.connect_btn, 1)
        self.disconnect_btn = QPushButton("断开")
        self.disconnect_btn.setObjectName("DangerGhost")
        self.disconnect_btn.setEnabled(False)
        self.disconnect_btn.clicked.connect(self._on_disconnect_clicked)
        buttons.addWidget(self.disconnect_btn, 1)
        layout.addLayout(buttons)

        layout.addWidget(self._divider())
        echo_heading = QLabel("回环测试")
        echo_heading.setObjectName("SectionHeading")
        layout.addWidget(echo_heading)
        echo_row = QHBoxLayout()
        echo_row.setSpacing(7)
        self.echo_edit = QLineEdit()
        self.echo_edit.setPlaceholderText("输入文本，例如 hello 键盘")
        self.echo_edit.returnPressed.connect(self._send_echo)
        echo_row.addWidget(self.echo_edit, 1)
        self.echo_btn = QPushButton("发送")
        self.echo_btn.setObjectName("Ghost")
        self.echo_btn.setEnabled(False)
        self.echo_btn.clicked.connect(self._send_echo)
        echo_row.addWidget(self.echo_btn)
        layout.addLayout(echo_row)
        echo_hint = QLabel("设备需先执行 test echo on")
        echo_hint.setObjectName("Hint")
        layout.addWidget(echo_hint)

        layout.addWidget(self._divider())
        rec_heading = QLabel("数据记录")
        rec_heading.setObjectName("SectionHeading")
        layout.addWidget(rec_heading)
        path_row = QHBoxLayout()
        path_row.setSpacing(6)
        self.record_path_edit = QLineEdit()
        self.record_path_edit.setPlaceholderText("CSV 保存路径")
        self.record_path_edit.setText(self._default_record_path())
        self.record_path_edit.setCursorPosition(0)
        self.record_path_edit.setToolTip(self.record_path_edit.text())
        self.record_path_edit.textEdited.connect(self._on_record_path_edited)
        path_row.addWidget(self.record_path_edit, 1)
        browse = QToolButton()
        browse.setText("…")
        browse.setToolTip("选择 CSV 保存位置")
        browse.clicked.connect(self._browse_record_path)
        path_row.addWidget(browse)
        layout.addLayout(path_row)

        rec_row = QHBoxLayout()
        rec_row.setSpacing(8)
        self.record_btn = QPushButton("开始录制")
        self.record_btn.setObjectName("RecordButton")
        self.record_btn.setCheckable(True)
        self.record_btn.setEnabled(False)
        self.record_btn.toggled.connect(self._on_record_toggled)
        rec_row.addWidget(self.record_btn, 1)
        self.compare_btn = QPushButton("比较 CSV")
        self.compare_btn.setObjectName("Ghost")
        self.compare_btn.clicked.connect(self._compare_dialogs)
        rec_row.addWidget(self.compare_btn)
        layout.addLayout(rec_row)

        layout.addWidget(self._divider())
        audio_heading = QLabel("音频接收 (AUDIO_DATA)")
        audio_heading.setObjectName("SectionHeading")
        layout.addWidget(audio_heading)

        self.audio_btn = QPushButton("开始接收音频")
        self.audio_btn.setObjectName("RecordButton")
        self.audio_btn.setCheckable(True)
        self.audio_btn.setEnabled(False)
        self.audio_btn.setToolTip(
            "接收设备端 audio send / audio stream on 发出的 ADPCM 块"
        )
        self.audio_btn.toggled.connect(self._on_audio_toggled)
        layout.addWidget(self.audio_btn)

        self.audio_status = QLabel("未接收")
        self.audio_status.setWordWrap(True)
        layout.addWidget(self.audio_status)

        self.audio_save_btn = QPushButton("保存 WAV…")
        self.audio_save_btn.setObjectName("Ghost")
        self.audio_save_btn.setEnabled(False)
        self.audio_save_btn.clicked.connect(self._save_audio_wav)
        layout.addWidget(self.audio_save_btn)

        reset_btn = QPushButton("清零统计")
        reset_btn.setObjectName("Ghost")
        reset_btn.clicked.connect(self._reset_stats)
        layout.addWidget(reset_btn)

        layout.addStretch(1)
        scroll.setWidget(content)
        card_layout.addWidget(scroll)
        return card

    def _build_usb_page(self) -> QWidget:
        page = QWidget()
        form = QFormLayout(page)
        form.setContentsMargins(0, 2, 0, 2)
        form.setHorizontalSpacing(8)
        form.setVerticalSpacing(7)
        form.setLabelAlignment(Qt.AlignmentFlag.AlignLeft)

        port_row = QHBoxLayout()
        port_row.setSpacing(6)
        self.port_combo = QComboBox()
        self.port_combo.setEditable(True)
        self.port_combo.setMinimumContentsLength(18)
        port_row.addWidget(self.port_combo, 1)
        refresh = QToolButton()
        refresh.setText("↻")
        refresh.setToolTip("刷新串口列表")
        refresh.clicked.connect(lambda: self._refresh_ports(silent=False))
        port_row.addWidget(refresh)
        form.addRow(self._form_label("串口"), port_row)
        hint = QLabel("CDC 不依赖波特率；找不到时可直接输入 COMx")
        hint.setObjectName("Hint")
        form.addRow("", hint)
        return page

    def _build_ble_page(self) -> QWidget:
        page = QWidget()
        form = QFormLayout(page)
        form.setContentsMargins(0, 2, 0, 2)
        form.setHorizontalSpacing(8)
        form.setVerticalSpacing(7)

        self.ble_name = QLineEdit("SF32")
        self.ble_name.setPlaceholderText("设备名子串，例如 SF32")
        form.addRow(self._form_label("设备名"), self.ble_name)

        self.ble_address = QLineEdit()
        self.ble_address.setPlaceholderText("可选，如 5C:FD:52:80:0C:3C")
        form.addRow(self._form_label("地址"), self.ble_address)

        paired_row = QHBoxLayout()
        paired_row.setSpacing(6)
        self.paired_combo = QComboBox()
        self.paired_combo.addItem("自动查找（推荐）", None)
        self.paired_combo.currentIndexChanged.connect(self._on_paired_selected)
        paired_row.addWidget(self.paired_combo, 1)
        paired_refresh = QToolButton()
        paired_refresh.setText("↻")
        paired_refresh.setToolTip("刷新 Windows 已配对设备")
        paired_refresh.clicked.connect(lambda: self._refresh_paired(silent=False))
        paired_row.addWidget(paired_refresh)
        form.addRow(self._form_label("配对设备"), paired_row)

        self.ble_paired_only = QCheckBox("仅用配对记录（不扫描）")
        self.ble_paired_only.setToolTip("设备被系统 HID 占用、不广播时使用")
        form.addRow("", self.ble_paired_only)

        hint = QLabel("已配对设备优先直连；无需忘记设备")
        hint.setObjectName("Hint")
        form.addRow("", hint)
        return page

    def _build_sim_page(self) -> QWidget:
        page = QWidget()
        form = QFormLayout(page)
        form.setContentsMargins(0, 2, 0, 2)
        form.setHorizontalSpacing(8)
        form.setVerticalSpacing(7)

        self.sim_rate = QSpinBox()
        self.sim_rate.setRange(1, 1000)
        self.sim_rate.setValue(20)
        self.sim_rate.setSuffix(" Hz")
        form.addRow(self._form_label("速率"), self.sim_rate)

        self.sim_size = QSpinBox()
        self.sim_size.setRange(1, 1000)
        self.sim_size.setValue(64)
        self.sim_size.setSuffix(" B")
        form.addRow(self._form_label("数据长度"), self.sim_size)

        self.sim_pattern = QComboBox()
        for text, key in (("INC 递增", "inc"), ("ASCII 文本", "ascii"), ("ZERO 全零", "zero")):
            self.sim_pattern.addItem(text, key)
        form.addRow(self._form_label("模式"), self.sim_pattern)

        self.sim_anomaly = QComboBox()
        for text, key in (
            ("无", "none"),
            ("拆包", "split"),
            ("粘包", "sticky"),
            ("脏字节", "dirty"),
            ("CRC 错", "badcrc"),
            ("全部异常", "all"),
        ):
            self.sim_anomaly.addItem(text, key)
        form.addRow(self._form_label("异常注入"), self.sim_anomaly)

        self.sim_burst = QSpinBox()
        self.sim_burst.setRange(0, 1000000)
        self.sim_burst.setSpecialValueText("持续发送")
        self.sim_burst.setValue(0)
        form.addRow(self._form_label("帧数"), self.sim_burst)
        return page

    def _build_right(self) -> QVBoxLayout:
        layout = QVBoxLayout()
        layout.setSpacing(12)

        layout.addWidget(self._build_voice_card())

        stats = QGridLayout()
        stats.setHorizontalSpacing(12)
        stats.setVerticalSpacing(12)
        specs = (
            ("frames", "已收帧数", PRIMARY),
            ("rate", "接收速率", TEAL),
            ("lost", "丢失帧", AMBER),
            ("dup", "重复帧", VIOLET),
            ("order", "乱序帧", INDIGO),
            ("crc", "CRC 错误", RED),
            ("bytes", "有效数据", GREEN),
            ("jitter", "到达抖动", "#0891b2"),
        )
        self.cards: Dict[str, StatCard] = {}
        for index, (key, title, color) in enumerate(specs):
            card = StatCard(title, color)
            self.cards[key] = card
            stats.addWidget(card, index // 4, index % 4)
        for col in range(4):
            stats.setColumnStretch(col, 1)
        layout.addLayout(stats)

        self.log = LogView()
        self.log_frames_check = self.log.frames_check
        self.log_stats_check = self.log.stats_check
        layout.addWidget(self.log, 1)
        return layout

    def _build_voice_card(self) -> QWidget:
        card = QFrame()
        card.setObjectName("Card")
        card_layout = QVBoxLayout(card)
        card_layout.setContentsMargins(15, 11, 15, 11)
        card_layout.setSpacing(8)

        top = QHBoxLayout()
        top.setSpacing(10)
        title = QLabel("语音输入")
        title.setObjectName("CardHeading")
        top.addWidget(title)
        self.voice_enable = QCheckBox("启用（松手识别 → 注入）")
        self.voice_enable.setToolTip("启用后按住 KEY1 说话，松开自动识别并注入焦点窗口")
        self.voice_enable.toggled.connect(self._on_voice_toggled)
        top.addWidget(self.voice_enable)
        self.voice_status = QLabel("未启用")
        self.voice_status.setObjectName("VoiceStatus")
        self.voice_status.setWordWrap(False)
        top.addWidget(self.voice_status, 1)
        card_layout.addLayout(top)

        controls = QHBoxLayout()
        controls.setSpacing(7)
        controls.addWidget(self._form_label("模型"))
        self.voice_model_edit = QLineEdit(
            os.environ.get("HOST_TEST_ASR_MODEL", "models/faster-whisper-small")
        )
        self.voice_model_edit.setPlaceholderText("本地模型目录 / 模型名")
        self.voice_model_edit.setToolTip("本地 faster-whisper 模型目录；找不到则按模型名下载")
        controls.addWidget(self.voice_model_edit, 1)
        model_browse = QToolButton()
        model_browse.setText("…")
        model_browse.setToolTip("选择本地模型目录")
        model_browse.clicked.connect(self._browse_voice_model)
        controls.addWidget(model_browse)
        controls.addSpacing(6)
        controls.addWidget(self._form_label("注入"))
        self.voice_mode = QComboBox()
        for text, key in (
            ("自动（短直打 / 长粘贴）", "auto"),
            ("SendInput 直打", "sendinput"),
            ("剪贴板 + Ctrl+V", "clipboard"),
            ("只识别，不注入", "off"),
        ):
            self.voice_mode.addItem(text, key)
        self.voice_mode.setMinimumWidth(210)
        self.voice_mode.currentIndexChanged.connect(self._on_voice_mode_changed)
        controls.addWidget(self.voice_mode)
        card_layout.addLayout(controls)

        self.voice_result = QLabel("识别结果会显示在这里；模型在后台加载，不阻塞界面")
        self.voice_result.setObjectName("VoiceResult")
        self.voice_result.setWordWrap(True)
        self.voice_result.setMaximumHeight(46)
        self.voice_result.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        card_layout.addWidget(self.voice_result)
        return card

    @staticmethod
    def _form_label(text: str) -> QLabel:
        label = QLabel(text)
        label.setObjectName("FormLabel")
        return label

    @staticmethod
    def _divider() -> QFrame:
        divider = QFrame()
        divider.setObjectName("Divider")
        divider.setFrameShape(QFrame.Shape.NoFrame)
        return divider

    # -- 信号与状态 -------------------------------------------------------

    def _connect_signals(self) -> None:
        self.sig_event.connect(self._on_transport_event, Qt.ConnectionType.QueuedConnection)
        self.sig_disconnected.connect(self._on_disconnected, Qt.ConnectionType.QueuedConnection)
        self.sig_voice_state.connect(self._on_voice_state, Qt.ConnectionType.QueuedConnection)
        self.sig_voice_result.connect(self._on_voice_result, Qt.ConnectionType.QueuedConnection)
        self.sig_voice_error.connect(self._on_voice_error, Qt.ConnectionType.QueuedConnection)
        self.sig_voice_log.connect(self._on_voice_log, Qt.ConnectionType.QueuedConnection)

    def _on_transport_changed(self, _index: int) -> None:
        if hasattr(self, "settings_stack"):
            self.settings_stack.setCurrentIndex(self.transport_combo.currentIndex())

    def _set_state(self, state: str, text: str) -> None:
        self._state = state
        self.status_pill.set_state(state, text)
        active = state in ("connecting", "connected", "reconnect")
        self.connect_btn.setEnabled(state in ("idle", "error"))
        self.connect_btn.setText(
            {"connecting": "连接中…", "connected": "已连接", "reconnect": "重连中…"}.get(state, "连接")
        )
        self.disconnect_btn.setEnabled(active)
        self.settings_stack.setEnabled(not active)
        self.transport_combo.setEnabled(not active)
        self.auto_reconnect.setEnabled(not active)
        connected = state == "connected"
        self.echo_btn.setEnabled(connected)
        self.record_btn.setEnabled(connected or self.recorder is not None)
        self.audio_btn.setEnabled(connected)
        if not connected and self.audio_btn.isChecked():
            # 断开时收尾：保留已收到的数据，允许继续保存 WAV
            self.audio_btn.blockSignals(True)
            self.audio_btn.setChecked(False)
            self.audio_btn.blockSignals(False)
            self.audio_btn.setText("开始接收音频")
            self._refresh_audio_status(final=True)
        if state == "idle":
            self.link_info.setText("尚未连接")
            self.mtu_info.setText("MTU — · 单帧 payload —")
        elif state == "error":
            self.link_info.setText("连接失败")
            self.mtu_info.setText("请检查参数或查看日志")
        elif state == "reconnect":
            self.link_info.setText("链路中断 · 正在重连")
            self.mtu_info.setText("等待设备恢复…")

    def _update_link_info(self) -> None:
        transport = self.transport
        if transport is None or not transport.connected:
            return
        config = self._last_config or {}
        target = ""
        if config.get("kind") == "usb":
            target = config.get("port", "")
        elif config.get("kind") == "ble":
            target = config.get("address") or config.get("name") or ""
        elif config.get("kind") == "sim":
            target = "内置测试源"
        self.link_info.setText(f"{transport.name} · {target}".rstrip(" ·"))
        self.mtu_info.setText(
            f"MTU {transport.mtu} · 单帧 payload ≤ {transport.max_payload} B"
        )

    # -- 端口 / 配对设备 --------------------------------------------------

    def _refresh_ports(self, silent: bool = False) -> None:
        previous = self.port_combo.currentData() if hasattr(self, "port_combo") else None
        try:
            ports = list_serial_ports()
        except TransportError as exc:
            self.log.append("warn", f"串口枚举失败：{exc}")
            return
        self.port_combo.blockSignals(True)
        self.port_combo.clear()
        for port in ports:
            label = port.device
            if port.description:
                label += f" · {port.description}"
            self.port_combo.addItem(label, port.device)
        if not ports:
            self.port_combo.addItem("未找到串口，请点击刷新", None)
        else:
            index = 0
            if previous:
                for i in range(self.port_combo.count()):
                    if self.port_combo.itemData(i) == previous:
                        index = i
                        break
            self.port_combo.setCurrentIndex(index)
        self.port_combo.blockSignals(False)
        if not silent:
            self.log.append("info", f"发现 {len(ports)} 个串口")

    def _refresh_paired(self, silent: bool = False) -> None:
        try:
            devices = list_paired_ble_devices()
        except TransportError as exc:
            if not silent:
                self.log.append("warn", f"读取已配对 BLE 设备失败：{exc}")
            return
        self.paired_combo.blockSignals(True)
        self.paired_combo.clear()
        self.paired_combo.addItem("自动查找（推荐）", None)
        for device in devices:
            state = "已连接" if device.get("connected") else "已配对"
            label = f"{device['name'] or '?'} · {device['address']} · {state}"
            self.paired_combo.addItem(label, device)
        self.paired_combo.blockSignals(False)
        if not silent:
            self.log.append("info", f"Windows 已配对 BLE 设备：{len(devices)} 台")
            for device in devices:
                state = "connected" if device.get("connected") else "paired"
                self.log.append(
                    "link",
                    f"PAIRED {device['address']}  {device['name'] or '?'}  "
                    f"type={device['address_type']}  {state}",
                )

    def _on_paired_selected(self, _index: int) -> None:
        device = self.paired_combo.currentData()
        if not device:
            return
        self.ble_name.setText(device.get("name") or "")
        self.ble_address.setText(device.get("address") or "")

    # -- 连接 -------------------------------------------------------------

    def _collect_config(self) -> Optional[Dict[str, Any]]:
        kind = self.transport_combo.currentData()
        if kind == "usb":
            port = self.port_combo.currentData()
            if not port:
                text = self.port_combo.currentText().strip()
                if text and "未找到" not in text:
                    port = text.split(" · ")[0].strip()
            if not port:
                self.log.append("error", "未选择串口；请刷新或手动输入 COMx")
                return None
            return {"kind": "usb", "port": port}
        if kind == "ble":
            name = self.ble_name.text().strip() or None
            address = self.ble_address.text().strip() or None
            paired_only = self.ble_paired_only.isChecked()
            if paired_only and not (name or address):
                self.log.append("error", "“仅用配对记录”需要填写设备名或地址")
                return None
            return {
                "kind": "ble",
                "name": name,
                "address": address,
                "paired": paired_only,
            }
        return {
            "kind": "sim",
            "rate": self.sim_rate.value(),
            "size": self.sim_size.value(),
            "pattern": self.sim_pattern.currentData(),
            "anomaly": self.sim_anomaly.currentData(),
            "burst": self.sim_burst.value(),
        }

    def _create_transport(self, config: Dict[str, Any]) -> Transport:
        kind = config["kind"]
        if kind == "usb":
            return SerialTransport(port=config["port"])
        if kind == "ble":
            return BleTransport(
                name_filter=config.get("name"),
                address=config.get("address"),
                paired=bool(config.get("paired")),
                scan_timeout=6.0,
                connect_timeout=30.0,
            )
        return SimTransport(
            rate_hz=config["rate"],
            size=config["size"],
            pattern=config["pattern"],
            anomaly=config["anomaly"],
            burst=config["burst"],
        )

    def _on_connect_clicked(self) -> None:
        if self._state in ("connecting", "connected", "reconnect"):
            return
        config = self._collect_config()
        if config is None:
            return
        self._begin_connect(config, reconnect=False)

    def _begin_connect(self, config: Dict[str, Any], reconnect: bool) -> None:
        if self._closing:
            return
        self._last_config = dict(config)
        self._reconnecting = reconnect
        self._intentional_close = False
        self.parser.reset()
        self._clear_queue()
        if not reconnect:
            self._reset_stats(silent=True)
        self._connect_epoch += 1
        epoch = self._connect_epoch

        try:
            transport = self._create_transport(config)
        except Exception as exc:
            self.log.append("error", f"创建传输失败：{exc}")
            self._set_state("error", "连接失败")
            return

        transport.set_rx(self.parser.feed)
        transport.set_event(self.sig_event.emit)
        transport.set_disconnect(lambda reason: self.sig_disconnected.emit(reason))
        self.transport = transport

        target = config.get("port") or config.get("address") or config.get("name") or "内置测试源"
        self._set_state("reconnect" if reconnect else "connecting", "重连中" if reconnect else "连接中")
        self.log.append(
            "link",
            f"正在连接 {transport.name} · {target}" if not reconnect else f"正在重连 {transport.name} · {target}",
        )
        self.connect_btn.setText("连接中…")

        thread = ConnectThread(transport, epoch, self)
        thread.success.connect(self._on_connected)
        thread.failed.connect(self._on_connect_failed)
        thread.finished.connect(self._on_connect_thread_finished)
        self._connect_thread = thread
        thread.start()

    def _on_connect_thread_finished(self) -> None:
        thread = self._connect_thread
        if thread is not None and not thread.isRunning():
            self._connect_thread = None
        thread = self._close_thread
        if thread is not None and not thread.isRunning():
            self._close_thread = None

    def _on_connected(self, epoch: int, transport_obj: object) -> None:
        transport = transport_obj
        if not isinstance(transport, Transport):
            return
        if epoch != self._connect_epoch or self._closing:
            try:
                transport.close()
            except Exception:
                pass
            return
        self.transport = transport
        self._intentional_close = False
        self._reconnecting = False
        self._capture_started_at = time.monotonic()
        self._set_state("connected", "已连接")
        self._update_link_info()
        self._drain_timer.start()
        self._last_stat_at = time.monotonic()
        ps = self.parser.stats_snapshot()
        self._err_seen = {
            "crc_err": ps["crc_err"],
            "oversize": ps["oversize"],
            "resync": ps["resync"],
        }
        self._update_cards()
        self.log.append(
            "link",
            f"已连接 {transport.name} · MTU {transport.mtu} · 单帧 payload ≤ {transport.max_payload} B",
        )
        if self.record_btn.isChecked() and self.recorder is None:
            self._start_recording()

    def _on_connect_failed(self, epoch: int, message: str) -> None:
        if epoch != self._connect_epoch or self._closing:
            return
        self.transport = None
        self._drain_timer.stop()
        self._update_link_info()
        if self._reconnecting and self.auto_reconnect.isChecked() and self._last_config:
            self._set_state("reconnect", "重连中")
            self.log.append("warn", f"重连失败：{message}；2 秒后重试…")
            QTimer.singleShot(2000, self._retry_reconnect)
            return
        self._set_state("error", "连接失败")
        self.log.append("error", f"连接失败：{message}")

    def _retry_reconnect(self) -> None:
        if self._state != "reconnect" or self._closing:
            return
        if self._last_config:
            self._begin_connect(self._last_config, reconnect=True)

    def _on_disconnect_clicked(self) -> None:
        if self._state in ("idle", "error"):
            return
        self._intentional_close = True
        self._connect_epoch += 1
        self._drain_timer.stop()
        self._stop_recording()
        transport, self.transport = self.transport, None
        self._set_state("connecting", "断开中…")
        self.log.append("link", "正在断开连接…")
        if transport is None:
            self._finish_disconnect()
            return
        self._close_thread = CloseThread(transport, self._connect_epoch, self)
        self._close_thread.done.connect(self._on_close_done)
        self._close_thread.finished.connect(self._on_connect_thread_finished)
        self._close_thread.start()

    def _on_close_done(self, _epoch: int) -> None:
        self._finish_disconnect()

    def _finish_disconnect(self) -> None:
        self._discard_open_audio("链路断开")
        self._intentional_close = False
        self._reconnecting = False
        self._set_state("idle", "未连接")
        self._drain_timer.stop()
        self._stop_recording()
        self._clear_queue()
        self.status_pill.set_state("idle", "未连接")
        self.record_status.setText("未录制")
        self.log.append("link", "已断开")
        self._update_cards()

    def _on_disconnected(self, reason: str) -> None:
        if self._closing or self._intentional_close:
            return
        self._discard_open_audio(reason)
        if self.transport is not None and self.transport.auto_reconnect:
            if self.auto_reconnect.isChecked():
                self._set_state("reconnect", "重连中")
                self.log.append("warn", f"链路断开：{reason}；等待 BLE 自动重连…")
            else:
                self.log.append("warn", f"链路断开：{reason}")
                self._set_state("idle", "未连接")
                self._drain_timer.stop()
                self._stop_recording()
                transport, self.transport = self.transport, None
                if transport is not None:
                    try:
                        transport.close()
                    except Exception:
                        pass
            return
        if self.auto_reconnect.isChecked() and self._last_config:
            self._set_state("reconnect", "重连中")
            self.log.append("warn", f"链路断开：{reason}；2 秒后重连…")
            self._drain_timer.stop()
            self._stop_recording()
            QTimer.singleShot(1500, self._reconnect_serial)
            return
        self.log.append("warn", f"链路断开：{reason}")
        self._set_state("idle", "未连接")
        self._drain_timer.stop()
        self._stop_recording()
        self._update_cards()

    def _reconnect_serial(self) -> None:
        if self._state != "reconnect" or self._closing or not self._last_config:
            return
        old, self.transport = self.transport, None
        if old is not None:
            try:
                old.close()
            except Exception:
                pass
        self._begin_connect(self._last_config, reconnect=True)

    def _on_transport_event(self, text: str) -> None:
        if self._closing:
            return
        low = text.lower()
        level = "link"
        if "error" in low or "失败" in text:
            level = "error"
        elif "warn" in low or "断开" in text or "no device" in low:
            level = "warn"
        self.log.append(level, text)
        if text.startswith("LINK  connected"):
            if self._state == "reconnect":
                self._set_state("connected", "已连接")
                self.log.append("info", "BLE 已自动重连")
            self._update_link_info()

    # -- 数据流 -----------------------------------------------------------

    def _on_frame_bg(self, frame: Frame) -> None:
        self.frame_queue.put((time.monotonic(), frame))

    def _clear_queue(self) -> None:
        while True:
            try:
                self.frame_queue.get_nowait()
            except queue.Empty:
                return

    def _drain_frames(self) -> None:
        frames: List[Tuple[float, Frame]] = []
        for _ in range(800):
            try:
                frames.append(self.frame_queue.get_nowait())
            except queue.Empty:
                break
        if not frames:
            return

        log_frames = self.log_frames_check.isChecked()
        log_limit = 200
        frame_entries: List[Tuple[str, str]] = []
        audio_seen = False
        audio_ctl = False
        audio_active = self.audio_btn.isChecked() or self.voice_enable.isChecked()
        for index, (arrival, frame) in enumerate(frames):
            if frame.type == TYPE_AUDIO_CTL:
                if audio_active and len(frame.payload) >= 3:
                    event = frame.payload[0]
                    seg_id = int.from_bytes(frame.payload[1:3], "little")
                    finished = self._audio.on_ctl(event, seg_id)
                    audio_ctl = True
                    frame_entries.append(
                        ("info",
                         f"设备{'开始说话' if event == 0 else '停止说话'} seg={seg_id}")
                    )
                    if finished is not None:
                        self._handle_finished_segment(finished)
                continue
            if frame.type == TYPE_AUDIO_DATA:
                if audio_active:
                    pcm = self._audio.feed(frame)
                    if pcm and self._player is not None:
                        self._player.write(pcm)
                    audio_seen = True
                continue
            info = self.checker.on_frame(frame, arrival)
            if self.recorder is not None:
                self.recorder.write(info, (arrival - self._capture_started_at) * 1000.0)
                self._record_rows += 1
            if log_frames and (len(frames) <= log_limit or index < 60):
                frame_entries.append(
                    (
                        "frame",
                        f"seq={frame.seq} len={len(frame.payload)} "
                        f"type=0x{frame.type:02X} flags=0x{frame.flags:02X} crc=OK",
                    )
                )
                frame_entries.append(
                    (
                        "data",
                        f"sha={info.sha256[:8]}…  {_short_data(info.data, max_hex=16, max_text=64)}",
                    )
                )
            if info.is_echo:
                pending = self.echo_pending.pop(info.seq, None)
                if pending is None:
                    frame_entries.append(("warn", f"seq={info.seq} 未匹配的回显帧"))
                else:
                    _sent_at, expected = pending
                    if info.data == expected:
                        self.echo_ok += 1
                        frame_entries.append(
                            ("echo", f"回显校验 OK seq={info.seq} ({len(info.data)}B)")
                        )
                    else:
                        self.echo_fail += 1
                        frame_entries.append(
                            (
                                "error",
                                f"回显内容不一致 seq={info.seq} "
                                f"收到 {len(info.data)}B / 期望 {len(expected)}B",
                            )
                        )
        if len(frames) > log_limit and log_frames:
            frame_entries.append(
                (
                    "warn",
                    f"日志限流：本批 {len(frames)} 帧，仅显示前 60 帧（统计不受影响）",
                )
            )
        if frame_entries:
            self.log.append_many(frame_entries)
        if audio_seen or audio_ctl:
            self._refresh_audio_status()
        if self.recorder is not None:
            self.record_status.setText(f"录制中 · {self._record_rows} 行")
        self._check_echo_timeouts()
        if len(frames) >= 20:
            self._update_cards()

    def _check_echo_timeouts(self) -> None:
        now = time.monotonic()
        expired = [
            seq for seq, (sent_at, _data) in self.echo_pending.items()
            if now - sent_at > 8.0
        ]
        for seq in expired:
            self.echo_pending.pop(seq, None)
            self.log.append("warn", f"回显超时 seq={seq}（设备可能未执行 test echo on）")

    def _send_echo(self) -> None:
        if self._state != "connected" or self.transport is None:
            self.log.append("warn", "尚未连接，无法发送回环数据")
            return
        text = self.echo_edit.text()
        if not text:
            self.log.append("warn", "回环内容为空")
            return
        data = text.encode("utf-8")
        seq = self.echo_seq
        payload = build_test_payload(int(time.monotonic() * 1000) & 0xFFFFFFFF, data, 0)
        max_payload = min(self.transport.max_payload, 1024)
        if len(payload) > max_payload:
            self.log.append("error", f"回环数据 {len(data)} B 超过链路上限 {max_payload} B")
            return
        frame = build_frame(TYPE_TEST_DATA, 0, seq, payload)
        if self.transport.name.startswith("BLE") and len(frame) > self.transport.mtu - 3:
            self.log.append("error", f"回环帧超过 ATT MTU-3={self.transport.mtu - 3} B")
            return
        try:
            self.transport.write(frame)
        except TransportError as exc:
            self.log.append("error", f"发送回环失败：{exc}")
            return
        self.echo_seq = (seq + 1) & 0xFFFF
        self.echo_pending[seq] = (time.monotonic(), data)
        self.log.append("echo", f"已发送 seq={seq} · {len(data)}B · 等待设备回显")

    # -- 统计 -------------------------------------------------------------

    def _reset_stats(self, silent: bool = False) -> None:
        self.checker = Checker(pattern="auto")
        self._parser_baseline = self.parser.stats_snapshot()
        self._err_seen = {
            "crc_err": self._parser_baseline["crc_err"],
            "oversize": self._parser_baseline["oversize"],
            "resync": self._parser_baseline["resync"],
        }
        self.echo_pending.clear()
        self.echo_ok = 0
        self.echo_fail = 0
        self.echo_seq = 0
        self._update_cards()
        if not silent:
            self.log.append("info", "统计已清零")

    def _log_link_errors(self) -> None:
        ps = self.parser.stats_snapshot()
        for key, label, level in (
            ("crc_err", "CRC 错误", "error"),
            ("oversize", "超长帧", "warn"),
            ("resync", "重同步字节", "warn"),
        ):
            current = ps[key]
            last = self._err_seen.get(key, current)
            if current > last:
                delta = current - last
                self._err_seen[key] = current
                self.log.append(level, f"{label} +{delta}（累计 {current}）")

    def _tick_ui(self) -> None:
        self._update_cards()
        self._update_parser_status()
        self._log_link_errors()
        if self._state == "connected" and self.transport is not None:
            if not self.transport.connected:
                self._on_disconnected("链路已断开")
            elif self.transport.auto_reconnect:
                self._update_link_info()
        elif self._state == "reconnect" and self.transport is not None and self.transport.connected:
            self._set_state("connected", "已连接")
            self._update_link_info()
            self.log.append("info", "链路已恢复")
        if (
            self._state == "connected"
            and self.log_stats_check.isChecked()
            and time.monotonic() - self._last_stat_at >= 1.0
        ):
            self._last_stat_at = time.monotonic()
            ps = self.parser.stats_snapshot()
            crc = ps["crc_err"] - self._parser_baseline["crc_err"]
            oversize = ps["oversize"] - self._parser_baseline["oversize"]
            self.log.append(
                "stat",
                f"rx={self.checker.recv} lost={self.checker.lost} dup={self.checker.dup} "
                f"crc_err={crc} oversize={oversize}",
            )

    def _update_cards(self) -> None:
        checker = self.checker
        ps = self.parser.stats_snapshot()
        crc = max(0, ps["crc_err"] - self._parser_baseline["crc_err"])
        oversize = max(0, ps["oversize"] - self._parser_baseline["oversize"])

        self.cards["frames"].set_value(
            _fmt_int(checker.recv),
            f"parser {_fmt_int(ps['frames'])} · unknown {ps['unknown_type']}",
        )
        self.cards["rate"].set_value(
            f"{checker.rate_fps:.1f} fps",
            f"duration {checker.duration_s:.1f}s",
        )
        self.cards["lost"].set_value(
            _fmt_int(checker.lost),
            "lost" + (f" · 首个 {checker.lost_ranges[0][0]}" if checker.lost_ranges else ""),
            tone="bad" if checker.lost else "normal",
        )
        self.cards["dup"].set_value(
            _fmt_int(checker.dup),
            "duplicate",
            tone="warn" if checker.dup else "normal",
        )
        self.cards["order"].set_value(
            _fmt_int(checker.order),
            "out-of-order",
            tone="warn" if checker.order else "normal",
        )
        self.cards["crc"].set_value(
            _fmt_int(crc),
            f"oversize {oversize} · resync {ps['resync'] - self._parser_baseline['resync']}",
            tone="bad" if crc else "normal",
        )
        self.cards["bytes"].set_value(
            _fmt_bytes(checker.payload_bytes),
            f"吞吐 {_fmt_bytes(checker.throughput_bps)}/s",
        )
        jitter = checker.jitter_ms
        self.cards["jitter"].set_value(
            f"{jitter:.1f} ms" if jitter is not None else "— ms",
            f"interval {checker.mean_interval_ms:.1f} ms" if checker.mean_interval_ms else "等待数据",
        )

    def _update_parser_status(self) -> None:
        ps = self.parser.stats_snapshot()
        self.parser_status.setText(
            f"解析器  frames={ps['frames']}  bytes={ps['bytes']}  "
            f"crc_err={ps['crc_err']}  resync={ps['resync']}  overflow={ps['overflow']}"
        )
        if self.recorder is not None:
            self.record_status.setText(f"录制中 · {self._record_rows} 行")
        elif self.record_path:
            self.record_status.setText("未录制")
        else:
            self.record_status.setText("未录制")

    # -- 录制 / 比较 ------------------------------------------------------

    def _default_record_path(self) -> str:
        stamp = datetime.now().strftime("%Y%m%d_%H%M%S_%f")[:-3]
        return f"host_test/run_{stamp}.csv"

    def _on_record_path_edited(self, text: str) -> None:
        self._auto_record_path = False
        self.record_path_edit.setToolTip(text)

    def _browse_record_path(self) -> None:
        path, _ = QFileDialog.getSaveFileName(
            self,
            "选择录制文件",
            self.record_path_edit.text() or self._default_record_path(),
            "CSV (*.csv)",
        )
        if path:
            self._auto_record_path = False
            self.record_path_edit.setText(path)
            self.record_path_edit.setCursorPosition(0)
            self.record_path_edit.setToolTip(path)

    def _on_record_toggled(self, checked: bool) -> None:
        if checked:
            if self._state != "connected":
                self.record_btn.blockSignals(True)
                self.record_btn.setChecked(False)
                self.record_btn.blockSignals(False)
                self.log.append("warn", "请先连接设备再录制")
                return
            self._start_recording()
        else:
            self._stop_recording()

    def _start_recording(self) -> None:
        if self.recorder is not None:
            return
        path = self.record_path_edit.text().strip()
        if not path:
            self._auto_record_path = True
        if self._auto_record_path:
            path = self._default_record_path()
            self.record_path_edit.setText(path)
            self.record_path_edit.setCursorPosition(0)
            self.record_path_edit.setToolTip(path)
        self.record_path = path
        try:
            self.recorder = Recorder(path)
        except OSError as exc:
            self.log.append("error", f"无法创建 CSV：{exc}")
            self.record_btn.blockSignals(True)
            self.record_btn.setChecked(False)
            self.record_btn.blockSignals(False)
            return
        self._record_rows = 0
        self.record_btn.blockSignals(True)
        self.record_btn.setChecked(True)
        self.record_btn.blockSignals(False)
        self.record_btn.setText("停止录制")
        self.log.append("rec", f"开始录制：{path}")
        self.record_status.setText("录制中 · 0 行")

    def _stop_recording(self) -> None:
        if self.recorder is None:
            return
        rows = self._record_rows
        path = self.record_path
        self.recorder.close()
        self.recorder = None
        self._record_rows = 0
        self.record_btn.blockSignals(True)
        self.record_btn.setChecked(False)
        self.record_btn.blockSignals(False)
        self.record_btn.setText("开始录制")
        self.log.append("rec", f"录制结束：{rows} 行 → {path}")
        self.record_status.setText("未录制")
        if self._auto_record_path:
            new_path = self._default_record_path()
            self.record_path_edit.setText(new_path)
            self.record_path_edit.setCursorPosition(0)
            self.record_path_edit.setToolTip(new_path)

    # -- 语音输入（ASR / 注入） -------------------------------------------

    def _browse_voice_model(self) -> None:
        start = self.voice_model_edit.text().strip() or str(Path.cwd() / "models")
        path = QFileDialog.getExistingDirectory(self, "选择 faster-whisper 模型目录", start)
        if path:
            self.voice_model_edit.setText(path)
            self.voice_model_edit.setToolTip(path)

    def _on_voice_mode_changed(self, _index: int) -> None:
        mode = self.voice_mode.currentData() or "auto"
        if self._voice_controller is not None:
            self._voice_controller.set_inject_mode(mode)
            self._voice_controller.set_inject_enabled(mode != "off")

    def _on_voice_toggled(self, checked: bool) -> None:
        if checked:
            if self._voice_controller is not None:
                self._voice_controller.close(timeout=2.0)
                self._voice_controller = None
            model = self.voice_model_edit.text().strip() or None
            mode = self.voice_mode.currentData() or "auto"
            engine = FasterWhisperEngine(
                model_path=model,
                language="zh",
                beam_size=1,
                on_log=self.sig_voice_log.emit,
            )
            controller = VoiceController(
                engine,
                Injector(),
                inject_enabled=(mode != "off"),
                inject_mode=mode,
                on_state=lambda state, detail: self.sig_voice_state.emit(state, detail),
                on_result=lambda result, inject_result, meta: self.sig_voice_result.emit(
                    result, inject_result, meta
                ),
                on_error=self.sig_voice_error.emit,
                on_log=self.sig_voice_log.emit,
            )
            self._voice_controller = controller
            self._voice_ready = False
            self.voice_model_edit.setEnabled(False)
            self.voice_status.setText("模型加载中…")
            self.voice_bar.setText("语音：加载模型…")
            self.log.append("voice", "启用语音输入，正在后台加载 ASR 模型…")
            controller.warmup_async()
        else:
            self._discard_open_audio("关闭语音输入")
            if self._voice_controller is not None:
                self._voice_controller.close(timeout=2.0)
                self._voice_controller = None
            self._voice_ready = False
            self.voice_model_edit.setEnabled(True)
            self.voice_status.setText("未启用")
            self.voice_bar.setText("语音：未启用")
            self.log.append("voice", "语音输入已关闭")

    def _on_voice_state(self, state: str, detail: str) -> None:
        if self._voice_controller is None:
            return
        detail = (detail or "").strip()
        if len(detail) > 90:
            detail = detail[:90] + "…"
        if state == "loading":
            self._voice_ready = False
            self.voice_status.setText("模型加载中…")
            self.voice_bar.setText("语音：加载模型…")
        elif state == "ready":
            self._voice_ready = True
            text = f"就绪 · {detail}" if detail else "就绪"
            self.voice_status.setText(text)
            self.voice_bar.setText("语音：就绪")
            self.voice_status.setStyleSheet("")
        elif state == "transcribing":
            self.voice_status.setText("识别中…")
            self.voice_bar.setText("语音：识别中…")
        elif state == "injecting":
            self.voice_status.setText("注入中…")
            self.voice_bar.setText("语音：注入中…")
        elif state == "error":
            self._voice_ready = False
            self.voice_status.setText(f"错误：{detail}")
            self.voice_status.setStyleSheet("color:#dc2626;")
            self.voice_bar.setText("语音：错误")
        else:
            self.voice_status.setText(detail or state)

    def _on_voice_result(self, result: Any, inject_result: Any, meta: Any) -> None:
        if self._voice_controller is None:
            return
        text = getattr(result, "text", "") or "（未识别到文字）"
        self.voice_result.setText(text)
        latency = getattr(result, "latency_ms", 0)
        audio_ms = getattr(result, "audio_ms", 0)
        self.voice_result.setToolTip(
            f"{text}\n\n音频 {audio_ms / 1000:.2f}s · ASR {latency} ms"
        )
        if inject_result is None:
            self.voice_status.setText("识别完成 · 未注入")
            self.voice_bar.setText("语音：识别完成（未注入）")
        elif getattr(inject_result, "ok", False):
            method = getattr(inject_result, "method", "")
            chars = getattr(inject_result, "chars", 0)
            self.voice_status.setText(f"识别完成 · 已注入 {chars} 字（{method}）")
            self.voice_bar.setText("语音：已注入")
            self.log.append("voice", f"注入成功 method={method} chars={chars}")
        else:
            error = getattr(inject_result, "error", "")
            self.voice_status.setText(f"识别完成 · 注入失败")
            self.voice_bar.setText("语音：注入失败")
            self.log.append("error", f"注入失败：{error}")

    def _on_voice_error(self, message: str) -> None:
        if self._voice_controller is None:
            return
        self.log.append("error", f"语音：{message}")
        short = message.strip().splitlines()[0] if message.strip() else "未知错误"
        self.voice_status.setText(f"错误：{short[:90]}")
        self.voice_bar.setText("语音：错误")
        if self._last_voice_wav:
            self.voice_status.setToolTip(f"上一段 WAV：{self._last_voice_wav}")

    def _on_voice_log(self, message: str) -> None:
        if self._voice_controller is None:
            return
        self.log.append("voice", message)

    def _save_voice_segment_wav(self, segment: AudioSegment) -> str:
        pcm, missing = decode_segment(segment)
        directory = Path.cwd() / "logs" / "voice"
        try:
            directory.mkdir(parents=True, exist_ok=True)
            stamp = time.strftime("%Y%m%d_%H%M%S")
            path = directory / f"seg_{stamp}_{segment.seg_id}.wav"
            write_wav(str(path), pcm)
        except OSError as exc:
            self.log.append("error", f"保存语音 WAV 失败：{exc}")
            return ""
        self._last_voice_wav = str(path)
        self.log.append(
            "rec",
            f"语音段 seg={segment.seg_id} {segment.received}/{segment.total} 块"
            f"{'，缺 ' + str(len(missing)) if missing else ''} → {path}",
        )
        return str(path)

    def _handle_finished_segment(self, segment: AudioSegment) -> None:
        if segment is None or not segment.blocks:
            self.log.append("warn", f"语音段 seg={getattr(segment, 'seg_id', '?')} 没有音频块")
            return
        self.log.append(
            "info",
            f"封段 seg={segment.seg_id} blocks={segment.received}/{segment.total} "
            f"duration={segment.duration_ms / 1000:.2f}s",
        )
        if not self.voice_enable.isChecked():
            return
        wav_path = self._save_voice_segment_wav(segment)
        controller = self._voice_controller
        if controller is None:
            self.log.append("warn", "语音控制器未就绪，未提交识别")
            return
        if not self._voice_ready:
            self.log.append("voice", "ASR 模型仍在加载，本段已排队，加载完成后自动识别")
        self.voice_status.setText("识别中…")
        self.voice_bar.setText("语音：识别中…")
        controller.submit_segment(segment.blocks, seg_id=segment.seg_id, source="ptt")
        self.log.append(
            "voice",
            f"提交识别 seg={segment.seg_id}（{segment.duration_ms / 1000:.2f}s）"
            + (f"，WAV {wav_path}" if wav_path else ""),
        )

    def _discard_open_audio(self, reason: str) -> None:
        segment = self._audio.discard_open(reason)
        if segment is not None and segment.blocks:
            self.log.append(
                "warn",
                f"未封口语音段 seg={segment.seg_id}（{segment.received} 块）已丢弃：{reason}",
            )

    # -- 音频接收（AUDIO_DATA） ---------------------------------------------

    def _on_audio_toggled(self, checked: bool) -> None:
        if checked:
            if self._state != "connected":
                self.audio_btn.blockSignals(True)
                self.audio_btn.setChecked(False)
                self.audio_btn.blockSignals(False)
                self.log.append("warn", "请先连接设备再接收音频")
                return
            self._audio.reset()
            self._audio_dirty = False
            self._start_player()
            self.audio_btn.setText("停止录制")
            self.audio_status.setText("已开启 · 按住 KEY1 说话，实时监听中")
            self.audio_save_btn.setEnabled(False)
            self.log.append("rec", "开始接收音频（实时播放 + 录制）")
        else:
            self._stop_player()
            self._finish_audio_session()

    def _start_player(self) -> None:
        if WinAudioOut is None:
            self.log.append("warn", "当前环境不支持实时播放，仅录制")
            return
        try:
            # 20 ms 一块 × 3 = 60 ms 上限，典型 20–40 ms；再小容易在 Windows
            # 调度抖动下欠载（欠载的表现是"咔哒"声）。
            self._player = WinAudioOut(rate=SAMPLE_RATE, channels=1, bits=16,
                                       buf_ms=20, buffers=3)
            self._player.start()
            self._player_timer.start()
            self.log.append("info", "实时播放已启动（waveOut 16 kHz / mono / 20 ms 缓冲）")
        except Exception as exc:
            self._player = None
            self.log.append("warn", f"实时播放启动失败，仅录制：{exc}")

    def _stop_player(self) -> None:
        self._player_timer.stop()
        if self._player is not None:
            try:
                self._player.stop()
            except Exception:
                pass
            self._player = None

    def _poll_player(self) -> None:
        if self._player is not None:
            self._player.poll()

    def _finish_audio_session(self) -> None:
        received, total, missing = self._audio.stats()
        self._refresh_audio_status(final=True)
        self.audio_btn.setText("开始接收音频")

        if received == 0:
            self.log.append("rec", "停止录制（未收到音频）")
            return

        pcm = self._audio.pcm()
        path = Path.cwd() / f"record_{time.strftime('%Y%m%d_%H%M%S')}.wav"
        try:
            write_wav(str(path), pcm)
        except OSError as exc:
            self.log.append("error", f"自动保存 WAV 失败：{exc}")
            return

        self._last_wav = str(path)
        duration = len(pcm) / 2 / SAMPLE_RATE
        self.log.append(
            "rec",
            f"停止录制：{duration:.2f} s（块 {received}/{total}，缺 {len(missing)}）→ {path}",
        )

    def _refresh_audio_status(self, final: bool = False) -> None:
        received, total, missing = self._audio.stats()

        if final:
            dev = "设备已停止"
        elif self._audio.talking:
            dev = "设备说话中"
        else:
            dev = "设备静音"

        if total == 0:
            self.audio_status.setText(f"{dev} · 尚未收到音频块")
            self.audio_save_btn.setEnabled(False)
            return

        seconds = total * 0.02
        text = (
            f"{dev} · 块 {received}/{total} · 缺失 {len(missing)} · "
            f"{seconds:.2f} s · 坏帧 {self._audio.bad}"
        )
        if missing:
            head = ", ".join(str(i) for i in missing[:8])
            text += f"\n缺失 seq: {head}{' …' if len(missing) > 8 else ''}"
        self.audio_status.setText(text)
        self.audio_save_btn.setEnabled(received > 0)

    def _save_audio_wav(self) -> None:
        received, total, missing = self._audio.stats()
        if received == 0:
            self.log.append("warn", "没有可保存的音频数据")
            return

        default = self._last_wav or str(Path.cwd() / "out.wav")
        path, _ = QFileDialog.getSaveFileName(self, "保存 WAV", default, "WAV (*.wav)")
        if not path:
            return

        pcm = self._audio.pcm()
        try:
            write_wav(path, pcm)
        except OSError as exc:
            self.log.append("error", f"保存 WAV 失败：{exc}")
            return

        duration = len(pcm) / 2 / SAMPLE_RATE
        self.log.append(
            "rec",
            f"已保存 {duration:.2f} s（{received}/{total} 块，缺 {len(missing)}）→ {path}",
        )

    def _compare_dialogs(self) -> None:
        path_a, _ = QFileDialog.getOpenFileName(self, "选择记录 A（USB）", str(Path.cwd()), "CSV (*.csv)")
        if not path_a:
            return
        path_b, _ = QFileDialog.getOpenFileName(self, "选择记录 B（BLE）", str(Path.cwd()), "CSV (*.csv)")
        if not path_b:
            return
        try:
            result = compare_records(path_a, path_b)
        except (OSError, ValueError) as exc:
            self.log.append("error", f"比较失败：{exc}")
            return
        for line in result.lines:
            self.log.append("info" if result.ok else "warn", f"COMPARE {line}")
        for failure in result.failures:
            self.log.append("warn", f"COMPARE  - {failure}")
        title = "比较通过" if result.ok else "比较失败"
        detail = "\n".join(result.lines + ([] if result.ok else ["", *result.failures]))
        if result.ok:
            QMessageBox.information(self, title, detail)
        else:
            QMessageBox.warning(self, title, detail)

    # -- 生命周期 ---------------------------------------------------------

    def closeEvent(self, event: Any) -> None:
        self._closing = True
        self._intentional_close = True
        self._drain_timer.stop()
        self._ui_timer.stop()
        self._stop_recording()
        if self._voice_controller is not None:
            try:
                self._voice_controller.close(timeout=2.0)
            except Exception:
                pass
            self._voice_controller = None
        transport = self.transport
        self.transport = None
        if transport is not None:
            try:
                transport.close()
            except Exception:
                pass
        thread = self._connect_thread
        if thread is not None and thread.isRunning():
            thread.wait(9000)
        close_thread = self._close_thread
        if close_thread is not None and close_thread.isRunning():
            close_thread.wait(5000)
        event.accept()


# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------


def _apply_palette(app: QApplication) -> None:
    palette = QPalette()
    palette.setColor(QPalette.ColorRole.Window, QColor(BG))
    palette.setColor(QPalette.ColorRole.WindowText, QColor(TEXT))
    palette.setColor(QPalette.ColorRole.Base, QColor("#ffffff"))
    palette.setColor(QPalette.ColorRole.AlternateBase, QColor("#f8fafd"))
    palette.setColor(QPalette.ColorRole.Text, QColor(TEXT))
    palette.setColor(QPalette.ColorRole.Button, QColor("#ffffff"))
    palette.setColor(QPalette.ColorRole.ButtonText, QColor(TEXT))
    palette.setColor(QPalette.ColorRole.Highlight, QColor(PRIMARY))
    palette.setColor(QPalette.ColorRole.HighlightedText, QColor("#ffffff"))
    palette.setColor(QPalette.ColorRole.ToolTipBase, QColor("#0f172a"))
    palette.setColor(QPalette.ColorRole.ToolTipText, QColor("#ffffff"))
    app.setPalette(palette)


def create_app(argv: Optional[List[str]] = None) -> QApplication:
    app = QApplication.instance()
    if app is None:
        app = QApplication(sys.argv if argv is None else [sys.argv[0], *argv])
    app.setApplicationName("DeskPilot Host")
    app.setApplicationDisplayName("DeskPilot 上位机验证台")
    app.setOrganizationName("DeskPilot")
    app.setStyle("Fusion")
    app.setWindowIcon(_make_icon())
    font = QFont("Microsoft YaHei UI")
    font.setPointSize(9)
    app.setFont(font)
    _apply_palette(app)
    app.setStyleSheet(QSS)
    return app


def main(argv: Optional[List[str]] = None) -> int:
    app = create_app(argv)
    window = MainWindow()
    window.show()
    # 无界面冒烟测试/CI 用：HOST_TEST_GUI_AUTOCLOSE_MS=1500
    autoclose = os.environ.get("HOST_TEST_GUI_AUTOCLOSE_MS")
    if autoclose:
        try:
            delay = max(200, int(autoclose))
            QTimer.singleShot(delay, window.close)
            QTimer.singleShot(delay + 150, app.quit)
        except ValueError:
            pass
    return int(app.exec())


if __name__ == "__main__":
    raise SystemExit(main())
