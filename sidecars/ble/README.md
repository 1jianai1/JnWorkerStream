# BLE sidecar（开发态）

headless Python 进程，stdin 收协议帧、stdout 只输出协议帧、stderr 为日志。
只做 BLE GATT 连接与字节转发，不解析 `A5 5A`。

## 依赖

- Python 3.10+
- `bleak`、`winrt`（Windows）
- 本机开发环境：`D:\Miniconda\envs\kbhost\python.exe`（已装好）

```powershell
pip install -r sidecars/ble/requirements.txt
```

## 运行

```powershell
python sidecars/ble/main.py
# 或
python -m sidecars.ble
```

Rust 侧通过 `JNWS_PYTHON` 指定解释器：

```powershell
$env:JNWS_PYTHON='D:\Miniconda\envs\kbhost\python.exe'
cargo test -p kb-transport --test ble_hardware -- --ignored --nocapture
```

## 方法（`docs/interfaces.md` §4.3）

| method | 说明 |
|---|---|
| `ble.scan` | 扫描 + 已配对合并去重 |
| `ble.list_paired` | 仅枚举 Windows 已配对设备（HID 占用不广播时用） |
| `ble.connect` | 优先配对直连；`address`/`name`/`paired_only`/`timeout_ms` |
| `ble.disconnect` | 幂等 |
| `ble.write` | `data_b64`，单包；> `mtu-3` 返回 `E_PAYLOAD_TOO_LARGE` |
| `ble.set_notify` | 重连后需重新开启 |
| `ping` | 心跳 |
| `shutdown` | 优雅退出 |

事件：`ble.bytes`（body=原始字节）、`ble.link`（`state`/`reason`/`mtu`）。

## 测试

```powershell
python -m unittest discover -s sidecars/ble -p "test_*.py"
```

## 故障排查

- `E_BLE_DEVICE_NOT_FOUND`：设备未开机/未在范围内；先用 `ble.list_paired` 确认配对记录；
- `E_BLE_CONNECT_TIMEOUT`：设备被其他主机占用，或未进入 BLE 模式；
- GATT 找不到 DATA_TX/DATA_RX：固件改过特征表且 Windows 缓存了旧结构，需删除配对记录后重配；
- 连接后 MTU 仍是 23：设备未做 MTU 交换，payload 上限会降为 9 B。
