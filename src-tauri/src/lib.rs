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
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            core: Mutex::new(Core::new(kb_transport::SidecarLaunch::default())),
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
#[tauri::command]
fn link_connect(
    address: Option<String>,
    name: Option<String>,
    paired_only: Option<bool>,
    scan_timeout_ms: Option<u32>,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, CommandError> {
    let cfg = BleCfg {
        address,
        name,
        paired_only: paired_only.unwrap_or(true),
        scan_timeout_ms: scan_timeout_ms.unwrap_or(6_000),
    };
    let link = state
        .core
        .lock()
        .map_err(|_| CommandError::lock())?
        .connect_ble(cfg)?;
    Ok(serde_json::json!({
        "kind": link.kind.as_str(),
        "state": link.state,
        "address": link.address,
        "mtu": link.mtu,
        "max_payload": link.max_payload,
        "opened_at_ms": link.opened_at_ms,
    }))
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
#[tauri::command]
fn voice_set_enabled(
    enabled: bool,
    model_path: Option<String>,
    python: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    let mut core = state.core.lock().map_err(|_| CommandError::lock())?;
    if !enabled {
        core.detach_voice();
        return Ok(());
    }
    if core.voice_enabled() {
        return Ok(());
    }
    let program = python
        .or_else(|| std::env::var("JNWS_PYTHON").ok())
        .unwrap_or_else(|| "python".to_string());
    let script = asr_script();
    let mut client = kb_voice::AsrClient::start(kb_voice::AsrLaunch::new(program, script))?;
    client.load(model_path.as_deref(), 0)?;
    client.warmup()?;
    core.attach_voice(client, kb_inject::Injector::new());
    Ok(())
}

fn asr_script() -> String {
    std::env::var("JNWS_ASR_SCRIPT").unwrap_or_else(|_| {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.push("sidecars")
            .push("asr")
            .push("main.py");
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
                match state.core.lock() {
                    Ok(mut core) => core.poll(),
                    Err(_) => break,
                }
            };
            for event in events {
                let _ = app.emit(event.name(), event.payload());
            }
            std::thread::sleep(kb_app::POLL_INTERVAL);
        })
        .expect("启动 poller 失败");
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
            transport_stats,
            voice_set_enabled,
        ])
        .setup(|app| {
            spawn_poller(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running JnWorkerStream");
}
