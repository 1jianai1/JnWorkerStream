"""下载 faster-whisper 模型（默认走 ModelScope 国内镜像）。

    python -m host_test.voice.download_model --model small --out models/faster-whisper-small

下载完成后 FasterWhisperEngine 可直接用 ``--model <目录>`` 加载。
"""

from __future__ import annotations

import argparse
import os
import sys
import time
import urllib.request
from pathlib import Path
from typing import List, Optional

MIRROR = "https://modelscope.cn/models/pengzhendong"
REPOS = {
    "tiny": "faster-whisper-tiny",
    "base": "faster-whisper-base",
    "small": "faster-whisper-small",
}
SKIP = {".gitattributes", "README.md", "configuration.json"}


def _api_files(repo: str) -> List[dict]:
    import json

    url = f"https://modelscope.cn/api/v1/models/pengzhendong/{repo}/repo/files?Revision=master&Root="
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=30) as response:
        data = json.load(response)
    return data.get("Data", {}).get("Files", [])


def download(model: str, out_dir: str, force: bool = False) -> int:
    repo = REPOS.get(model, model)
    files = _api_files(repo)
    if not files:
        print(f"没有取到 {repo} 的文件列表", file=sys.stderr)
        return 1
    target = Path(out_dir)
    target.mkdir(parents=True, exist_ok=True)
    failed = 0
    for item in files:
        name = item.get("Name", "")
        if not name or name in SKIP or item.get("Type") != "blob":
            continue
        size = int(item.get("Size") or 0)
        path = target / name
        if path.exists() and path.stat().st_size == size and not force:
            print(f"skip {name} ({size} B)")
            continue
        url = f"{MIRROR}/{repo}/resolve/master/{name}"
        print(f"download {url} ({size / 1048576:.1f} MB)")
        part = path.with_suffix(path.suffix + ".part")
        started = time.time()
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
            with urllib.request.urlopen(req, timeout=120) as response, open(part, "wb") as fh:
                total = 0
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    fh.write(chunk)
                    total += len(chunk)
                    if total // (20 * 1024 * 1024) != (total - len(chunk)) // (20 * 1024 * 1024):
                        print(f"  {name}: {total / 1048576:.0f} MB")
            os.replace(part, path)
            elapsed = max(0.01, time.time() - started)
            print(f"done {name}: {total / 1048576:.1f} MB in {elapsed:.1f}s")
        except Exception as exc:
            failed += 1
            print(f"FAILED {name}: {exc}", file=sys.stderr)
            try:
                part.unlink()
            except OSError:
                pass
    if failed:
        return 1
    print(f"model ready: {target}")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="host_test.voice.download_model")
    parser.add_argument("--model", default="small", choices=sorted(REPOS))
    parser.add_argument("--out", default=None, help="输出目录（默认 models/faster-whisper-<model>）")
    parser.add_argument("--force", action="store_true", help="已存在也重新下载")
    args = parser.parse_args(argv)
    out = args.out or f"models/faster-whisper-{args.model}"
    return download(args.model, out, args.force)


if __name__ == "__main__":
    raise SystemExit(main())
