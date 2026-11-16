//! Python sidecar 监督 + 帧协议（P2c）。
//!
//! 边界（`docs/interfaces.md` §4、ADR-004/007）：
//!
//! - [codec]：`JN` 长度前缀帧，stdout 只读协议，stderr 逐行转日志；
//! - [client]：子进程生命周期（spawn → hello → request/response → 退出清理）、
//!   请求关联、超时、事件队列；
//! - 不做 BLE/ASR 业务，不解析 `A5 5A`。
//!
//! 只依赖 `serde_json`，不引入异步运行时；读写各自一个线程，
//! 请求在调用线程阻塞等待，符合“传输读线程不阻塞 UI”的分层。

pub mod client;
pub mod codec;
pub mod config;
pub mod error;
mod shared;

pub use client::SidecarClient;
pub use codec::{SidecarFrame, MAGIC, MAX_FRAME, PROTO_VERSION};
pub use config::SidecarConfig;
pub use error::SidecarError;
pub use shared::{SidecarEvent, SidecarStatus};
