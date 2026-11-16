//! USB CDC 传输：端口解析、读线程、写入、断连事件与统计。

#[cfg(test)]
mod tests;
mod types;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::counters::Counters;
use crate::error::TransportError;
use crate::event_queue::EventQueue;
use crate::platform::{self, PortError, SerialPort};
use crate::reconnect::ReconnectPolicy;
use crate::{
    now_ms, DisconnectReason, LinkInfo, Transport, TransportCfg, TransportEvent, TransportKind,
    TransportStats, MAX_PAYLOAD_USB, MTU_USB,
};

pub use types::{UsbCfg, UsbPortInfo};

/// 读线程一次最多读 4 KiB（`docs/architecture.md` §3.2）。
const READ_CHUNK: usize = 4 * 1024;
/// 事件队列容量；满时丢最旧并累加 `dropped`。
const EVENT_QUEUE_CAPACITY: usize = 256;

/// 枚举当前系统串口（已按 `device` 排序）。
pub fn list_ports() -> Result<Vec<UsbPortInfo>, TransportError> {
    platform::list_ports()
}

/// USB CDC 传输实现。
#[derive(Debug)]
pub struct UsbTransport {
    cfg: UsbCfg,
    link: Option<LinkInfo>,
    port: Option<Arc<dyn SerialPort>>,
    reader: Option<JoinHandle<()>>,
    queue: Arc<EventQueue>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<Option<String>>>,
    policy: ReconnectPolicy,
    opened_before: bool,
}

impl Default for UsbTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbTransport {
    pub fn new() -> Self {
        Self {
            cfg: UsbCfg::default(),
            link: None,
            port: None,
            reader: None,
            queue: Arc::new(EventQueue::new(EVENT_QUEUE_CAPACITY)),
            running: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            counters: Arc::new(Counters::default()),
            last_error: Arc::new(Mutex::new(None)),
            policy: ReconnectPolicy::default(),
            opened_before: false,
        }
    }

    /// 当前连接态。
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// 当前链路信息。
    pub fn link(&self) -> Option<&LinkInfo> {
        self.link.as_ref()
    }

    /// 重连策略（自动重连由应用层按 [`ReconnectPolicy::delay_ms`] 驱动）。
    pub fn reconnect_policy(&self) -> &ReconnectPolicy {
        &self.policy
    }

    pub fn set_reconnect_policy(&mut self, policy: ReconnectPolicy) {
        self.policy = policy;
    }

    /// 传输统计快照。
    pub fn stats(&self) -> TransportStats {
        TransportStats {
            rx_bytes: self.counters.rx(),
            tx_bytes: self.counters.tx(),
            errors: self.counters.errors(),
            reconnects: self.counters.reconnects(),
            dropped: self.queue.dropped(),
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        }
    }

    /// 打开 USB CDC（`port` 优先，否则按过滤条件唯一匹配）。
    pub fn open_usb(&mut self) -> Result<LinkInfo, TransportError> {
        if self.is_connected() {
            if let Some(link) = &self.link {
                return Ok(link.clone());
            }
        }
        let address = resolve_port(&self.cfg)?;
        let port = platform::open_port(&address)?;
        self.start_with_port(port, &address)
    }

    /// 用指定字节通道启动（测试/回环用）。
    #[cfg(test)]
    pub(crate) fn open_with_port(
        &mut self,
        port: Box<dyn SerialPort>,
        address: &str,
    ) -> Result<LinkInfo, TransportError> {
        self.start_with_port(port, address)
    }

    /// 写入完整帧字节。
    pub fn write_frame(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        if !self.is_connected() {
            return Err(TransportError::NotConnected);
        }
        let Some(port) = self.port.as_ref() else {
            return Err(TransportError::NotConnected);
        };
        match port.write_all(frame) {
            Ok(()) => {
                self.counters
                    .tx_bytes
                    .fetch_add(frame.len() as u64, Ordering::Relaxed);
                Ok(())
            }
            Err(error) => {
                let detail = error.to_string();
                if self.connected.swap(false, Ordering::AcqRel) {
                    self.counters.add_error();
                    *self
                        .last_error
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(detail.clone());
                    self.queue.push(TransportEvent::Disconnected {
                        reason: reason_for(&error),
                        detail: Some(detail.clone()),
                    });
                }
                Err(TransportError::WriteFailed { detail })
            }
        }
    }

    /// 关闭链路（幂等）。
    pub fn close_link(&mut self) -> Result<(), TransportError> {
        self.stop_reader();
        Ok(())
    }

    fn start_with_port(
        &mut self,
        port: Box<dyn SerialPort>,
        address: &str,
    ) -> Result<LinkInfo, TransportError> {
        self.stop_reader();

        let port: Arc<dyn SerialPort> = Arc::from(port);
        self.running.store(true, Ordering::Release);
        self.connected.store(true, Ordering::Release);

        let reader = spawn_reader(
            Arc::clone(&port),
            Arc::clone(&self.running),
            Arc::clone(&self.connected),
            Arc::clone(&self.counters),
            Arc::clone(&self.queue),
            Arc::clone(&self.last_error),
        )?;

        self.port = Some(port);
        self.reader = Some(reader);
        let link = LinkInfo {
            kind: TransportKind::Usb,
            state: "connected",
            address: address.to_string(),
            mtu: MTU_USB,
            max_payload: MAX_PAYLOAD_USB,
            opened_at_ms: now_ms(),
        };
        self.link = Some(link.clone());
        if self.opened_before {
            self.counters.add_reconnect();
        }
        self.opened_before = true;
        *self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        Ok(link)
    }

    fn stop_reader(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(port) = self.port.as_ref() {
            port.cancel();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(port) = self.port.take() {
            port.close();
        }
        self.connected.store(false, Ordering::Release);
        self.link = None;
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        self.stop_reader();
    }
}

impl Transport for UsbTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Usb
    }

    fn max_payload(&self) -> usize {
        MAX_PAYLOAD_USB
    }

    fn open(&mut self, cfg: &TransportCfg) -> Result<LinkInfo, TransportError> {
        match cfg {
            TransportCfg::Usb(usb) => {
                self.cfg = usb.clone();
                self.open_usb()
            }
            TransportCfg::Ble(_) => Err(TransportError::NotImplemented {
                what: "BLE 传输（P2c sidecar）",
            }),
        }
    }

    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        self.write_frame(frame)
    }

    fn close(&mut self) -> Result<(), TransportError> {
        self.close_link()
    }

    fn events(&mut self) -> Vec<TransportEvent> {
        self.queue.drain()
    }
}

fn resolve_port(cfg: &UsbCfg) -> Result<String, TransportError> {
    if let Some(port) = &cfg.port {
        if port.trim().is_empty() {
            return Err(TransportError::InvalidArg {
                detail: "port 不能为空字符串".to_string(),
            });
        }
        return Ok(port.clone());
    }

    let ports = platform::list_ports()?;
    let matched: Vec<&UsbPortInfo> = ports.iter().filter(|info| cfg.matches(info)).collect();
    match matched.len() {
        0 => Err(TransportError::PortNotFound {
            detail: cfg.describe(),
            available: ports.iter().map(UsbPortInfo::pretty).collect(),
        }),
        1 => Ok(matched[0].device.clone()),
        _ => Err(TransportError::InvalidArg {
            detail: format!(
                "匹配到多个串口，请用 port 明确指定: {}",
                matched
                    .iter()
                    .map(|info| info.pretty())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }),
    }
}

fn spawn_reader(
    port: Arc<dyn SerialPort>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    counters: Arc<Counters>,
    queue: Arc<EventQueue>,
    last_error: Arc<Mutex<Option<String>>>,
) -> Result<JoinHandle<()>, TransportError> {
    thread::Builder::new()
        .name("usb-rx".to_string())
        .spawn(move || reader_loop(port, running, connected, counters, queue, last_error))
        .map_err(|error| TransportError::io(format!("启动 USB 读线程失败: {error}")))
}

fn reader_loop(
    port: Arc<dyn SerialPort>,
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    counters: Arc<Counters>,
    queue: Arc<EventQueue>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    let mut buffer = vec![0u8; READ_CHUNK];
    while running.load(Ordering::Acquire) {
        match port.read(&mut buffer) {
            Ok(0) => continue,
            Ok(read) => {
                counters.add_rx(read);
                queue.push(TransportEvent::Bytes(buffer[..read].to_vec()));
            }
            Err(PortError::Timeout) => continue,
            Err(error) => {
                if running.load(Ordering::Acquire) && connected.swap(false, Ordering::AcqRel) {
                    counters.add_error();
                    let detail = error.to_string();
                    *last_error
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(detail.clone());
                    queue.push(TransportEvent::Disconnected {
                        reason: reason_for(&error),
                        detail: Some(detail),
                    });
                }
                break;
            }
        }
    }
}

fn reason_for(error: &PortError) -> DisconnectReason {
    match error {
        PortError::DeviceRemoved => DisconnectReason::DeviceRemoved,
        PortError::Cancelled => DisconnectReason::User,
        PortError::Timeout => DisconnectReason::LinkLost,
        PortError::Io { .. } => DisconnectReason::LinkLost,
    }
}
