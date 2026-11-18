//! 语音链路（P2c-2）：录音段 → ASR sidecar → 文本注入。
//!
//! 边界（`docs/interfaces.md` §5、ADR-005）：
//!
//! - 分段与解码在 `kb-audio`（本 crate 只拿 [`AudioSegment`] 的 PCM）；
//! - 识别走 `sidecars/asr`（`asr.load` / `asr.warmup` / `asr.transcribe`）；
//! - 注入在 `kb-inject`（SendInput / 剪贴板），本 crate 通过 [`TextInjector`] 解耦，便于测试；
//! - 一次按键一次识别：段结束才有结果，不并发。
//!
//! 未实现：`asr.cancel`（`SidecarClient` 的请求 id 由内部生成，暂不暴露；串行链路上不阻塞）。

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use kb_audio::AudioSegment;
use kb_inject::{InjectMode, InjectOutcome};
use kb_sidecar::{SidecarClient, SidecarConfig, SidecarError};
use serde_json::{json, Map};

/// `asr.load` 超时（`docs/interfaces.md` §4.5）。
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// `asr.warmup` 超时。
pub const WARMUP_TIMEOUT: Duration = Duration::from_secs(60);
/// `asr.transcribe` 超时。
pub const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(30);
/// 采样率（冻结：只接受 16 kHz）。
pub const SAMPLE_RATE: u32 = 16_000;

/// ASR sidecar 启动参数（与 `BleTransport` 的 `SidecarLaunch` 同构）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsrLaunch {
    pub program: String,
    pub script: String,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl AsrLaunch {
    pub fn new(program: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            script: script.into(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// 一次识别结果（`asr.transcribe` 响应）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcription {
    pub text: String,
    pub language: String,
    pub audio_ms: u64,
    pub latency_ms: u64,
}

/// ASR 客户端：`kb-sidecar` 之上的薄封装。
#[derive(Debug)]
pub struct AsrClient {
    client: SidecarClient,
    loaded: bool,
}

impl AsrClient {
    /// 启动 sidecar 并校验 `role=asr`。
    pub fn start(launch: AsrLaunch) -> Result<Self, SidecarError> {
        let mut config = SidecarConfig::asr(launch.program, launch.script);
        config.cwd = launch.cwd;
        config.env = launch.env;
        let client = SidecarClient::start(config)?;
        Ok(Self {
            client,
            loaded: false,
        })
    }

    pub fn is_alive(&self) -> bool {
        self.client.is_alive()
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// 最近若干行 sidecar stderr（诊断用）。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.client.stderr_tail()
    }

    /// `asr.load`（幂等）。
    pub fn load(
        &mut self,
        model_path: Option<&str>,
        cpu_threads: u32,
    ) -> Result<u64, SidecarError> {
        let mut params = Map::new();
        if let Some(path) = model_path {
            params.insert("model_path".to_string(), json!(path));
        }
        params.insert("device".to_string(), json!("cpu"));
        params.insert("compute_type".to_string(), json!("int8"));
        params.insert("language".to_string(), json!("zh"));
        params.insert("beam_size".to_string(), json!(1));
        params.insert("vad_filter".to_string(), json!(true));
        params.insert("cpu_threads".to_string(), json!(cpu_threads));

        let frame = self
            .client
            .request_timeout("asr.load", params, LOAD_TIMEOUT)?;
        if !frame.ok() {
            return Err(remote(&frame));
        }
        self.loaded = true;
        Ok(frame.get_u64("load_ms").unwrap_or(0))
    }

    /// `asr.warmup`：0.5 s 静音前推。
    pub fn warmup(&mut self) -> Result<u64, SidecarError> {
        let frame = self
            .client
            .request_timeout("asr.warmup", Map::new(), WARMUP_TIMEOUT)?;
        if !frame.ok() {
            return Err(remote(&frame));
        }
        Ok(frame.get_u64("warmup_ms").unwrap_or(0))
    }

    /// `asr.transcribe`：PCM16 16 kHz 单声道，body 走二进制。
    pub fn transcribe(&mut self, pcm: &[i16]) -> Result<Transcription, SidecarError> {
        let body = pcm_to_bytes(pcm);
        let mut params = Map::new();
        params.insert("sample_rate".to_string(), json!(SAMPLE_RATE));
        let frame =
            self.client
                .request_with_body("asr.transcribe", params, &body, TRANSCRIBE_TIMEOUT)?;
        if !frame.ok() {
            return Err(remote(&frame));
        }
        Ok(Transcription {
            text: frame.get_str("text").unwrap_or_default().to_string(),
            language: frame.get_str("language").unwrap_or_default().to_string(),
            audio_ms: frame.get_u64("audio_ms").unwrap_or(0),
            latency_ms: frame.get_u64("latency_ms").unwrap_or(0),
        })
    }

    /// `asr.unload`。
    pub fn unload(&mut self) -> Result<(), SidecarError> {
        let frame =
            self.client
                .request_timeout("asr.unload", Map::new(), Duration::from_secs(10))?;
        self.loaded = false;
        if frame.ok() {
            Ok(())
        } else {
            Err(remote(&frame))
        }
    }

    /// 停止 sidecar（`shutdown` → 2 s → kill）。
    pub fn stop(&mut self) {
        self.client.stop();
    }
}

impl Drop for AsrClient {
    fn drop(&mut self) {
        self.stop();
    }
}

fn remote(frame: &kb_sidecar::SidecarFrame) -> SidecarError {
    SidecarError::Remote {
        code: "E_ASR_FAILED",
        message: frame
            .error_message()
            .unwrap_or("ASR sidecar 返回失败")
            .to_string(),
    }
}

fn pcm_to_bytes(pcm: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(pcm.len() * 2);
    for sample in pcm {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// 注入抽象：生产用 [`kb_inject::Injector`]，测试用记录型假实现。
pub trait TextInjector {
    /// PTT 按下（段开始）时调用；生产实现用它记住目标窗口，测试默认不用关心。
    fn begin_segment(&mut self) {}

    fn inject_text(&mut self, text: &str) -> InjectOutcome;
}

impl TextInjector for kb_inject::Injector {
    fn begin_segment(&mut self) {
        kb_inject::Injector::begin_segment(self);
    }

    fn inject_text(&mut self, text: &str) -> InjectOutcome {
        self.inject(text, InjectMode::Auto)
    }
}

impl<T: TextInjector + ?Sized> TextInjector for Box<T> {
    fn begin_segment(&mut self) {
        (**self).begin_segment();
    }

    fn inject_text(&mut self, text: &str) -> InjectOutcome {
        (**self).inject_text(text)
    }
}

/// 一次「段 → 文本 → 注入」的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceResult {
    pub seg_id: i32,
    pub text: String,
    pub audio_ms: u64,
    pub latency_ms: u64,
    pub injected: Option<InjectOutcome>,
    /// 跳过/失败原因；`None` 表示识别并注入成功。
    pub note: Option<String>,
}

impl VoiceResult {
    pub fn skipped(seg_id: i32, reason: &str) -> Self {
        Self {
            seg_id,
            text: String::new(),
            audio_ms: 0,
            latency_ms: 0,
            injected: None,
            note: Some(reason.to_string()),
        }
    }

    pub fn failed(seg_id: i32, error: impl fmt::Display) -> Self {
        Self {
            seg_id,
            text: String::new(),
            audio_ms: 0,
            latency_ms: 0,
            injected: None,
            note: Some(error.to_string()),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.note.is_none()
    }
}

/// 段 → 识别 → 注入的串行编排。
#[derive(Debug)]
pub struct VoicePipeline<I: TextInjector> {
    asr: AsrClient,
    injector: I,
}

impl<I: TextInjector> VoicePipeline<I> {
    pub fn new(asr: AsrClient, injector: I) -> Self {
        Self { asr, injector }
    }

    pub fn asr(&self) -> &AsrClient {
        &self.asr
    }

    pub fn asr_mut(&mut self) -> &mut AsrClient {
        &mut self.asr
    }

    pub fn injector_mut(&mut self) -> &mut I {
        &mut self.injector
    }

    /// PTT 按下时调用：让注入器记住当前光标所在的窗口。
    pub fn begin_segment(&mut self) {
        self.injector.begin_segment();
    }

    /// 处理一个已封口的录音段：解码 → 识别 → 注入。
    pub fn handle_segment(&mut self, segment: &AudioSegment) -> VoiceResult {
        let seg_id = segment.seg_id;
        let (pcm, _missing) = kb_audio::decode_segment(segment);
        if pcm.is_empty() {
            return VoiceResult::skipped(seg_id, "empty_audio");
        }
        let transcription = match self.asr.transcribe(&pcm) {
            Ok(value) => value,
            Err(error) => return VoiceResult::failed(seg_id, error),
        };
        if transcription.text.trim().is_empty() {
            return VoiceResult {
                seg_id,
                text: String::new(),
                audio_ms: transcription.audio_ms,
                latency_ms: transcription.latency_ms,
                injected: None,
                note: Some("empty_text".to_string()),
            };
        }
        let injected = self.injector.inject_text(&transcription.text);
        VoiceResult {
            seg_id,
            text: transcription.text,
            audio_ms: transcription.audio_ms,
            latency_ms: transcription.latency_ms,
            injected: Some(injected),
            note: None,
        }
    }
}
