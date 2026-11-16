//! BLE 传输：Python sidecar 客户端 + 字节/事件转发。
//!
//! 不解析帧；`write` 只接收完整帧字节，`events` 产出原始字节与断连事件。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kb_sidecar::{SidecarClient, SidecarConfig, SidecarError, SidecarStatus};
use serde_json::{json, Map};

use crate::counters::Counters;
use crate::error::TransportError;
use crate::event_queue::EventQueue;
use crate::reconnect::ReconnectPolicy;
use crate::{
    now_ms, BleCfg, DisconnectReason, LinkInfo, Transport, TransportCfg, TransportEvent,
    TransportKind, TransportStats,
};

mod support;

use support::{encode_base64, map_remote, map_sidecar_error, parse_devices};
pub use support::{BleDeviceInfo, SidecarLaunch};

const EVENT_QUEUE_CAPACITY: usize = 256;
const BLE_DEFAULT_MTU: u32 = 247;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(35);
/// 自动重连的单次连接预算：比手动连接短，避免一次重试长时间占住调用线程。
const RETRY_CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// 重连进度快照（对应 `link.state` 的 `reconnect_attempt` / `next_retry_ms`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconnectStatus {
    /// 已排期的重试序号（0 = 当前不在重连中）。
    pub attempt: u32,
    /// 下一次重试的时间戳（Unix ms）。
    pub next_retry_ms: Option<u64>,
}

/// 自动重连的内部状态。
#[derive(Debug, Default)]
struct RetryState {
    attempt: u32,
    due_at_ms: Option<u64>,
    /// 用户主动关闭/尚未 open 时为 true，抑制自动重连。
    suspended: bool,
}

/// BLE 传输实现。
#[derive(Debug)]
pub struct BleTransport {
    launch: SidecarLaunch,
    cfg: BleCfg,
    /// 最近一次成功连接的参数，用于自动重连。
    last_cfg: Option<BleCfg>,
    client: Option<SidecarClient>,
    link: Option<LinkInfo>,
    queue: Arc<EventQueue>,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<Option<String>>>,
    policy: ReconnectPolicy,
    retry: RetryState,
    opened_before: bool,
    sidecar_dead_reported: bool,
}

impl Default for BleTransport {
    fn default() -> Self {
        Self::new(SidecarLaunch::default())
    }
}

impl BleTransport {
    pub fn new(launch: SidecarLaunch) -> Self {
        Self {
            launch,
            cfg: BleCfg::default(),
            last_cfg: None,
            client: None,
            link: None,
            queue: Arc::new(EventQueue::new(EVENT_QUEUE_CAPACITY)),
            counters: Arc::new(Counters::default()),
            last_error: Arc::new(Mutex::new(None)),
            policy: ReconnectPolicy::default(),
            retry: RetryState::default(),
            opened_before: false,
            sidecar_dead_reported: false,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_some() && self.client.as_ref().is_some_and(SidecarClient::is_alive)
    }

    pub fn link(&self) -> Option<&LinkInfo> {
        self.link.as_ref()
    }

    pub fn reconnect_policy(&self) -> &ReconnectPolicy {
        &self.policy
    }

    pub fn set_reconnect_policy(&mut self, policy: ReconnectPolicy) {
        self.policy = policy;
    }

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

    /// 当前重连进度（不在重连中时 `attempt = 0`）。
    pub fn reconnect_status(&self) -> ReconnectStatus {
        ReconnectStatus {
            attempt: self.retry.attempt,
            next_retry_ms: self.retry.due_at_ms,
        }
    }

    /// 立即重试（UI 的“立即重试”）：把下一次退避提前到当前时刻。
    ///
    /// 返回 `false` 表示当前没有可重试的目标（未 open 过或已被手动关闭）。
    pub fn retry_now(&mut self) -> bool {
        if self.retry.suspended || self.last_cfg.is_none() {
            return false;
        }
        self.retry.due_at_ms = Some(0);
        true
    }

    /// 打开 BLE（已连接时幂等）。
    pub fn open_ble(&mut self, cfg: &BleCfg) -> Result<LinkInfo, TransportError> {
        if self.is_connected() {
            if let Some(link) = &self.link {
                return Ok(link.clone());
            }
        }
        self.cfg = cfg.clone();
        let link = self.connect_with(cfg, CONNECT_TIMEOUT)?;
        self.last_cfg = Some(cfg.clone());
        self.retry.attempt = 0;
        self.retry.due_at_ms = None;
        self.retry.suspended = false;
        Ok(link)
    }

    /// 连接实现：`open_ble` 与自动重连共用，只有超时预算不同。
    fn connect_with(
        &mut self,
        cfg: &BleCfg,
        timeout: Duration,
    ) -> Result<LinkInfo, TransportError> {
        if self.is_connected() {
            if let Some(link) = &self.link {
                return Ok(link.clone());
            }
        }
        self.ensure_client()?;

        let mut params = Map::new();
        if let Some(address) = &cfg.address {
            params.insert("address".to_string(), json!(address));
        }
        if let Some(name) = &cfg.name {
            params.insert("name".to_string(), json!(name));
        }
        params.insert("paired_only".to_string(), json!(cfg.paired_only));
        params.insert("timeout_ms".to_string(), json!(timeout.as_millis() as u64));

        let client = self.client.as_ref().expect("ensure_client");
        let response = match client.request_timeout("ble.connect", params, timeout) {
            Ok(frame) => frame,
            Err(SidecarError::Timeout { .. }) => {
                let detail = "ble.connect 超时（设备未连接/未广播）".to_string();
                self.record_error(&detail);
                return Err(TransportError::ConnectTimeout { detail });
            }
            Err(error) => {
                let mapped = map_sidecar_error(error);
                self.record_error(&mapped.message());
                return Err(mapped);
            }
        };
        if !response.ok() {
            let mapped = map_remote(
                response.error_code().unwrap_or("E_SIDECAR_PROTOCOL"),
                response.error_message().unwrap_or("BLE 连接失败"),
            );
            self.record_error(&mapped.message());
            return Err(mapped);
        }

        let mtu = response.get_u64("mtu").unwrap_or(23) as u32;
        let link = LinkInfo {
            kind: TransportKind::Ble,
            state: "connected",
            address: response.get_str("address").unwrap_or_default().to_string(),
            mtu,
            max_payload: mtu.saturating_sub(14) as usize,
            opened_at_ms: now_ms(),
        };
        if self.opened_before {
            self.counters.add_reconnect();
        }
        self.opened_before = true;
        self.sidecar_dead_reported = false;
        self.link = Some(link.clone());
        self.clear_error();
        Ok(link)
    }

    /// 写入完整帧字节（单包，不分片）。
    pub fn write_frame(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        if !self.is_connected() {
            return Err(TransportError::NotConnected);
        }
        let mtu = self.link.as_ref().map_or(BLE_DEFAULT_MTU, |link| link.mtu) as usize;
        let max_frame = mtu.saturating_sub(3);
        if frame.len() > max_frame {
            return Err(TransportError::PayloadTooLarge {
                len: frame.len(),
                max: max_frame,
            });
        }
        let data_b64 = encode_base64(frame);
        let mut params = Map::new();
        params.insert("data_b64".to_string(), json!(data_b64));
        let client = self.client.as_ref().ok_or(TransportError::NotConnected)?;
        match client.request_timeout("ble.write", params, WRITE_TIMEOUT) {
            Ok(response) if response.ok() => {
                self.counters.add_tx(frame.len());
                Ok(())
            }
            Ok(response) => {
                let mapped = map_remote(
                    response.error_code().unwrap_or("E_WRITE_FAILED"),
                    response.error_message().unwrap_or("BLE 写入失败"),
                );
                self.counters.add_error();
                self.record_error(&mapped.message());
                Err(mapped)
            }
            Err(error) => {
                let mapped = map_sidecar_error(error);
                self.counters.add_error();
                self.record_error(&mapped.message());
                Err(mapped)
            }
        }
    }

    /// 扫描 BLE 设备（含已配对记录，合并去重）。
    pub fn scan(
        &mut self,
        timeout_ms: u32,
        name_filter: Option<&str>,
    ) -> Result<Vec<BleDeviceInfo>, TransportError> {
        self.ensure_client()?;
        let mut params = Map::new();
        params.insert("timeout_ms".to_string(), json!(timeout_ms));
        if let Some(name) = name_filter {
            params.insert("name_filter".to_string(), json!(name));
        }
        let client = self.client.as_ref().expect("ensure_client");
        let frame = client.call("ble.scan", params).map_err(|error| {
            let mapped = map_sidecar_error(error);
            self.record_error(&mapped.message());
            mapped
        })?;
        Ok(parse_devices(&frame.header))
    }

    /// 枚举 Windows 已配对 BLE 设备。
    pub fn list_paired(&mut self) -> Result<Vec<BleDeviceInfo>, TransportError> {
        self.ensure_client()?;
        let client = self.client.as_ref().expect("ensure_client");
        let frame = client
            .call("ble.list_paired", Map::new())
            .map_err(|error| {
                let mapped = map_sidecar_error(error);
                self.record_error(&mapped.message());
                mapped
            })?;
        Ok(parse_devices(&frame.header))
    }

    /// 取出并映射 sidecar 事件。
    pub fn drain_events(&mut self) -> Vec<TransportEvent> {
        let (events, alive, status) = match &self.client {
            Some(client) => (client.try_events(), client.is_alive(), client.status()),
            None => (Vec::new(), false, SidecarStatus::Exited { code: None }),
        };

        for event in events {
            match event.event() {
                Some("ble.bytes") => {
                    self.counters.add_rx(event.body.len());
                    self.queue.push(TransportEvent::Bytes(event.body));
                }
                Some("ble.link") if event.get_str("state") == Some("disconnected") => {
                    let reason = match event.get_str("reason") {
                        Some("device_removed") => DisconnectReason::DeviceRemoved,
                        Some("user") => DisconnectReason::User,
                        _ => DisconnectReason::LinkLost,
                    };
                    self.link = None;
                    self.queue.push(TransportEvent::Disconnected {
                        reason,
                        detail: event.get_str("reason").map(str::to_string),
                    });
                }
                _ => {}
            }
        }

        if !alive && !self.sidecar_dead_reported {
            self.sidecar_dead_reported = true;
            self.counters.add_error();
            let detail = format!("BLE sidecar 已退出（{status:?}）");
            self.record_error(&detail);
            if self.link.take().is_some() {
                self.queue.push(TransportEvent::Disconnected {
                    reason: DisconnectReason::Error,
                    detail: Some(detail.clone()),
                });
            }
            self.queue
                .push(TransportEvent::Error(TransportError::Sidecar { detail }));
        }

        self.poll_reconnect();
        self.queue.drain()
    }

    /// 自动重连调度：断连后按 `ReconnectPolicy` 排期，到期在本调用内尝试一次。
    ///
    /// 由 [`Self::drain_events`] 驱动，因此调用方只需持续 `events()`；
    /// 未 open 过、被手动 `close_link`、或 `auto=false` 时不动。
    fn poll_reconnect(&mut self) {
        if self.retry.suspended || !self.policy.should_retry() || self.last_cfg.is_none() {
            return;
        }
        if self.link.is_some() {
            return;
        }
        let now = now_ms();
        match self.retry.due_at_ms {
            None => self.arm_retry(now),
            Some(due) if now < due => {}
            Some(_) => self.attempt_reconnect(),
        }
    }

    /// 排下一次重试（attempt 从 1 开始），并播报 `Reconnecting`。
    fn arm_retry(&mut self, at_ms: u64) {
        self.retry.attempt = self.retry.attempt.saturating_add(1);
        let delay_ms = self.policy.delay_ms(self.retry.attempt);
        self.retry.due_at_ms = Some(at_ms.saturating_add(delay_ms));
        self.queue.push(TransportEvent::Reconnecting {
            attempt: self.retry.attempt,
            delay_ms,
        });
    }

    /// 到期执行一次重连；失败则继续排期（退避翻倍，封顶）。
    fn attempt_reconnect(&mut self) {
        let Some(cfg) = self.last_cfg.clone() else {
            self.retry.due_at_ms = None;
            return;
        };
        self.retry.due_at_ms = None;
        match self.connect_with(&cfg, RETRY_CONNECT_TIMEOUT) {
            Ok(link) => {
                self.retry.attempt = 0;
                self.retry.suspended = false;
                self.queue.push(TransportEvent::Reconnected { link });
            }
            Err(error) => {
                self.counters.add_error();
                self.record_error(&error.message());
                self.arm_retry(now_ms());
            }
        }
    }

    /// 关闭：disconnect → 停 sidecar。
    pub fn close_link(&mut self) -> Result<(), TransportError> {
        if let Some(mut client) = self.client.take() {
            if client.is_alive() {
                let _ =
                    client.request_timeout("ble.disconnect", Map::new(), Duration::from_secs(2));
            }
            client.stop();
        }
        self.link = None;
        self.retry.suspended = true;
        self.retry.due_at_ms = None;
        self.retry.attempt = 0;
        self.sidecar_dead_reported = true;
        Ok(())
    }

    /// sidecar 崩溃后按原配置重启（不自动重连 BLE）。
    pub fn restart_sidecar(&mut self) -> Result<(), TransportError> {
        if let Some(mut client) = self.client.take() {
            client.stop();
        }
        self.link = None;
        self.retry.due_at_ms = None;
        self.sidecar_dead_reported = false;
        self.ensure_client()
    }

    fn ensure_client(&mut self) -> Result<(), TransportError> {
        if self.client.as_ref().is_some_and(SidecarClient::is_alive) {
            return Ok(());
        }
        if let Some(mut dead) = self.client.take() {
            dead.stop();
        }
        let mut cfg = SidecarConfig::ble(self.launch.program.clone(), self.launch.script.clone())
            .with_request_timeout(Duration::from_secs(10));
        cfg.cwd = self.launch.cwd.clone();
        cfg.env = self.launch.env.clone();
        let client = SidecarClient::start(cfg).map_err(|error| {
            let mapped = map_sidecar_error(error);
            self.record_error(&mapped.message());
            mapped
        })?;
        self.client = Some(client);
        self.sidecar_dead_reported = false;
        Ok(())
    }

    fn record_error(&self, detail: &str) {
        *self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(detail.to_string());
    }

    fn clear_error(&self) {
        *self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        let _ = self.close_link();
    }
}

impl Transport for BleTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Ble
    }

    fn max_payload(&self) -> usize {
        self.link
            .as_ref()
            .map_or(BLE_DEFAULT_MTU as usize - 14, |link| link.max_payload)
    }

    fn open(&mut self, cfg: &TransportCfg) -> Result<LinkInfo, TransportError> {
        match cfg {
            TransportCfg::Ble(ble) => self.open_ble(ble),
            TransportCfg::Usb(_) => Err(TransportError::InvalidArg {
                detail: "BleTransport 只接受 kind=ble".to_string(),
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
        self.drain_events()
    }
}
