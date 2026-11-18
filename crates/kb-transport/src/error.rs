//! 传输层错误与 `E_*` 错误码映射（`docs/interfaces.md` §1.4）。

use std::fmt;

/// 传输层失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// 没有匹配的端口（或显式端口不存在）。
    PortNotFound {
        detail: String,
        available: Vec<String>,
    },
    /// 端口被占用/权限不足。
    PortBusy { port: String, detail: String },
    /// 链路未连接。
    NotConnected,
    /// 写入失败/不完整。
    WriteFailed { detail: String },
    /// 参数非法（例如过滤条件匹配到多个端口）。
    InvalidArg { detail: String },
    /// 当前阶段未实现。
    NotImplemented { what: &'static str },
    /// 其它 IO 失败。
    Io { detail: String },
    /// BLE 未发现设备。
    DeviceNotFound { detail: String },
    /// BLE 连接超时。
    ConnectTimeout { detail: String },
    /// 用户主动取消（例如连接中点了「终止连接」）。
    Cancelled,
    /// 单帧超过链路 payload 上限（ADR-011）。
    PayloadTooLarge { len: usize, max: usize },
    /// sidecar 不可用/已退出。
    Sidecar { detail: String },
    /// sidecar 协议不匹配/帧非法。
    Protocol { detail: String },
}

impl TransportError {
    /// 映射到冻结的 `E_*` 错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::PortNotFound { .. } => "E_PORT_NOT_FOUND",
            Self::PortBusy { .. } => "E_PORT_BUSY",
            Self::NotConnected => "E_NOT_CONNECTED",
            Self::WriteFailed { .. } => "E_WRITE_FAILED",
            Self::InvalidArg { .. } => "E_INVALID_ARG",
            Self::NotImplemented { .. } => "E_NOT_IMPLEMENTED",
            Self::Io { .. } => "E_IO",
            Self::DeviceNotFound { .. } => "E_BLE_DEVICE_NOT_FOUND",
            Self::ConnectTimeout { .. } => "E_BLE_CONNECT_TIMEOUT",
            Self::Cancelled => "E_CANCELLED",
            Self::PayloadTooLarge { .. } => "E_PAYLOAD_TOO_LARGE",
            Self::Sidecar { .. } => "E_SIDECAR_UNAVAILABLE",
            Self::Protocol { .. } => "E_SIDECAR_PROTOCOL",
        }
    }

    /// 同一操作能否直接重试。
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::PortBusy { .. }
                | Self::WriteFailed { .. }
                | Self::PortNotFound { .. }
                | Self::DeviceNotFound { .. }
                | Self::ConnectTimeout { .. }
                | Self::Sidecar { .. }
        )
    }

    /// 用户可读消息（不含错误码前缀）。
    pub fn message(&self) -> String {
        match self {
            Self::PortNotFound { detail, available } => {
                if available.is_empty() {
                    format!("未找到匹配的 USB CDC 端口：{detail}")
                } else {
                    format!(
                        "未找到匹配的 USB CDC 端口：{detail}；当前端口: {}",
                        available.join(", ")
                    )
                }
            }
            Self::PortBusy { port, detail } => format!("端口 {port} 不可用：{detail}"),
            Self::NotConnected => "链路未连接".to_string(),
            Self::WriteFailed { detail } => format!("写入失败：{detail}"),
            Self::InvalidArg { detail } => detail.clone(),
            Self::NotImplemented { what } => format!("{what} 在当前阶段未实现"),
            Self::Io { detail } => detail.clone(),
            Self::DeviceNotFound { detail } => format!("未发现 BLE 设备：{detail}"),
            Self::ConnectTimeout { detail } => format!("BLE 连接超时：{detail}"),
            Self::Cancelled => "操作已取消".to_string(),
            Self::PayloadTooLarge { len, max } => {
                format!("单帧 {len} B 超过链路上限 {max} B（BLE 不分片）")
            }
            Self::Sidecar { detail } => format!("BLE sidecar 不可用：{detail}"),
            Self::Protocol { detail } => format!("sidecar 协议错误：{detail}"),
        }
    }

    pub(crate) fn io(detail: impl Into<String>) -> Self {
        Self::Io {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for TransportError {}
