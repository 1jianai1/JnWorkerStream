//! Windows USB CDC（虚拟串口）实现。
//!
//! 读超时 50 ms、写超时 2 s、读块 4 KiB 由上层决定；写入循环处理部分写并 flush。
//! 句柄关闭前先 `CancelIoEx` 取消挂起的 `ReadFile`，避免读线程卡死。

mod enumerate;
mod ffi;

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::TransportError;
use crate::platform::{PortError, SerialPort};

pub use enumerate::list_ports;

/// 打开 `device`（如 `COM3`）。
pub fn open_port(device: &str) -> Result<Box<dyn SerialPort>, TransportError> {
    Ok(Box::new(WinSerial::open(device)?))
}

/// 原生串口句柄包装。
#[derive(Debug)]
pub struct WinSerial {
    handle: isize,
    closed: AtomicBool,
}

impl WinSerial {
    fn open(device: &str) -> Result<Self, TransportError> {
        let path = if device.starts_with("\\\\") {
            device.to_string()
        } else {
            format!("\\\\.\\{device}")
        };
        let wide = ffi::wide(&path);
        let handle = unsafe {
            ffi::CreateFileW(
                wide.as_ptr(),
                ffi::GENERIC_READ | ffi::GENERIC_WRITE,
                0,
                null(),
                ffi::OPEN_EXISTING,
                ffi::FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if handle == ffi::INVALID_HANDLE_VALUE {
            return Err(map_open_error(device));
        }

        let serial = Self {
            handle,
            closed: AtomicBool::new(false),
        };
        if let Err(error) = serial.configure() {
            serial.close();
            return Err(error);
        }
        Ok(serial)
    }

    fn configure(&self) -> Result<(), TransportError> {
        let mut dcb = ffi::Dcb::default();
        let got = unsafe { ffi::GetCommState(self.handle, &mut dcb) };
        if got == 0 {
            // CDC 不依赖波特率；沿用默认值，但仍尝试写入一次。
            dcb = ffi::Dcb::default();
        }
        dcb.length = std::mem::size_of::<ffi::Dcb>() as u32;
        dcb.baud_rate = 115_200;
        dcb.byte_size = 8;
        dcb.parity = 0;
        dcb.stop_bits = 0;

        if unsafe { ffi::SetCommState(self.handle, &dcb) } == 0 {
            let code = unsafe { ffi::GetLastError() };
            return Err(TransportError::io(format!(
                "配置串口失败（SetCommState {code}: {}）",
                ffi::last_error_message(code)
            )));
        }

        let timeouts = ffi::CommTimeouts {
            // 特殊组合：有数据立即返回，否则最多等 50 ms。
            read_interval_timeout: ffi::MAXDWORD,
            read_total_multiplier: ffi::MAXDWORD,
            read_total_constant: 50,
            write_total_multiplier: 0,
            write_total_constant: 2_000,
        };
        if unsafe { ffi::SetCommTimeouts(self.handle, &timeouts) } == 0 {
            let code = unsafe { ffi::GetLastError() };
            return Err(TransportError::io(format!(
                "配置串口超时失败（SetCommTimeouts {code}: {}）",
                ffi::last_error_message(code)
            )));
        }

        // 清掉上电/上次残留的缓冲，避免半帧脏数据。
        unsafe {
            ffi::PurgeComm(self.handle, ffi::PURGE_RXCLEAR | ffi::PURGE_TXCLEAR);
        }
        Ok(())
    }

    fn map_io_error(&self) -> PortError {
        if self.closed.load(Ordering::Acquire) {
            return PortError::Cancelled;
        }
        let code = unsafe { ffi::GetLastError() };
        match code {
            ffi::ERROR_OPERATION_ABORTED | ffi::ERROR_INVALID_HANDLE => PortError::Cancelled,
            ffi::ERROR_DEVICE_NOT_CONNECTED
            | ffi::ERROR_DEVICE_REMOVED
            | ffi::ERROR_FILE_NOT_FOUND
            | ffi::ERROR_PATH_NOT_FOUND => PortError::DeviceRemoved,
            other => PortError::Io {
                code: other,
                message: ffi::last_error_message(other),
            },
        }
    }
}

impl SerialPort for WinSerial {
    fn read(&self, buf: &mut [u8]) -> Result<usize, PortError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PortError::Cancelled);
        }
        if buf.is_empty() {
            return Err(PortError::Timeout);
        }
        let mut read: u32 = 0;
        let ok = unsafe {
            ffi::ReadFile(
                self.handle,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(self.map_io_error());
        }
        if read == 0 {
            return Err(PortError::Timeout);
        }
        Ok(read as usize)
    }

    fn write_all(&self, data: &[u8]) -> Result<(), PortError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PortError::Cancelled);
        }
        let mut written = 0usize;
        while written < data.len() {
            let mut count: u32 = 0;
            let ok = unsafe {
                ffi::WriteFile(
                    self.handle,
                    data[written..].as_ptr(),
                    (data.len() - written) as u32,
                    &mut count,
                    null_mut(),
                )
            };
            if ok == 0 {
                return Err(self.map_io_error());
            }
            if count == 0 {
                return Err(PortError::Io {
                    code: 0,
                    message: "串口写入 0 字节".to_string(),
                });
            }
            written += count as usize;
        }
        if unsafe { ffi::FlushFileBuffers(self.handle) } == 0 {
            return Err(self.map_io_error());
        }
        Ok(())
    }

    fn cancel(&self) {
        unsafe {
            ffi::CancelIoEx(self.handle, null());
        }
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            unsafe {
                ffi::CancelIoEx(self.handle, null());
                ffi::CloseHandle(self.handle);
            }
        }
    }
}

impl Drop for WinSerial {
    fn drop(&mut self) {
        self.close();
    }
}

fn map_open_error(device: &str) -> TransportError {
    let code = unsafe { ffi::GetLastError() };
    let message = ffi::last_error_message(code);
    match code {
        ffi::ERROR_FILE_NOT_FOUND | ffi::ERROR_PATH_NOT_FOUND => TransportError::PortNotFound {
            detail: format!("{device} 不存在（{message}）"),
            available: Vec::new(),
        },
        ffi::ERROR_ACCESS_DENIED | ffi::ERROR_SHARING_VIOLATION => TransportError::PortBusy {
            port: device.to_string(),
            detail: message,
        },
        other => TransportError::io(format!("打开 {device} 失败（{other}: {message}）")),
    }
}
