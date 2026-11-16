"""帧协议：复用 `sidecars/ble/protocol.py` 的冻结实现（ADR-007）。

ASR 与 BLE sidecar 用同一套帧格式，所以这里不复制第二份编解码代码：按文件路径加载
`sidecars/ble/protocol.py`（绕开 `sidecars.ble.__init__` 对 bleak 的依赖），再把名字
重新导出给本包。改动帧格式只需改一处。
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

_PATH = Path(__file__).resolve().parents[1] / "ble" / "protocol.py"
if not _PATH.is_file():  # pragma: no cover - 打包后目录结构异常
    raise ImportError(f"找不到共享协议实现：{_PATH}")

_SPEC = importlib.util.spec_from_file_location("jnws_sidecar_protocol", _PATH)
if _SPEC is None or _SPEC.loader is None:  # pragma: no cover
    raise ImportError(f"无法加载协议实现：{_PATH}")
_MODULE = importlib.util.module_from_spec(_SPEC)
sys.modules.setdefault("jnws_sidecar_protocol", _MODULE)
_SPEC.loader.exec_module(_MODULE)

#: 实际加载的共享实现（`sidecars/ble/protocol.py`），便于自检/排查。
SOURCE = _PATH
SOURCE_MODULE = _MODULE.__name__

MAGIC = _MODULE.MAGIC
PROTO_VERSION = _MODULE.PROTO_VERSION
HEADER_PREFIX = _MODULE.HEADER_PREFIX
MAX_FRAME = _MODULE.MAX_FRAME
FLAG_BODY_GZIP = _MODULE.FLAG_BODY_GZIP
ProtocolError = _MODULE.ProtocolError
now_ms = _MODULE.now_ms
encode_frame = _MODULE.encode_frame
write_frame = _MODULE.write_frame
read_frame = _MODULE.read_frame
hello = _MODULE.hello
response = _MODULE.response
error = _MODULE.error
event = _MODULE.event

__all__ = [
    "SOURCE",
    "SOURCE_MODULE",
    "MAGIC",
    "PROTO_VERSION",
    "HEADER_PREFIX",
    "MAX_FRAME",
    "FLAG_BODY_GZIP",
    "ProtocolError",
    "now_ms",
    "encode_frame",
    "write_frame",
    "read_frame",
    "hello",
    "response",
    "error",
    "event",
]
