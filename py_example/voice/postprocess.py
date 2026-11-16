"""识别文本轻量后处理（V1 只做清理，热词/命令留给 V3）。"""

from __future__ import annotations


def clean_text(text: str) -> str:
    if not text:
        return ""
    # 去掉首尾空白和多余换行；保留中文标点
    text = text.replace("\u200b", "").replace("\ufeff", "")
    text = " ".join(line.strip() for line in text.splitlines() if line.strip())
    return text.strip()
