//! 平台抽象：只暴露“串口字节通道”，不泄漏 Win32 类型。

use std::fmt;

use crate::usb::UsbPortInfo;

/// 平台层错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// 读超时（无数据），不是故障。
    Timeout,
    /// 句柄已被关闭/操作被取消。
    Cancelled,
    /// 设备被拔出。
    DeviceRemoved,
    /// 其它 Win32 失败。
    Io { code: u32, message: String },
}

impl PortError {
    pub fn is_fatal(&self) -> bool {
        !matches!(self, Self::Timeout)
    }

    pub fn code(&self) -> u32 {
        match self {
            Self::Io { code, .. } => *code,
            _ => 0,
        }
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "读超时"),
            Self::Cancelled => write!(f, "操作已取消"),
            Self::DeviceRemoved => write!(f, "设备已移除"),
            Self::Io { code, message } => write!(f, "Win32 错误 {code}: {message}"),
        }
    }
}

/// 串口字节通道。
///
/// 读与写可以并发（Win32 对同一串口句柄允许一个读线程 + 一个写线程），
/// 因此方法接收 `&self`，实现内部自行处理状态。
pub trait SerialPort: Send + Sync + fmt::Debug {
    /// 阻塞读，最多等待实现的读超时（USB CDC 为 50 ms）；超时返回 [`PortError::Timeout`]。
    fn read(&self, buf: &mut [u8]) -> Result<usize, PortError>;
    /// 写入全部字节（处理部分写）并 flush。
    fn write_all(&self, data: &[u8]) -> Result<(), PortError>;
    /// 取消挂起的读，用于关闭前解除阻塞。
    fn cancel(&self);
    /// 关闭句柄（幂等）。
    fn close(&self);
}

/// 枚举当前系统串口。
pub fn list_ports() -> Result<Vec<UsbPortInfo>, crate::TransportError> {
    platform_impl::list_ports()
}

/// 打开一个串口设备（例如 `COM3`）。
pub fn open_port(device: &str) -> Result<Box<dyn SerialPort>, crate::TransportError> {
    platform_impl::open_port(device)
}

#[cfg(windows)]
mod platform_impl {
    pub use super::win::{list_ports, open_port};
}

#[cfg(windows)]
pub mod win;

#[cfg(not(windows))]
mod platform_impl {
    use crate::error::TransportError;
    use crate::usb::UsbPortInfo;

    use super::SerialPort;

    pub fn list_ports() -> Result<Vec<UsbPortInfo>, TransportError> {
        Ok(Vec::new())
    }

    pub fn open_port(_device: &str) -> Result<Box<dyn SerialPort>, TransportError> {
        Err(TransportError::NotImplemented {
            what: "非 Windows 平台的 USB CDC",
        })
    }
}
