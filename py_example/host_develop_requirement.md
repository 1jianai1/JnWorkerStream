# 上位机重构开发要求与迁移要点

> 版本：v1.0  
> 日期：2026-11-08  
> 当前参考实现：`py_example/`（v1.1.0，Python / PySide6）  
> 目标工程：`G:\Code\My_Project\DigitalKeyboard\JnWorkerStream`  
> 本文状态：**需求、决策与迁移约束，不含实现代码**
> 目标阶段：只生成架构与后续开发所需的 skill 文档，不生成代码。

---

## 1. 背景与最终决定

当前上位机已经具备一套可用的核心能力：

- USB CDC / BLE 私有 GATT / 模拟链路；
- `A5 5A` 帧解析、校验、丢帧统计、CSV 录制与比较；
- ADPCM 音频接收、PTT 分段、实时播放、WAV 保存；
- faster-whisper 本地识别、SendInput / 剪贴板注入；
- 浅色 GUI、日志筛选/暂停/导出、自动重连；
- 29 项无硬件自检。

现状约 **8,001 行 Python / 25 个模块**，其中 `gui.py` 约 2,551 行。参考实现的复杂度集中在：

1. Windows BLE：配对设备直连、真实 MTU、HID 占用时不广播；
2. ASR：faster-whisper + CTranslate2 模型；
3. Windows 文本注入；
4. GUI 日志/高频事件处理。

**最终决定：重构为 TypeScript + Rust + Tauri 的前后端架构，目标目录为 `JnWorkerStream`。**

为避免同时重赌 BLE、ASR、UI 三层，重构采用渐进迁移：

- 协议、串口、音频、注入、状态管理逐步进入 Rust；
- BLE、ASR 先以 **headless Python sidecar** 保留，Rust 通过显式进程协议调用；
- 前端 TypeScript 只做 UI 与命令/事件；
- BLE、ASR 分别设置替换里程碑，sidecar 不是永久架构；
- Windows 优先，跨平台作为后续目标，不阻塞第一阶段。

---

## 2. 目标与非目标

### 2.1 目标

- 精美的浅色桌面 UI，日志可读、可筛选、可暂停、可导出；
- Rust 负责协议、传输、音频、注入、状态和进程管理；
- TypeScript 负责界面和交互；
- 本机 sidecar 通信不依赖网络端口；
- 支持后续扩展：脚本 Worker、AI Provider、工具调用；
- 每个开发阶段都可以独立构建、运行、测试、截图验收；
- 保留现有硬件链路的既有能力，不因重构退步。

### 2.2 非目标（当前阶段）

- 不生成任何项目代码、脚手架、依赖配置；
- 不修改当前 `py_example/` 的既有功能；
- 不实现远程 HTTP/WebSocket 服务端；
- 不先做 2.4G、脚本引擎或 AI 工具调用；
- 不承诺 Windows/macOS/Linux 三平台同一阶段上线；
- 不把 BLE/ASR 直接搬进 TypeScript 浏览器环境。

---

## 3. 当前参考实现盘点

| 参考文件 | 能力 | 目标归属 |
|---|---|---|
| `frame.py` | CRC16、组帧、流式解析、重同步 | Rust `protocol` |
| `check.py` | 序号/模式/抖动、CSV、比较 | Rust `check` |
| `transport/base.py` | 传输抽象 | Rust trait |
| `transport/serial_tx.py` | USB CDC、选口、读线程、重连 | Rust `transport/serial` |
| `transport/ble_tx.py` | BLE、配对直连、MTU、Notify | Python BLE sidecar（过渡）→ Rust BLE |
| `transport/sim_tx.py` | 模拟设备、异常注入 | Rust `transport/sim` |
| `audio_rx.py` | ADPCM、WAV、按段输出 | Rust `audio` |
| `win_audio.py` | waveOut 实时播放 | Rust `audio/playback`（cpal/WASAPI） |
| `voice/session.py` | `AUDIO_CTL` 分段与丢弃规则 | Rust `audio/session` |
| `voice/controller.py` | 解码→ASR→注入 | Rust `voice/controller` |
| `voice/asr/*` | faster-whisper 本地引擎 | Python ASR sidecar（过渡）→ 原生 ASR |
| `voice/inject.py` | SendInput / 剪贴板 | Rust `inject` |
| `voice/download_model.py` | 模型下载 | Rust `model/downloader` |
| `gui.py` | PySide6 界面 | TypeScript/Tauri 重写 |
| `cli.py` / `audio_rx.py` | CLI | Rust CLI 子命令 |
| `selftest.py` | 29 项回归 | `cargo test` + Playwright + 硬件脚本 |

### 3.1 当前协议事实

- 帧：`A5 5A | ver(1) | type(1) | flags(1) | seq(2 LE) | len(2 LE) | payload | crc16(2 LE)`；
- CRC-16/CCITT-FALSE，多字节小端；
- `AUDIO_CTL(0x02)`：`ev(1) + seg_id(2 LE)`；一次 KEY1 = 一个 `seg_id` = 一段 = 一次 ASR；
- `AUDIO_DATA(0x01)`：`fmt(1) + block_seq(2 LE) + 160B ADPCM`；
- `TEST_DATA(0x60)` 用于回环/测试；
- 新段开始前未封口、链路断开未封口：丢弃，不拼接。

### 3.2 当前已验证事实

- 29 项自检通过；
- 4.1 s 示例 WAV 识别约 1.2–1.3 s；
- BLE 配对直连实测 MTU 247；
- GUI 可完成“合成音频段 → ASR → 结果卡显示”；
- SendInput 中英文注入临时焦点窗口成功；
- 剪贴板注入与恢复成功；
- 首句模型加载/预热约 3–6 s。

---

## 4. 目标架构

```
Tauri WebView（TypeScript）
  ├─ 连接 / 统计 / 日志 / 音频 / 语音 / 设置
  └─ 只发命令、收批量事件，不直接访问硬件
        │ Tauri commands / events（进程内 IPC）
Rust core
  ├─ protocol：frame / crc / parser
  ├─ check：seq / csv / compare
  ├─ transport：serial / ble / sim
  ├─ audio：adpcm / session / playback / wav
  ├─ voice：segmentation / controller
  ├─ inject：SendInput / clipboard
  └─ sidecar supervisor
        ├─ Python BLE sidecar（过渡）
        └─ Python ASR sidecar（过渡）
```

### 4.1 目录建议（未来代码阶段才创建）

```
JnWorkerStream/
  apps/desktop/            # Tauri + TS
  src-tauri/               # Tauri 壳、commands、events、state
  crates/
    kb-protocol/
    kb-check/
    kb-transport/
    kb-audio/
    kb-voice/
    kb-inject/
    kb-sidecar/
  sidecars/
    ble/                   # Python headless BLE（过渡）
    asr/                   # Python headless ASR（过渡）
  skills/                  # 本次唯一产物：开发技能
  docs/                    # 架构、接口、ADR、验收
  models/                  # 模型（不入库）
```

### 4.2 职责边界

- **TypeScript**：页面、组件、样式、交互、状态展示、命令调用；不得直接实现 BLE/串口/ASR/注入；
- **Rust**：所有硬件、协议、音频、注入、文件与进程；是唯一业务状态源；
- **Python sidecar**：只保留 BLE 与 ASR 的现有实现；不包含 PySide6、不包含 UI、不包含业务命令；
- **Sidecar 协议**：本地 stdin/stdout 长度前缀二进制帧；不使用 HTTP、不使用每次启动 CLI。

---

## 5. 技术决策（必须写 ADR，不得随意变更）

| 编号 | 决策 | 状态 |
|---|---|---|
| D-01 | 采用 Tauri 2 + TypeScript 前端 + Rust core；使用进程内 IPC | 已定 |
| D-02 | Windows 优先；跨平台作为后续阶段 | 已定 |
| D-03 | UI 使用 TypeScript；不得使用 Web Bluetooth 替代 Rust BLE | 已定 |
| D-04 | BLE 与 ASR 第一阶段保留为 Python headless sidecar | 已定（过渡） |
| D-05 | 协议解析、串口、音频分段/播放、注入、状态由 Rust 负责 | 已定 |
| D-06 | Rust 是唯一状态源，前端只镜像快照/增量 | 已定 |
| D-07 | Sidecar 使用长度前缀 + 二进制 payload + JSON 头 | 已定 |
| D-08 | 高频事件必须批量发送；音频与原始帧不逐条走 IPC | 已定 |
| D-09 | 公共接口变更必须新增 ADR 并获确认 | 已定 |
| D-10 | 2.4G、TCP/UDP、脚本、AI 属于后续阶段，未定协议前不实现 | 已定 |

### 5.1 Sidecar 要点

- 常驻进程，启动后握手：协议版本、能力、模型路径；
- stdout 只输出协议；日志走 stderr；
- 请求带 `request_id`，响应/错误关联；
- BLE 原始字节输出给 Rust，由 Rust 统一解析；
- ASR 接收 PCM/WAV，返回文本、语言、音频时长、ASR 延迟；
- 有界队列、背压、心跳、超时、崩溃重启；
- 退出时清理子进程，防止孤儿进程；
- 发布阶段用 PyInstaller/Nuitka 打包，不依赖用户本机 Python；开发阶段可用 `kbhost`。

### 5.2 BLE 迁移原则

- Python BLE sidecar 保留当前 `ble_tx.py` 的关键行为：扫描、已配对枚举、按地址直连、MTU、Notify、重连；
- Rust BLE 原生实现必须先通过同一套硬件验收；
- 若 `btleplug` 覆盖不了配对直连/真实 MTU，使用 `windows` crate 封装 WinRT；
- 在 Rust BLE 通过验收前，不得删除 Python BLE sidecar。

### 5.3 ASR 迁移原则

- 第一版 ASR 使用 Python sidecar，保证现有准确率/延迟；
- 后续对比 `whisper-rs` / `sherpa-onnx`；
- 原生 ASR 需使用同一批 WAV 验证准确率和 <1.5 s 延迟后才可替换；
- 模型文件不入库；提供下载/校验流程。

### 5.4 注入与播放

- 注入用 Rust + Windows `SendInput` / 剪贴板，保留焦点校验、剪贴板恢复、UIPI 提示；
- 播放用 Rust `cpal` 或 WASAPI；不经过 TS/sidecar；
- 音频播放、ASR、BLE 在不同线程/进程，互不阻塞。

---

## 6. 正确开发流程（强制顺序）

1. **先设计架构**  
   让 Agent 生成项目结构、模块职责、接口定义、技术决策文档。  
   **门禁：架构文档评审通过后才能写代码。**
2. **先确定 UI 设计系统**  
   已选定 PostHog 浅色设计语言，先产出 `docs/design-system.md`；不得自由配色。  
   **门禁：设计 token/组件规则评审通过。**
3. **再完成 UI 原型**  
   不接硬件，用 Mock 数据完成主要页面；可运行、可截图、可交互。
4. **逐个实现通信模块**  
   顺序：串口 → TCP/UDP → BLE → USB（CDC/HID）→ 自定义 2.4G。  
   每个模块独立验收，不同时展开。
5. **接入真实设备**  
   验证数据收发、断连、重连、吞吐、长稳；BLE 需验证 HID 占用场景。
6. **增加脚本和 AI**  
   先做脚本 Worker（隔离/权限/超时），再做 AI Provider 和工具调用。
7. **持续回归测试**  
   每个功能完成后：构建 → 单元测试 → 运行 → 截图 → 控制台检查 → UI 对比 → 报告。

**禁止**先写完整业务再补 UI/测试；**禁止**在没有硬件验证前宣称通信模块完成。

### 6.1 UI 设计系统（已选 PostHog）

- 主设计语言：PostHog 浅色设计（暖米色画布、白色卡片、橄榄文字、橙黄主操作、语义状态色）；
- 参考文件：`G:\OpenProj\awesome-design-md\awesome-design-md\design-md\posthog\DESIGN.md`；
- 对应 Skill：`JnWorkerStream/skills/jnws-ui-design/SKILL.md`；
- UI 设计先于 UI Mock，必须先产出 `docs/design-system.md`；
- 不复制 PostHog 品牌 Logo、吉祥物、插画和营销页结构；
- 不做纯深色主题；深色只用于代码/原始字节块；
- 所有 UI 任务和 UI Review 都必须读取该 Skill。

---

## 7. 单个 Agent 任务契约（每个任务必须满足）

每个任务下发时至少包含：

- **修改范围**：允许改哪些目录/文件，禁止改哪些；
- **明确输入**：参考文件、协议、Mock 数据、设计稿、验收环境；
- **明确输出**：要产生的文件、页面、命令或测试；
- **可执行验收条件**：命令、截图、日志、阈值；
- **公共接口冻结**：不得擅自修改现有接口/事件/Schema；需要改必须先提 ADR；
- **报告要求**：列出修改文件、运行命令、测试结果、截图路径、未完成项。

Agent 完成任务时必须报告：

1. 修改了哪些文件；
2. 执行了哪些构建/测试命令；
3. 测试输出/截图证据；
4. 是否触碰公共接口；
5. 已知限制与下一步。

---

## 8. 自动验收机制（强制配置）

| 工具 | 用途 |
|---|---|
| Playwright | 自动打开页面、点击、断言、截图 |
| 截图对比 | UI 视觉回归，检查布局/可读性 |
| Vitest | 前端单元/组件测试 |
| ESLint + TypeScript | 静态检查与类型检查 |
| Rust `cargo test` | 协议、音频、传输、sidecar 协议 |
| GitHub Actions | 自动构建与测试 |
| 控制台错误采集 | 前端运行时错误/警告 |
| 独立 UI Review Agent | 检查视觉一致性、信息密度、交互、异常态 |

### 每个功能完成后的最低动作

1. 编译/类型检查通过；
2. 单元测试通过；
3. 自动运行应用；
4. 截取关键页面；
5. 检查浏览器/控制台错误；
6. 与上一版截图对比；
7. 输出测试报告；
8. 更新验收记录。

### 建议截图基线

- 连接页/仪表盘；
- 日志页（长日志、筛选、暂停）；
- 音频/语音页（识别中/成功/失败）；
- 设置/异常态；
- 1280×800 与 1920×1080，后续补 1366×768。

### 独立 UI Review

由另一个 Agent 只做评审，不改代码。检查：

- 主题/视觉一致性；
- 文字对比度、字号、信息密度；
- 按钮点击、空态、错误态、加载态；
- 窗口缩放/面板重叠；
- 高频数据下卡顿；
- 页面间回归影响；
- 给出具体、可执行的修改意见。

---

## 9. 阶段计划与门禁

| 阶段 | 内容 | 门禁 |
|---|---|---|
| P0 | 架构、接口、ADR、技能文档 | 文档评审通过，不写代码 |
| P1 | UI 设计系统 + UI 原型（Mock） | PostHog token 落地、主要页面可运行、Playwright 截图、无控制台错误 |
| P2a | 串口传输 | 真实 COM 收数、断连重连、吞吐达标 |
| P2b | TCP/UDP | 协议明确、帧边界/丢包行为可测 |
| P2c | BLE（sidecar） | 已配对/HID 占用/不广播/MTU/Notify/重连通过 |
| P2d | USB（CDC/HID） | 与 BLE 同数据一致；HID 不由上位机解析 |
| P2e | 2.4G | 硬件协议冻结后才启动 |
| P3 | 真实设备联调 | 30 分钟长稳、高吞吐、断连恢复 |
| P4 | 脚本 Worker | 隔离、超时、权限、错误可观测 |
| P5 | AI Provider / 工具调用 | 接口抽象、密钥安全、流式/取消、隐私 |
| P6 | 发布 | 安装包、签名、模型分发、更新与回滚 |

### 通信模块统一验收

- 字节流与协议解析解耦；
- 断开不崩溃，可自动/手动重连；
- 有背压，不无限缓存；
- 有统计：收/发字节、错误、重连次数；
- 有可重复的集成测试；
- 有真实硬件测试记录。

---

## 10. 风险清单

| 风险 | 影响 | 应对 |
|---|---|---|
| Rust BLE 无法复现配对直连/MTU | 核心链路不可用 | 先 spike；必要时 `windows` crate 自定义 |
| ASR 换引擎准确率/延迟退化 | 体验下降 | Python sidecar 过渡；同 WAV 对比后再换 |
| IPC 高频事件拖慢 UI | 卡顿 | 批处理、虚拟列表、有界队列 |
| Python sidecar 打包失败 | 无法分发 | 早期 PyInstaller/Nuitka spike；模型外置 |
| 注入被 UIPI 拦截 | 管理员窗口无字 | 明确提示、管理员运行、剪贴板兜底 |
| 跨平台差异 | macOS/Linux 不可用 | Windows 优先，平台层 trait 隔离 |
| 状态双写 | UI 与后端不一致 | Rust 唯一状态源，前端只读镜像 |
| 公共接口漂移 | 模块返工 | 接口冻结 + ADR + 类型生成 |
| Agent 虚报完成 | 功能不可用 | 自动验收 + 截图 + 独立 Review |
| 模型/依赖体积 | 安装包过大 | 模型按需下载，sidecar 按平台构建 |

---

## 11. 目标目录的 Skill 组织

目标目录本次**只生成 `skills/` 文档**，不生成代码：

```
JnWorkerStream/
  skills/
    README.md
    jnws-orchestrator/SKILL.md
    jnws-architecture/SKILL.md
    jnws-ui-design/SKILL.md
    jnws-ui-prototype/SKILL.md
    jnws-transport-serial/SKILL.md
    jnws-transport-network/SKILL.md
    jnws-transport-ble/SKILL.md
    jnws-transport-usb/SKILL.md
    jnws-transport-24g/SKILL.md
    jnws-python-sidecar/SKILL.md
    jnws-hardware-integration/SKILL.md
    jnws-script-worker/SKILL.md
    jnws-ai-provider/SKILL.md
    jnws-regression-automation/SKILL.md
    jnws-ui-review/SKILL.md
    jnws-release/SKILL.md
```

任务 → Skill 映射以 `JnWorkerStream/skills/README.md` 为准。Agent 开始任务前必须先读对应 Skill，并遵守其中的输入、范围、验收与报告要求。

---

## 12. 当前状态

- 本文与 `JnWorkerStream/skills/` 是**重构前的规划产物**；
- 当前没有在 `JnWorkerStream` 中创建任何源码、脚手架、配置或依赖；
- 当前 `py_example/` 保持可用，作为迁移参考和行为基线；
- 下一步建议：P0 架构文档 + UI 设计系统 + P1 UI Mock，先不接硬件。
