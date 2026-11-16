//! sidecar 配置。

use std::path::PathBuf;
use std::time::Duration;

/// sidecar 启动配置。
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// 日志/ID 前缀，例如 `ble`。
    pub name: String,
    /// 期望的 hello `role`。
    pub role: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub hello_timeout: Duration,
    pub request_timeout: Duration,
    pub event_capacity: usize,
    pub stderr_capacity: usize,
}

impl SidecarConfig {
    /// BLE sidecar 的开发默认值（用 `python <script>` 启动）。
    pub fn ble(program: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            name: "ble".to_string(),
            role: "ble".to_string(),
            program: program.into(),
            args: vec![script.into()],
            cwd: None,
            env: Vec::new(),
            hello_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            event_capacity: 512,
            stderr_capacity: 200,
        }
    }

    /// ASR sidecar 的开发默认值（用 `python <script>` 启动）。
    pub fn asr(program: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            name: "asr".to_string(),
            role: "asr".to_string(),
            program: program.into(),
            args: vec![script.into()],
            cwd: None,
            env: Vec::new(),
            hello_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(60),
            event_capacity: 64,
            stderr_capacity: 200,
        }
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}
