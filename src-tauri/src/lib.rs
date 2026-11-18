//! Tauri 壳（P2c）：只做命令转发与事件 emit，业务全在 `kb-app`。
//!
//! 命令名在 Rust 侧是下划线形式（Tauri 不允许点号标识符），TS 侧按
//! `docs/interfaces.md` §2.1 的冻结名做一层薄映射（`link.connect` → `invoke("link_connect")`）。
//!
//! 注意：本机拉不到 Tauri 依赖，这个文件**未在本机编译验证**；
//! 可验证的部分（状态机、快照、事件载荷）都在 `crates/kb-app` 且有测试。

use std::sync::Mutex;

use kb_app::{Core, Snapshot};
use kb_transport::{BleCfg, TransportError};
use serde::Serialize;
use tauri::{Emitter, Manager};

/// 命令错误（`docs/interfaces.md` §1.4）。
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn lock() -> Self {
        Self::new("E_IO", "状态锁已损坏")
    }
}

impl From<TransportError> for CommandError {
    fn from(error: TransportError) -> Self {
        Self {
            code: error.code(),
            message: error.message(),
        }
    }
}

impl From<kb_sidecar::SidecarError> for CommandError {
    fn from(error: kb_sidecar::SidecarError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

struct AppState {
    core: Mutex<Core<kb_inject::Injector>>,
    /// 与 `Core` 共享的取消令牌；置位不需要拿 Core 锁，所以「终止连接」不会被连接流程阻塞。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Default for AppState {
    fn default() -> Self {
        let core = Core::new(kb_transport::SidecarLaunch::default());
        let cancel = core.cancel_handle();
        Self {
            core: Mutex::new(core),
            cancel,
        }
    }
}

#[tauri::command]
fn app_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, CommandError> {
    let core = state.core.lock().map_err(|_| CommandError::lock())?;
    Ok(core.snapshot())
}

#[tauri::command]
fn app_ping() -> serde_json::Value {
    serde_json::json!({ "pong": true, "ts_ms": now_ms() })
}

/// `link.connect`：BLE 参数逐字段传入（`kb_transport::BleCfg` 不带 serde 依赖）。
///
/// 用 `async` + `spawn_blocking`：连接是阻塞调用（最长 `Core::DEFAULT_CONNECT_TIMEOUT` = 10 s），
/// 放在主线程会把整个 WebView 卡住（点击无响应）。同步命令在 Tauri 里默认跑在主线程。
#[tauri::command]
async fn link_connect(
    address: Option<String>,
    name: Option<String>,
    paired_only: Option<bool>,
    scan_timeout_ms: Option<u32>,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, CommandError> {
    let cfg = BleCfg {
        address,
        name,
        paired_only: paired_only.unwrap_or(true),
        scan_timeout_ms: scan_timeout_ms.unwrap_or(6_000),
    };
    let handle = app.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        let mut core = state.core.lock().map_err(|_| CommandError::lock())?;
        core.connect_ble(cfg)
            .map(|link| {
                serde_json::json!({
                    "kind": link.kind.as_str(),
                    "state": link.state,
                    "address": link.address,
                    "mtu": link.mtu,
                    "max_payload": link.max_payload,
                    "opened_at_ms": link.opened_at_ms,
                })
            })
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError::new("E_IO", format!("连接线程异常：{error}")))?;
    joined
}

/// `link.cancel_connect`：连接中由 UI 调用，置位取消令牌。
///
/// 物理连接无法立刻中断，因此这里**立即返回**；连接流程在本次请求结束（≤10 s）后自行清理，
/// 并以 `E_CANCELLED` 结束，UI 先切到「已取消」不需要等它。
#[tauri::command]
fn link_cancel_connect(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state
        .cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn link_disconnect(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state
        .core
        .lock()
        .map_err(|_| CommandError::lock())?
        .disconnect()?;
    Ok(())
}

#[tauri::command]
fn link_reconnect_policy(auto: bool, state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state
        .core
        .lock()
        .map_err(|_| CommandError::lock())?
        .set_auto_reconnect(auto);
    Ok(())
}

/// `link.retry_now`：把下一次退避提前到当前时刻（UI 的「立即重试」）。
#[tauri::command]
fn link_retry(state: tauri::State<'_, AppState>) -> Result<bool, CommandError> {
    Ok(state
        .core
        .lock()
        .map_err(|_| CommandError::lock())?
        .retry_now())
}

#[tauri::command]
fn transport_stats(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, CommandError> {
    let snapshot = state
        .core
        .lock()
        .map_err(|_| CommandError::lock())?
        .snapshot();
    Ok(serde_json::json!({
        "rx_bytes": snapshot.transport.rx_bytes,
        "tx_bytes": snapshot.transport.tx_bytes,
        "errors": snapshot.transport.errors,
        "reconnects": snapshot.transport.reconnects,
        "dropped": snapshot.transport.dropped,
        "last_error": snapshot.transport.last_error,
    }))
}

/// `voice.set_enabled`：`enabled=false` 只卸载语音链路，不影响收数。
///
/// 启动 ASR sidecar + 加载模型 + 预热最长各 60 s。这些动作**在 Core 锁之外**
/// 用 `spawn_blocking` 做，否则同步命令会卡住主线程、整个窗口无响应（模型 3–6 s）。
#[tauri::command]
async fn voice_set_enabled(
    enabled: bool,
    model_path: Option<String>,
    python: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), CommandError> {
    if !enabled {
        let state = app.state::<AppState>();
        state
            .core
            .lock()
            .map_err(|_| CommandError::lock())?
            .detach_voice();
        return Ok(());
    }
    {
        let state = app.state::<AppState>();
        let core = state.core.lock().map_err(|_| CommandError::lock())?;
        if core.voice_enabled() {
            return Ok(());
        }
    }

    let program = python
        .or_else(|| std::env::var("JNWS_PYTHON").ok())
        .unwrap_or_else(|| "python".to_string());
    let script = asr_script();
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = kb_voice::AsrClient::start(kb_voice::AsrLaunch::new(program, script))?;
        client.load(model_path.as_deref(), 0)?;
        client.warmup()?;

        let state = app.state::<AppState>();
        let mut core = state.core.lock().map_err(|_| CommandError::lock())?;
        // 并发点两次「启用」时只挂一条链路；后完成的 client 在这里被 drop。
        if core.voice_enabled() {
            return Ok(());
        }
        core.attach_voice(client, kb_inject::Injector::new());
        Ok(())
    })
    .await
    .map_err(|error| CommandError::new("E_IO", format!("语音线程异常：{error}")))?
}

fn asr_script() -> String {
    std::env::var("JNWS_ASR_SCRIPT").unwrap_or_else(|_| {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.push("sidecars");
        path.push("asr");
        path.push("main.py");
        path.display().to_string()
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 10 ms 轮询核心，把事件按冻结名字 emit 给前端。
fn spawn_poller(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("kb-app-poller".to_string())
        .spawn(move || loop {
            let events = {
                let state = app.state::<AppState>();
                // guard 先于 `state` 声明，drop 顺序保证它在 `state` 之前释放。
                let mut core = match state.core.lock() {
                    Ok(guard) => guard,
                    Err(_) => break,
                };
                core.poll()
            };
            for event in events {
                // Tauri 会校验事件名（只允许字母数字和 `- / : _`）；失败必须可见，
                // 不能静默吞掉，否则前端只会收到 1 s 一次的快照、看起来“没有动作”。
                if let Err(error) = app.emit(event.name(), event.payload()) {
                    eprintln!("emit {} 失败：{error}", event.name());
                }
            }
            std::thread::sleep(kb_app::POLL_INTERVAL);
        })
        .expect("启动 poller 失败");
}

/// WebView2 用户数据目录：默认放在**可执行文件旁边**（`cargo run` 时即
/// `target\debug\webview2-data`），不写 C 盘的 `%LOCALAPPDATA%`。
///
/// 原因：本机 `%LOCALAPPDATA%` 下建目录会被拒绝（`os error 5`），而工程目录可写；
/// 显式指定也便于排查 “0x800700AA 资源在使用中”（数据目录被占住时会上报）。
///
/// 优先级：`JNWS_WEBVIEW_DATA_DIR` → `<exe 所在目录>/webview2-data` → 系统临时目录。
fn webview_data_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("JNWS_WEBVIEW_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            return parent.join("webview2-data");
        }
    }
    std::env::temp_dir().join("jnws-webview2")
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            app_snapshot,
            app_ping,
            link_connect,
            link_disconnect,
            link_reconnect_policy,
            link_retry,
            link_cancel_connect,
            transport_stats,
            voice_set_enabled,
        ])
        .setup(|app| {
            #[cfg(windows)]
            {
                let integrity = kb_inject::current_integrity();
                eprintln!(
                    "[app] integrity={:?} ({})",
                    integrity,
                    kb_inject::integrity_label(integrity)
                );
                if integrity == Some(0x1000) {
                    eprintln!(
                        "[app] WARN: 当前进程是 Low 完整性（受限终端/沙箱），\
                         无法向普通应用注入；请从普通 PowerShell 或管理员 PowerShell 启动"
                    );
                }
            }
            let data_dir = webview_data_dir();
            if let Err(error) = std::fs::create_dir_all(&data_dir) {
                eprintln!("创建 WebView2 数据目录失败：{data_dir:?} {error}");
            }
            let _window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("JnWorkerStream")
            .inner_size(1280.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .center()
            .data_directory(data_dir.clone())
            .build()
            .map_err(|error| {
                eprintln!("创建主窗口失败（data_dir={data_dir:?}）：{error:?}");
                error
            })?;
            spawn_poller(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running JnWorkerStream");
}
