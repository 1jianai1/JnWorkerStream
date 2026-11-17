# JnWorkerStream

Windows 优先的数字键盘上位机：**Tauri 2 + React 19/TypeScript + Rust**。
当前阶段 **P2c-1（BLE 链路）**：`sidecars/ble` + `kb-sidecar` + `kb-transport/ble`
已落地，真机直连 MTU 247、415 帧/0 CRC、5 段 WAV 已通过；P2a 的协议/校验/音频/USB 核心保留。
UI 仍使用 P1 Mock，30 分钟长稳/断线重连与 ASR/注入待完成。

## 快速开始

```powershell
# 前端（Mock 数据，浏览器即可运行）
pnpm --dir apps/desktop install
pnpm --dir apps/desktop dev            # http://localhost:5173

# 类型检查 / 单测 / Lint / 构建
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop build

# Playwright 冒烟与截图（需 Edge/Chromium；本机可用 JNWS_CDP 连接已启动浏览器）
pnpm --dir apps/desktop e2e
pnpm --dir apps/desktop e2e:shots

# Rust 设置持久化 + 协议/校验/音频/传输/sidecar
cargo test -p kb-config
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# BLE sidecar（开发机使用 kbhost Python）
python -m unittest discover -s sidecars/ble -p "test_*.py"
$env:JNWS_PYTHON='D:\Miniconda\envs\kbhost\python.exe'
cargo test -p kb-transport --test ble_hardware -- --ignored --nocapture

# Tauri 壳（需正常网络下载依赖并安装 WebView2；先准备 tauri-cli）
cargo install tauri-cli --version "^2"
cargo tauri dev
```

## 文档

- 需求基线：`py_example/host_develop_requirement.md`
- 入门导读（不熟前端/Rust 从这读）：`docs/beginner/README.md`
- 文档索引：`docs/README.md`
- 阶段门禁：`docs/acceptance.md`
- 接口冻结清单：`docs/interfaces.md`
- 设计系统：`docs/design-system.md`
- P1 验收记录：`docs/acceptance-records/P1-ui-mock.md`
- P2a 验收记录：`docs/acceptance-records/P2a-usb-protocol.md`
- P2c-1 验收记录：`docs/acceptance-records/P2c1-ble-link.md`
- 截图基线：`docs/screenshots/p1/`

## 目录

```text
apps/desktop/   React + Vite 前端（Mock 仅经 lib/ipc.ts 进入）
src-tauri/      最小 Tauri 2 壳
crates/         kb-config（P1）；kb-protocol/kb-check/kb-transport/kb-audio（P2a）；kb-sidecar（P2c）
sidecars/ble/   BLE sidecar（Python + bleak/WinRT）
docs/           架构、接口、ADR、验收、截图
skills/         开发技能
py_example/     Python 参考实现（只读）
```

## 约束

- Rust 是唯一业务状态源（ADR-006）；P1 前端只镜像 Mock，P2a 起切换 Tauri 命令/事件。
- 公共接口变更必须先写 ADR（ADR-009）。
- 未到达阶段的传输/脚本/AI 不实现（ADR-010）。
- 不修改 `py_example/**`。
