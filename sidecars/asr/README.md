# ASR sidecar（开发态）

headless Python 进程，stdin 收协议帧、stdout 只输出协议帧、stderr 为日志。
只做「PCM → 文本」，不做分段与注入（分段在 `kb-audio`，注入在 `kb-inject`）。

帧格式直接复用 `sidecars/ble/protocol.py`（同一份冻结实现，见 ADR-007），本目录不复制编解码代码。

## 依赖

- Python 3.10+
- `faster-whisper==1.2.1` + `numpy`
- 模型：`models/faster-whisper-small`（已随仓库工作区下载，见 `.gitignore`）
- 本机开发环境：`D:\Miniconda\envs\kbhost\python.exe`

```powershell
pip install -r sidecars/asr/requirements.txt
```

模型下载（HF 直连不通时用镜像）：

```powershell
$env:HF_ENDPOINT='https://hf-mirror.com'
$env:HF_HOME="$PWD\.cache\hf"          # 沙箱下必须放工作区内
$env:HF_HUB_DISABLE_XET='1'
python -c "from huggingface_hub import snapshot_download; snapshot_download('Systran/faster-whisper-small', local_dir='models/faster-whisper-small')"
```

## 运行

```powershell
python -m sidecars.asr                 # 真实引擎
python -m sidecars.asr --selftest      # 不需要模型
$env:JNWS_ASR_FAKE='1'; python -m sidecars.asr   # EchoEngine，给集成测试用
```

模型路径优先级：`asr.load.model_path` → `JNWS_ASR_MODEL` → `models/faster-whisper-small` → 模型名 `small`。

## 方法（`docs/interfaces.md` §4.4）

| method | 说明 |
|---|---|
| `asr.load` | 幂等；`device=cpu`、`compute_type=int8`、`language=zh`、`beam_size=1`、`vad_filter=true` |
| `asr.warmup` | 0.5 s 静音前推，压掉首句冷启动 |
| `asr.transcribe` | body = PCM16 16 kHz 单声道；只接受 16000 Hz |
| `asr.cancel` | `target_id` 指向要取消的 transcribe；迟到结果按 id 丢弃，响应带 `cancelled:true` |
| `asr.unload` | 释放模型 |
| `ping` / `shutdown` | 心跳 / 优雅退出 |

识别串行：`asr.transcribe` 进单线程队列，`ping`/`asr.cancel` 不会被识别阻塞。

## 测试

```powershell
python -m unittest discover -s sidecars/asr -p "test_*.py"
python -m sidecars.asr --selftest
```

## 已知约束

- 不做重采样：上游必须给 16 kHz；48 kHz 输入返回 `E_INVALID_ARG`；
- 取消是「丢弃结果」而不是「中断推理」（faster-whisper 无法中断单次前推）；
- 未装 `faster-whisper` 时 `asr.load` 返回 `E_ASR_FAILED`，模型目录缺失返回 `E_ASR_MODEL_MISSING`。
