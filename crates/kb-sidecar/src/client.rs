//! 子进程生命周期 + 请求/响应关联。

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::codec::{self, SidecarFrame, PROTO_VERSION};
use crate::config::SidecarConfig;
use crate::error::SidecarError;
use crate::shared::{Shared, SidecarEvent, SidecarStatus};

/// sidecar 客户端。
#[derive(Debug)]
pub struct SidecarClient {
    config: SidecarConfig,
    shared: Arc<Shared>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    reader: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
    next_id: AtomicU64,
}

impl SidecarClient {
    /// 启动子进程并等待 hello。
    pub fn start(config: SidecarConfig) -> Result<Self, SidecarError> {
        let mut command = Command::new(&config.program);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &config.env {
            command.env(key, value);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn().map_err(|error| SidecarError::Spawn {
            detail: format!("{}: {error}", config.program),
        })?;
        let stdin = child.stdin.take().ok_or_else(|| SidecarError::Spawn {
            detail: "无法获取 stdin".to_string(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| SidecarError::Spawn {
            detail: "无法获取 stdout".to_string(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| SidecarError::Spawn {
            detail: "无法获取 stderr".to_string(),
        })?;

        let shared = Arc::new(Shared::new(config.event_capacity, config.stderr_capacity));
        let (hello_tx, hello_rx) = mpsc::channel();
        let reader = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name(format!("{}-stdout", config.name))
                .spawn(move || reader_loop(shared, stdout, hello_tx))
                .map_err(|error| SidecarError::Spawn {
                    detail: format!("启动 stdout 线程失败: {error}"),
                })?
        };
        let stderr_thread = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name(format!("{}-stderr", config.name))
                .spawn(move || stderr_loop(shared, stderr))
                .map_err(|error| SidecarError::Spawn {
                    detail: format!("启动 stderr 线程失败: {error}"),
                })?
        };

        let hello = match hello_rx.recv_timeout(config.hello_timeout) {
            Ok(frame) => frame,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SidecarError::HelloTimeout {
                    name: config.name.clone(),
                    timeout_ms: config.hello_timeout.as_millis() as u64,
                });
            }
        };

        let version = hello.get_u64("proto_version").unwrap_or(0) as u8;
        if version != PROTO_VERSION {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SidecarError::version_mismatch(version, PROTO_VERSION));
        }
        let role = hello.get_str("role").unwrap_or("").to_string();
        if role != config.role {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SidecarError::RoleMismatch {
                got: role,
                want: config.role.clone(),
            });
        }

        *shared.lock_status() = SidecarStatus::Ready;
        Ok(Self {
            config,
            shared,
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(Some(child)),
            reader: Some(reader),
            stderr: Some(stderr_thread),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn config(&self) -> &SidecarConfig {
        &self.config
    }

    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::Acquire)
    }

    pub fn status(&self) -> SidecarStatus {
        self.shared.lock_status().clone()
    }

    pub fn dropped_events(&self) -> u64 {
        self.shared.lock_events().dropped
    }

    /// 最近若干行 stderr（诊断用）。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.shared
            .stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// 发请求并等待响应（不看 `ok`）。
    pub fn request(
        &self,
        method: &str,
        params: Map<String, Value>,
    ) -> Result<SidecarFrame, SidecarError> {
        self.request_timeout(method, params, self.config.request_timeout)
    }

    /// 发请求并等待响应；超时返回 `Timeout`。
    pub fn request_timeout(
        &self,
        method: &str,
        params: Map<String, Value>,
        timeout: Duration,
    ) -> Result<SidecarFrame, SidecarError> {
        self.request_with_body(method, params, &[], timeout)
    }

    /// 发带二进制 body 的请求（`asr.transcribe` 的 PCM 走这里）并等待响应。
    pub fn request_with_body(
        &self,
        method: &str,
        params: Map<String, Value>,
        body: &[u8],
        timeout: Duration,
    ) -> Result<SidecarFrame, SidecarError> {
        if !self.is_alive() {
            return Err(SidecarError::Exited {
                code: self.exit_code(),
            });
        }
        let id = format!(
            "{}-{}",
            self.config.name,
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let mut header = Map::new();
        header.insert("kind".to_string(), json!("request"));
        header.insert("id".to_string(), json!(id));
        header.insert("ts_ms".to_string(), json!(now_ms()));
        header.insert("method".to_string(), json!(method));
        for (key, value) in params {
            header.insert(key, value);
        }

        let (sender, receiver) = mpsc::channel();
        self.shared.lock_pending().insert(id.clone(), sender);
        let write_result = {
            let mut guard = self
                .stdin
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match guard.as_mut() {
                Some(stdin) => codec::write_frame(stdin, &header, body),
                None => Err(SidecarError::NotRunning),
            }
        };
        if let Err(error) = write_result {
            self.shared.lock_pending().remove(&id);
            return Err(error);
        }

        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.shared.lock_pending().remove(&id);
                Err(SidecarError::Timeout {
                    method: method.to_string(),
                    timeout_ms: timeout.as_millis() as u64,
                })
            }
            Err(RecvTimeoutError::Disconnected) => Err(SidecarError::Exited {
                code: self.exit_code(),
            }),
        }
    }

    /// 发请求，`ok=false` 时返回 `Remote` 错误。
    pub fn call(
        &self,
        method: &str,
        params: Map<String, Value>,
    ) -> Result<SidecarFrame, SidecarError> {
        let frame = self.request(method, params)?;
        if frame.ok() {
            Ok(frame)
        } else {
            Err(SidecarError::Remote {
                code: static_code(frame.error_code().unwrap_or("E_SIDECAR_PROTOCOL")),
                message: frame
                    .error_message()
                    .unwrap_or("sidecar 返回失败")
                    .to_string(),
            })
        }
    }

    /// 阻塞等待下一个事件。
    pub fn next_event(&self, timeout: Duration) -> Option<SidecarEvent> {
        let mut queue = self.shared.lock_events();
        if queue.items.is_empty() {
            let (guard, result) = self
                .shared
                .cv
                .wait_timeout(queue, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue = guard;
            if result.timed_out() && queue.items.is_empty() {
                return None;
            }
        }
        queue.items.pop_front()
    }

    /// 取走当前全部事件。
    pub fn try_events(&self) -> Vec<SidecarEvent> {
        let mut queue = self.shared.lock_events();
        queue.items.drain(..).collect()
    }

    /// 进程退出码（未退出时为 `None`）。
    pub fn exit_code(&self) -> Option<i32> {
        let code = {
            let mut guard = self
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
                .and_then(|status| status.code())
        };
        if code.is_some() {
            *self
                .shared
                .last_exit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = code;
        }
        code.or_else(|| {
            *self
                .shared
                .last_exit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        })
    }

    /// 停止：shutdown → 关闭 stdin → 2 s 后 kill。
    pub fn stop(&mut self) {
        if self.is_alive() {
            let _ = self.request_timeout("shutdown", Map::new(), Duration::from_millis(500));
        }
        let _ = self
            .stdin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();

        if let Some(mut child) = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        *self
                            .shared
                            .last_exit
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = status.code();
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        if let Ok(status) = child.wait() {
                            *self
                                .shared
                                .last_exit
                                .lock()
                                .unwrap_or_else(|p| p.into_inner()) = status.code();
                        }
                        break;
                    }
                }
            }
        }

        self.shared.alive.store(false, Ordering::Release);
        self.shared.fail(SidecarError::Exited { code: None });
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr.take() {
            let _ = handle.join();
        }
    }

    /// 停止后按原配置重启。
    pub fn restart(&mut self) -> Result<(), SidecarError> {
        let config = self.config.clone();
        self.stop();
        let new = Self::start(config)?;
        *self = new;
        Ok(())
    }
}

impl Drop for SidecarClient {
    fn drop(&mut self) {
        self.stop();
    }
}

fn reader_loop(shared: Arc<Shared>, stdout: ChildStdout, hello: Sender<SidecarFrame>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match codec::read_frame(&mut reader) {
            Ok(Some(frame)) => match frame.kind() {
                Some("hello") => {
                    let _ = hello.send(frame);
                }
                Some("response") => {
                    if let Some(id) = frame.id() {
                        if let Some(sender) = shared.lock_pending().remove(id) {
                            let _ = sender.send(Ok(frame));
                        }
                    }
                }
                _ => shared.push_event(SidecarEvent {
                    header: frame.header,
                    body: frame.body,
                }),
            },
            Ok(None) => {
                shared.fail(SidecarError::Exited { code: None });
                break;
            }
            Err(error) => {
                shared.fail(error);
                break;
            }
        }
    }
}

fn stderr_loop(shared: Arc<Shared>, stderr: ChildStderr) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        shared.push_stderr(line);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 把 sidecar 返回的错误码映射为静态字符串（未知码归为协议错误）。
fn static_code(code: &str) -> &'static str {
    match code {
        "E_NOT_CONNECTED" => "E_NOT_CONNECTED",
        "E_BLE_DEVICE_NOT_FOUND" => "E_BLE_DEVICE_NOT_FOUND",
        "E_BLE_CONNECT_TIMEOUT" => "E_BLE_CONNECT_TIMEOUT",
        "E_PAYLOAD_TOO_LARGE" => "E_PAYLOAD_TOO_LARGE",
        "E_WRITE_FAILED" => "E_WRITE_FAILED",
        "E_NOT_IMPLEMENTED" => "E_NOT_IMPLEMENTED",
        "E_INVALID_ARG" => "E_INVALID_ARG",
        "E_IO" => "E_IO",
        "E_ASR_FAILED" => "E_ASR_FAILED",
        "E_ASR_TIMEOUT" => "E_ASR_TIMEOUT",
        "E_ASR_MODEL_MISSING" => "E_ASR_MODEL_MISSING",
        "E_SIDECAR_PROTOCOL" => "E_SIDECAR_PROTOCOL",
        _ => "E_SIDECAR_PROTOCOL",
    }
}
