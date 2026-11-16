"""JnWorkerStream ASR sidecar（开发态）。

运行（推荐 kbhost 环境）::

    python -m sidecars.asr
    python -m sidecars.asr --selftest

注：包本身不 import `main`，避免 `sidecars.asr.main` 子模块被同名函数覆盖；
入口见 `__main__.py`。
"""
