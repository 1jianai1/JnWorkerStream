//! 传输统计计数（USB/BLE 共用）。

use std::sync::atomic::{AtomicU64, Ordering};

/// 原子计数器；所有字段都是累计值。
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub rx_bytes: AtomicU64,
    pub tx_bytes: AtomicU64,
    pub errors: AtomicU64,
    pub reconnects: AtomicU64,
}

impl Counters {
    pub fn rx(&self) -> u64 {
        self.rx_bytes.load(Ordering::Relaxed)
    }

    pub fn tx(&self) -> u64 {
        self.tx_bytes.load(Ordering::Relaxed)
    }

    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    pub fn reconnects(&self) -> u64 {
        self.reconnects.load(Ordering::Relaxed)
    }

    pub fn add_rx(&self, bytes: usize) {
        self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn add_tx(&self, bytes: usize) {
        self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn add_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }
}
