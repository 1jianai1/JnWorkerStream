//! 应用核心（无 Tauri 依赖）：链路状态机 + 帧/分段路由 + 语音识别 + 事件与快照。
//!
//! 分层（`docs/architecture.md` §5、ADR-006）：
//!
//! - 本 crate 是**唯一状态源**：连接状态、解析统计、语音结果都在这里；
//! - `src-tauri` 只做三件事：命令转发、把 [`CoreEvent`] 原样 emit、把 [`Snapshot`] 原样返回；
//! - 事件名与字段遵循 `docs/interfaces.md` §2.3，快照遵循 §2.4。
//!
//! 为什么不在 `src-tauri` 里写：本机拉不到 Tauri 依赖，写在壳里等于无法编译验证。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kb_audio::VoiceSession;
use kb_protocol::{FrameParser, MAX_PAYLOAD, TYPE_AUDIO_CTL, TYPE_AUDIO_DATA};
use kb_transport::{
    BleCfg, BleTransport, LinkInfo, SidecarLaunch, Transport, TransportCfg, TransportEvent,
};
use kb_voice::{AsrClient, TextInjector, VoicePipeline, VoiceResult};
use serde::Serialize;

/// 一次 `poll()` 里最多处理的事件数，避免单帧卡住调用方。
pub const MAX_EVENTS_PER_POLL: usize = 512;

/// 链路状态（`link.state.state`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Error,
}

impl LinkState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }
}

/// `link.state` 事件的载荷（`docs/interfaces.md` §2.3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkSnapshot {
    pub revision: u64,
    pub state: &'static str,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_payload: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconnect_attempt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_retry_ms: Option<u64>,
}

/// `frame.batch` 里的一条（音频 PCM 不出现，只放元数据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameBrief {
    pub seq: u16,
    #[serde(rename = "type")]
    pub ty: u8,
    pub flags: u8,
    pub len: usize,
    pub at_ms: u64,
}

/// `frame.batch` 载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameBatch {
    pub revision: u64,
    pub dropped: u64,
    pub frames: Vec<FrameBrief>,
}

/// `voice.result` 载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceEvent {
    pub revision: u64,
    pub seg_id: i32,
    pub text: String,
    pub language: String,
    pub audio_ms: u64,
    pub latency_ms: u64,
    pub inject: InjectReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 注入结果（`voice.result.inject`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InjectReport {
    pub mode: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 核心产出的事件；`name()` 即 Tauri 事件名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    LinkState(LinkSnapshot),
    FrameBatch(FrameBatch),
    VoiceResult(VoiceEvent),
    Error { revision: u64, message: String },
}

impl CoreEvent {
    /// Tauri 事件名。
    ///
    /// **不能带点号**：Tauri 2 的 `EventName` 只允许字母数字和 `- / : _`
    /// （`tauri/src/event/event_name.rs`），带 `.` 的 `emit` 会直接返回
    /// `IllegalEventName`。这里用连字符代替文档里的逻辑名 `link.state` 等（ADR-021）。
    pub fn name(&self) -> &'static str {
        match self {
            Self::LinkState(_) => "link-state",
            Self::FrameBatch(_) => "frame-batch",
            Self::VoiceResult(_) => "voice-result",
            Self::Error { .. } => "app-error",
        }
    }

    /// 给 Tauri `emit` 用的 JSON 载荷（字段名与 `docs/interfaces.md` §2.3 一致）。
    pub fn payload(&self) -> serde_json::Value {
        let encoded = match self {
            Self::LinkState(value) => serde_json::to_value(value).ok(),
            Self::FrameBatch(value) => serde_json::to_value(value).ok(),
            Self::VoiceResult(value) => serde_json::to_value(value).ok(),
            Self::Error { revision, message } => {
                Some(serde_json::json!({ "revision": revision, "message": message }))
            }
        };
        encoded.unwrap_or(serde_json::Value::Null)
    }
}

/// 解析统计的对外形态（`Snapshot.parser`，字段名与 `docs/interfaces.md` §2.4 一致）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ParserSnapshot {
    pub frames: u64,
    pub bytes: u64,
    pub crc_err: u64,
    pub oversize: u64,
    pub unknown_type: u64,
    pub resync: u64,
    pub overflow: u64,
}

/// 传输统计的对外形态（`Snapshot.transport`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TransportSnapshot {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub errors: u64,
    pub reconnects: u64,
    pub dropped: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 语音统计（`Snapshot.voice`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VoiceSnapshot {
    pub enabled: bool,
    pub segments: u64,
    pub interrupted: u64,
    pub failures: u64,
    pub last_text: Option<String>,
}

/// 快照（`docs/interfaces.md` §2.4 的子集；`check`/`recording`/`settings` 属后续阶段）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub link: LinkSnapshot,
    pub parser: ParserSnapshot,
    pub transport: TransportSnapshot,
    pub voice: VoiceSnapshot,
}

impl Default for LinkSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            state: "idle",
            kind: "ble",
            address: None,
            mtu: None,
            max_payload: None,
            last_error: None,
            reconnect_attempt: None,
            next_retry_ms: None,
        }
    }
}

/// 应用核心。
pub struct Core<I: TextInjector = kb_inject::Injector> {
    transport: BleTransport,
    parser: FrameParser,
    session: VoiceSession,
    voice: Option<VoicePipeline<I>>,
    link: LinkSnapshot,
    revision: u64,
    segments: u64,
    failures: u64,
    last_text: Option<String>,
    pending_frames: Vec<FrameBrief>,
    /// 取消令牌：命令层可以在**不拿 Core 锁**的情况下置位，从而打断「连接中」的等待。
    cancel: Arc<AtomicBool>,
    connect_timeout: Duration,
}

impl<I: TextInjector> Core<I> {
    /// 手动连接的超时：连不上就自动结束，避免 UI 无限转圈。
    pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new(launch: SidecarLaunch) -> Self {
        Self {
            transport: BleTransport::new(launch),
            parser: FrameParser::new(MAX_PAYLOAD),
            session: VoiceSession::new(),
            voice: None,
            link: LinkSnapshot::default(),
            revision: 0,
            segments: 0,
            failures: 0,
            last_text: None,
            pending_frames: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            connect_timeout: Self::DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// 取消令牌句柄：命令层保存一份，点「终止连接」时直接置位，不需要抢 Core 锁。
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// 设置手动连接超时（默认 10 s）。
    pub fn set_connect_timeout(&mut self, timeout: Duration) {
        self.connect_timeout = timeout;
        self.transport.set_connect_timeout(timeout);
    }

    /// 挂上语音链路（ASR 客户端 + 注入器）；`None` 表示只收不识别。
    pub fn attach_voice(&mut self, client: AsrClient, injector: I) {
        self.voice = Some(VoicePipeline::new(client, injector));
    }

    pub fn detach_voice(&mut self) {
        self.voice = None;
    }

    pub fn voice_enabled(&self) -> bool {
        self.voice.is_some()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn link(&self) -> &LinkSnapshot {
        &self.link
    }

    /// `link.connect`。
    ///
    /// 连接最多等 `connect_timeout`（默认 10 s）；期间用户点「终止连接」会把
    /// [`Self::cancel`] 置位，连接返回后立刻清理并回 `E_CANCELLED`。
    pub fn connect_ble(&mut self, cfg: BleCfg) -> Result<LinkInfo, kb_transport::TransportError> {
        self.cancel.store(false, Ordering::SeqCst);
        self.transport.set_connect_timeout(self.connect_timeout);
        self.bump();
        self.link.state = LinkState::Connecting.as_str();
        self.link.last_error = None;

        let result = self.transport.open(&TransportCfg::Ble(cfg));
        if self.cancel.swap(false, Ordering::SeqCst) {
            let _ = self.transport.close();
            self.bump();
            self.link = LinkSnapshot {
                revision: self.revision,
                state: LinkState::Disconnected.as_str(),
                kind: "ble",
                ..LinkSnapshot::default()
            };
            return Err(kb_transport::TransportError::Cancelled);
        }

        match result {
            Ok(link) => {
                self.apply_link(&link);
                Ok(link)
            }
            Err(error) => {
                self.bump();
                self.link.state = LinkState::Error.as_str();
                self.link.last_error = Some(error.message());
                Err(error)
            }
        }
    }

    /// `link.disconnect`（幂等）。
    pub fn disconnect(&mut self) -> Result<(), kb_transport::TransportError> {
        let result = self.transport.close();
        self.bump();
        self.link = LinkSnapshot {
            revision: self.revision,
            kind: "ble",
            ..LinkSnapshot::default()
        };
        result
    }

    /// `link.reconnect_policy`。
    pub fn set_auto_reconnect(&mut self, auto: bool) {
        let mut policy = self.transport.reconnect_policy().clone();
        policy.auto = auto;
        self.transport.set_reconnect_policy(policy);
    }

    /// 立即重试（UI 的「立即重试」）。
    pub fn retry_now(&mut self) -> bool {
        self.transport.retry_now()
    }

    /// 驱动一次：把传输层事件转成核心事件。调用方按 10–50 ms 周期调用。
    pub fn poll(&mut self) -> Vec<CoreEvent> {
        let mut events = Vec::new();
        let transport_events = self.transport.events();
        for event in transport_events.into_iter().take(MAX_EVENTS_PER_POLL) {
            match event {
                TransportEvent::Bytes(bytes) => {
                    events.extend(self.ingest(&bytes));
                }
                TransportEvent::Disconnected { reason, detail } => {
                    self.bump();
                    self.link.state = LinkState::Disconnected.as_str();
                    self.link.last_error = detail.clone();
                    events.push(CoreEvent::LinkState(LinkSnapshot {
                        revision: self.revision,
                        kind: "ble",
                        ..self.link.clone()
                    }));
                    let _ = reason;
                }
                TransportEvent::Reconnecting { attempt, delay_ms } => {
                    self.bump();
                    self.link.state = LinkState::Reconnecting.as_str();
                    self.link.reconnect_attempt = Some(attempt);
                    self.link.next_retry_ms = Some(delay_ms);
                    events.push(CoreEvent::LinkState(LinkSnapshot {
                        revision: self.revision,
                        kind: "ble",
                        ..self.link.clone()
                    }));
                }
                TransportEvent::Reconnected { link } => {
                    self.bump();
                    self.apply_link(&link);
                    events.push(CoreEvent::LinkState(LinkSnapshot {
                        revision: self.revision,
                        kind: "ble",
                        ..self.link.clone()
                    }));
                }
                TransportEvent::Error(error) => {
                    self.bump();
                    self.link.last_error = Some(error.message());
                    events.push(CoreEvent::Error {
                        revision: self.revision,
                        message: error.message(),
                    });
                }
            }
        }
        if !self.pending_frames.is_empty() {
            events.push(CoreEvent::FrameBatch(FrameBatch {
                revision: self.revision,
                dropped: 0,
                frames: std::mem::take(&mut self.pending_frames),
            }));
        }
        events
    }

    /// 处理一段原始字节（`poll()` 与测试共用）。
    pub fn ingest(&mut self, bytes: &[u8]) -> Vec<CoreEvent> {
        let mut events = Vec::new();
        let frames = self.parser.feed(bytes);
        let at_ms = now_ms();
        let mut completed: Vec<VoiceResult> = Vec::new();

        for frame in frames {
            self.pending_frames.push(FrameBrief {
                seq: frame.seq,
                ty: frame.ty,
                flags: frame.flags,
                len: frame.payload.len(),
                at_ms,
            });
            match frame.ty {
                TYPE_AUDIO_CTL => {
                    // PTT 按下（ev=0）时先记下当前前台窗口：识别结果 1–3 s 后才回来，
                    // 期间用户可能点回上位机，注入时要能切回他说话时的目标窗口。
                    if frame.payload.first() == Some(&0) {
                        if let Some(pipeline) = self.voice.as_mut() {
                            pipeline.begin_segment();
                        }
                    }
                    if let Some(segment) = self.session.on_ctl_frame(&frame, at_ms) {
                        completed.push(self.handle_segment(&segment));
                    }
                }
                TYPE_AUDIO_DATA => {
                    let _ = self.session.feed(&frame, at_ms);
                }
                _ => {}
            }
        }

        for result in completed {
            events.push(self.voice_event(result));
        }
        events
    }

    fn handle_segment(&mut self, segment: &kb_audio::AudioSegment) -> VoiceResult {
        self.segments += 1;
        match self.voice.as_mut() {
            Some(pipeline) => {
                let result = pipeline.handle_segment(segment);
                if !result.is_ok() {
                    self.failures += 1;
                }
                if !result.text.is_empty() {
                    self.last_text = Some(result.text.clone());
                }
                result
            }
            None => VoiceResult::skipped(segment.seg_id, "voice_disabled"),
        }
    }

    fn voice_event(&mut self, result: VoiceResult) -> CoreEvent {
        self.bump();
        let inject = match &result.injected {
            Some(outcome) => InjectReport {
                mode: outcome.method,
                ok: outcome.ok,
                error: outcome.error.clone(),
            },
            None => InjectReport {
                mode: "none",
                ok: false,
                error: result.note.clone(),
            },
        };
        CoreEvent::VoiceResult(VoiceEvent {
            revision: self.revision,
            seg_id: result.seg_id,
            text: result.text,
            language: "zh".to_string(),
            audio_ms: result.audio_ms,
            latency_ms: result.latency_ms,
            inject,
            note: result.note,
        })
    }

    fn apply_link(&mut self, link: &LinkInfo) {
        self.link.state = LinkState::Connected.as_str();
        self.link.kind = link.kind.as_str();
        self.link.address = Some(link.address.clone());
        self.link.mtu = Some(link.mtu);
        self.link.max_payload = Some(link.max_payload);
        self.link.last_error = None;
        self.link.reconnect_attempt = None;
        self.link.next_retry_ms = None;
    }

    fn bump(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// `app.snapshot`。
    pub fn snapshot(&self) -> Snapshot {
        let parser = self.parser.stats();
        let transport = self.transport.stats();
        let (received, _total, _missing) = self.session.stats();
        let _ = received;
        Snapshot {
            revision: self.revision,
            link: LinkSnapshot {
                revision: self.revision,
                ..self.link.clone()
            },
            parser: ParserSnapshot {
                frames: parser.frames,
                bytes: parser.bytes,
                crc_err: parser.crc_err,
                oversize: parser.oversize,
                unknown_type: parser.unknown_type,
                resync: parser.resync,
                overflow: parser.overflow,
            },
            transport: TransportSnapshot {
                rx_bytes: transport.rx_bytes,
                tx_bytes: transport.tx_bytes,
                errors: transport.errors,
                reconnects: transport.reconnects,
                dropped: transport.dropped,
                last_error: transport.last_error,
            },
            voice: VoiceSnapshot {
                enabled: self.voice.is_some(),
                segments: self.segments,
                interrupted: self.session.interrupted(),
                failures: self.failures,
                last_text: self.last_text.clone(),
            },
        }
    }
}

impl<I: TextInjector> Drop for Core<I> {
    fn drop(&mut self) {
        let _ = self.transport.close();
    }
}

/// 轮询周期（`docs/architecture.md` §5.1）。
pub const POLL_INTERVAL: Duration = Duration::from_millis(10);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
