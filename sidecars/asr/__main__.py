"""`python -m sidecars.asr` 入口。"""

import sys

from .main import run_cli, selftest

if __name__ == "__main__":
    if "--selftest" in sys.argv:
        raise SystemExit(selftest())
    run_cli()
