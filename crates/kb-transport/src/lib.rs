//! 传输层（P2a）：USB CDC 字节通道、端口枚举、断连检测、重连退避与统计。
//!
//! 边界（`docs/interfaces.md` §5、ADR-018）：
//!
//! - 传输层**只搬字节**，不解析 `A5 5A`，不构建帧；
//! - [`Transport::write`] 只接收完整帧字节；
//! - [`Transport::events`] 产出 `Bytes` / `Disconnected` / `Error`，由上层解析器消费；
//! - 产品与接口统一称 `usb`，不称 UART/串口。
//!
//! 平台相关代码集中在 [`platform`]；缩略/模拟链路由测试用 `SerialPort` 替身提供，
//! 生产构建不暴露（与 ADR-018 删除 `SimTransport` 的结论一致）。

pub mod ble;
mod counters;
pub mod error;
mod event_queue;
pub mod platform;
pub mod reconnect;
pub mod usb;

pub use ble::{BleDeviceInfo, BleTransport, ReconnectStatus, SidecarLaunch};
pub use error::TransportError;
pub use platform::open_port;
pub use reconnect::ReconnectPolicy;
pub use usb::{list_ports, UsbCfg, UsbPortInfo, UsbTransport};

/// USB CDC 的逻辑 MTU（`docs/interfaces.md` §3.3）。
pub const MTU_USB: u32 = 1024;
/// USB CDC 单帧 payload 上限：`1024 - 11`。
pub const MAX_PAYLOAD_USB: usize = kb_protocol::MAX_PAYLOAD - kb_protocol::OVERHEAD;

/// 链路种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Usb,
    Ble,
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Ble => "ble",
        }
    }
}

/// BLE 连接参数（客户端实现属 P2c；当前只保留冻结字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleCfg {
    pub address: Option<String>,
    pub name: Option<String>,
    pub paired_only: bool,
    pub scan_timeout_ms: u32,
}

impl Default for BleCfg {
    fn default() -> Self {
        Self {
            address: None,
            name: None,
            paired_only: false,
            scan_timeout_ms: 6_000,
        }
    }
}

/// 打开链路的参数，对应 `link.connect` 的 `kind + usb|ble`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportCfg {
    Usb(UsbCfg),
    Ble(BleCfg),
}

/// 成功打开的链路信息（对应 `LinkInfo`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub kind: TransportKind,
    pub state: &'static str,
    pub address: String,
    pub mtu: u32,
    pub max_payload: usize,
    pub opened_at_ms: u64,
}

/// 断连原因（`link_lost` / `device_removed` / `user`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    LinkLost,
    DeviceRemoved,
    User,
    Error,
}

impl DisconnectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinkLost => "link_lost",
            Self::DeviceRemoved => "device_removed",
            Self::User => "user",
            Self::Error => "error",
        }
    }
}

/// 传输层事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    Bytes(Vec<u8>),
    Disconnected {
        reason: DisconnectReason,
        detail: Option<String>,
    },
    /// 已排期第 `attempt` 次自动重连，`delay_ms` 后尝试。
    Reconnecting {
        attempt: u32,
        delay_ms: u64,
    },
    /// 自动重连成功，链路恢复可用（Notify 已在连接内重新订阅）。
    Reconnected {
        link: LinkInfo,
    },
    Error(TransportError),
}

/// 传输统计（`transport.stats` / `stats.tick`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportStats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub errors: u64,
    pub reconnects: u64,
    /// 事件队列满时丢弃的字节事件数。
    pub dropped: u64,
    pub last_error: Option<String>,
}

/// 链路抽象（语义平台无关，`docs/interfaces.md` §5）。
pub trait Transport {
    fn kind(&self) -> TransportKind;
    fn max_payload(&self) -> usize;
    fn open(&mut self, cfg: &TransportCfg) -> Result<LinkInfo, TransportError>;
    fn write(&mut self, frame: &[u8]) -> Result<(), TransportError>;
    fn close(&mut self) -> Result<(), TransportError>;
    fn events(&mut self) -> Vec<TransportEvent>;
}

/// Unix 毫秒时间戳。
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
