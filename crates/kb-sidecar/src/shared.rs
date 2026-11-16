//! 进程共享状态：待响应请求、事件队列、stderr 环形缓冲、存活标记。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Condvar, Mutex, MutexGuard};

use serde_json::{Map, Value};

use crate::codec::SidecarFrame;
use crate::error::SidecarError;

/// 子进程状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarStatus {
    Starting,
    Ready,
    Exited { code: Option<i32> },
    ProtocolError(String),
}

/// sidecar 主动推送的事件。
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarEvent {
    pub header: Map<String, Value>,
    pub body: Vec<u8>,
}

impl SidecarEvent {
    pub fn event(&self) -> Option<&str> {
        self.header.get("event").and_then(Value::as_str)
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.header.get(key).and_then(Value::as_str)
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.header.get(key).and_then(Value::as_u64)
    }
}

#[derive(Debug, Default)]
pub(crate) struct EventQueue {
    pub items: VecDeque<SidecarEvent>,
    pub dropped: u64,
}

/// 子进程与所有线程共享的状态。
#[derive(Debug)]
pub(crate) struct Shared {
    pub pending: Mutex<HashMap<String, Sender<Result<SidecarFrame, SidecarError>>>>,
    pub events: Mutex<EventQueue>,
    pub cv: Condvar,
    pub alive: AtomicBool,
    pub status: Mutex<SidecarStatus>,
    pub stderr: Mutex<VecDeque<String>>,
    pub last_exit: Mutex<Option<i32>>,
    event_capacity: usize,
    stderr_capacity: usize,
}

impl Shared {
    pub fn new(event_capacity: usize, stderr_capacity: usize) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            events: Mutex::new(EventQueue::default()),
            cv: Condvar::new(),
            alive: AtomicBool::new(true),
            status: Mutex::new(SidecarStatus::Starting),
            stderr: Mutex::new(VecDeque::new()),
            last_exit: Mutex::new(None),
            event_capacity: event_capacity.max(1),
            stderr_capacity: stderr_capacity.max(1),
        }
    }

    pub fn lock_pending(
        &self,
    ) -> MutexGuard<'_, HashMap<String, Sender<Result<SidecarFrame, SidecarError>>>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn lock_events(&self) -> MutexGuard<'_, EventQueue> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn lock_status(&self) -> MutexGuard<'_, SidecarStatus> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn push_event(&self, event: SidecarEvent) {
        let mut queue = self.lock_events();
        if queue.items.len() >= self.event_capacity {
            queue.items.pop_front();
            queue.dropped += 1;
        }
        queue.items.push_back(event);
        drop(queue);
        self.cv.notify_all();
    }

    pub fn fail(&self, error: SidecarError) {
        let was_alive = self.alive.swap(false, Ordering::AcqRel);
        if was_alive {
            let mut status = self.lock_status();
            *status = match &error {
                SidecarError::Protocol { detail } => SidecarStatus::ProtocolError(detail.clone()),
                _ => SidecarStatus::Exited { code: None },
            };
        }
        let pending = std::mem::take(&mut *self.lock_pending());
        for (_, sender) in pending {
            let _ = sender.send(Err(error.clone()));
        }
        self.cv.notify_all();
    }

    pub fn push_stderr(&self, line: String) {
        let mut lines = self
            .stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lines.len() >= self.stderr_capacity {
            lines.pop_front();
        }
        lines.push_back(line);
    }
}
