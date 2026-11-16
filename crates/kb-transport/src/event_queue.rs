//! 有界事件队列：满时丢最旧，避免采集线程被渲染阻塞（ADR-008）。

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use crate::TransportEvent;

#[derive(Debug, Default)]
struct QueueInner {
    items: VecDeque<TransportEvent>,
    dropped: u64,
}

/// 有界事件队列（USB/BLE 共用）。
#[derive(Debug)]
pub(crate) struct EventQueue {
    capacity: usize,
    inner: Mutex<QueueInner>,
}

impl EventQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(QueueInner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, QueueInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn push(&self, event: TransportEvent) {
        let mut inner = self.lock();
        if inner.items.len() >= self.capacity {
            inner.items.pop_front();
            inner.dropped += 1;
        }
        inner.items.push_back(event);
    }

    pub fn drain(&self) -> Vec<TransportEvent> {
        self.lock().items.drain(..).collect()
    }

    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }
}
