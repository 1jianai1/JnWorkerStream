//! sidecar 错误与 `E_*` 映射。

use std::fmt;

/// `kb-sidecar` 的失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarError {
    /// 进程启动失败。
    Spawn { detail: String },
    /// hello 握手超时。
    HelloTimeout {
        name: String,
        timeout_ms: u64,
        /// sidecar 退出前最后几行 stderr（定位「python 缺依赖 / 脚本路径错」最有效）。
        stderr: Vec<String>,
    },
    /// 帧/JSON 非法。
    Protocol { detail: String },
    /// 协议版本不匹配。
    VersionMismatch { got: u8, want: u8 },
    /// 角色不匹配（ble/asr）。
    RoleMismatch { got: String, want: String },
    /// 进程已退出。
    Exited { code: Option<i32> },
    /// 请求超时。
    Timeout { method: String, timeout_ms: u64 },
    /// 写入 stdin 失败。
    WriteFailed { detail: String },
    /// sidecar 返回 `ok=false`（保留其 `E_*` 码）。
    Remote { code: &'static str, message: String },
    /// 进程未运行。
    NotRunning,
}

impl SidecarError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Spawn { .. } | Self::Exited { .. } | Self::NotRunning => "E_SIDECAR_UNAVAILABLE",
            Self::HelloTimeout { .. } => "E_SIDECAR_UNAVAILABLE",
            Self::Protocol { .. } | Self::VersionMismatch { .. } | Self::RoleMismatch { .. } => {
                "E_SIDECAR_PROTOCOL"
            }
            Self::Timeout { .. } => "E_IO",
            Self::WriteFailed { .. } => "E_WRITE_FAILED",
            Self::Remote { code, .. } => code,
        }
    }

    pub(crate) fn protocol(detail: impl Into<String>) -> Self {
        Self::Protocol {
            detail: detail.into(),
        }
    }

    pub(crate) fn write_failed(detail: impl Into<String>) -> Self {
        Self::WriteFailed {
            detail: detail.into(),
        }
    }

    pub(crate) fn version_mismatch(got: u8, want: u8) -> Self {
        Self::VersionMismatch { got, want }
    }
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { detail } => write!(f, "启动 sidecar 失败：{detail}"),
            Self::HelloTimeout {
                name,
                timeout_ms,
                stderr,
            } => {
                if stderr.is_empty() {
                    write!(f, "sidecar {name} hello 超时（{timeout_ms} ms）")
                } else {
                    write!(
                        f,
                        "sidecar {name} hello 超时（{timeout_ms} ms）；stderr: {}",
                        stderr.join(" | ")
                    )
                }
            }
            Self::Protocol { detail } => write!(f, "sidecar 协议错误：{detail}"),
            Self::VersionMismatch { got, want } => {
                write!(f, "sidecar 协议版本不匹配：{got} != {want}")
            }
            Self::RoleMismatch { got, want } => {
                write!(f, "sidecar 角色不匹配：{got:?} != {want:?}")
            }
            Self::Exited { code } => match code {
                Some(code) => write!(f, "sidecar 已退出（code={code}）"),
                None => write!(f, "sidecar 已退出"),
            },
            Self::Timeout { method, timeout_ms } => {
                write!(f, "sidecar 请求 {method} 超时（{timeout_ms} ms）")
            }
            Self::WriteFailed { detail } => write!(f, "写 sidecar 失败：{detail}"),
            Self::Remote { code, message } => write!(f, "{code}: {message}"),
            Self::NotRunning => write!(f, "sidecar 未运行"),
        }
    }
}

impl std::error::Error for SidecarError {}
