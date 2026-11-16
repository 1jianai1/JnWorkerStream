//! `kb-app` 的核心测试：喂合成音频帧 → 分段 → fake ASR sidecar → `voice.result`。
//!
//! 需要本机 `python`（可用 `JNWS_PYTHON` 指定）。

use std::path::PathBuf;
use std::process::Command;

use kb_app::{Core, CoreEvent, LinkState};
use kb_audio::{build_audio_ctl, build_audio_payload};
use kb_inject::InjectOutcome;
use kb_protocol::{build_frame, AUDIO_FMT_ADPCM, TYPE_AUDIO_CTL, TYPE_AUDIO_DATA};
use kb_transport::SidecarLaunch;
use kb_voice::{AsrClient, AsrLaunch, TextInjector};

fn python() -> String {
    std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".to_string())
}

fn python_available() -> bool {
    Command::new(python())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn repo_path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.join(relative)
}

#[derive(Debug, Default)]
struct RecordingInjector {
    texts: Vec<String>,
}

impl TextInjector for RecordingInjector {
    fn inject_text(&mut self, text: &str) -> InjectOutcome {
        self.texts.push(text.to_string());
        InjectOutcome::success("none", text.chars().count())
    }
}

fn core() -> Core<RecordingInjector> {
    Core::new(SidecarLaunch::default())
}

fn asr_client() -> AsrClient {
    let fixture = repo_path("crates/kb-voice/tests/fixtures/fake_asr_sidecar.py");
    AsrClient::start(AsrLaunch::new(python(), fixture.display().to_string()))
        .expect("启动 fake ASR sidecar")
}

fn audio_ctl(event: u8, seg_id: u16) -> Vec<u8> {
    build_frame(TYPE_AUDIO_CTL, 0, seg_id, &build_audio_ctl(event, seg_id)).expect("组帧")
}

fn audio_data(block_seq: u16) -> Vec<u8> {
    let payload = build_audio_payload(AUDIO_FMT_ADPCM, block_seq, &[0u8; 160]);
    build_frame(TYPE_AUDIO_DATA, 0, block_seq, &payload).expect("组帧")
}

#[test]
fn link_state_starts_idle() {
    let core = core();
    let snapshot = core.snapshot();
    assert_eq!(snapshot.link.state, LinkState::Idle.as_str());
    assert_eq!(snapshot.voice.segments, 0);
    assert!(!snapshot.voice.enabled);
}

#[test]
fn frames_without_voice_produce_frame_batch_only() {
    if !python_available() {
        return;
    }
    let mut core = core();
    let _ = core.ingest(&audio_ctl(0, 1));
    // 帧批由 `poll()` 统一 flush（UI 侧 40 ms 节流的位置）。
    let events = core.poll();
    assert!(events
        .iter()
        .any(|event| matches!(event, CoreEvent::FrameBatch(_))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, CoreEvent::VoiceResult(_))));

    let snapshot = core.snapshot();
    assert_eq!(snapshot.parser.frames, 1);
}

#[test]
fn segment_goes_through_asr_and_injection() {
    if !python_available() {
        eprintln!("skip: python 不可用");
        return;
    }
    let mut core = core();
    core.attach_voice(asr_client(), RecordingInjector::default());

    // 开段 → 2 个数据块 → 封段；fake ASR 回 "len=<body 字节>"。
    let mut events = Vec::new();
    events.extend(core.ingest(&audio_ctl(0, 7)));
    events.extend(core.ingest(&audio_data(0)));
    events.extend(core.ingest(&audio_data(1)));
    events.extend(core.ingest(&audio_ctl(1, 7)));

    let voice: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            CoreEvent::VoiceResult(payload) => Some(payload.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(voice.len(), 1, "封段应产出一次 voice.result");
    let result = &voice[0];
    assert_eq!(result.seg_id, 7);
    // 2 块 × 320 采样 × 2 B = 1280 B。
    assert_eq!(result.text, "len=1280");
    assert_eq!(result.audio_ms, 40);
    assert!(result.inject.ok, "注入应成功：{:?}", result.inject);

    let snapshot = core.snapshot();
    assert_eq!(snapshot.voice.segments, 1);
    assert_eq!(snapshot.voice.failures, 0);
    assert_eq!(snapshot.voice.last_text.as_deref(), Some("len=1280"));
    assert_eq!(snapshot.parser.frames, 4);
    assert_eq!(snapshot.parser.crc_err, 0);

    let event = events
        .iter()
        .find(|event| matches!(event, CoreEvent::VoiceResult(_)))
        .expect("voice.result");
    assert_eq!(event.name(), "voice.result");
    let payload = event.payload();
    assert_eq!(payload["seg_id"], 7);
    assert_eq!(payload["text"], "len=1280");
    assert_eq!(payload["inject"]["ok"], true);
}

#[test]
fn disconnect_resets_link_snapshot() {
    let mut core = core();
    // 没连接时 disconnect 也应幂等，并把状态归到 idle。
    core.disconnect().expect("disconnect 应幂等");
    assert_eq!(core.snapshot().link.state, LinkState::Idle.as_str());
}

#[test]
fn event_names_match_tauri_contract() {
    let core = core();
    let _ = core;
    assert_eq!(LinkState::Connected.as_str(), "connected");
    assert_eq!(LinkState::Reconnecting.as_str(), "reconnecting");
}

#[test]
fn snapshot_serializes_with_frozen_field_names() {
    let core = core();
    let json = serde_json::to_value(core.snapshot()).expect("snapshot 序列化");
    assert!(json.get("revision").is_some());
    assert!(json.get("parser").is_some());
    assert!(json["parser"].get("crc_err").is_some());
    assert!(json["transport"].get("rx_bytes").is_some());
    assert_eq!(json["link"]["state"], "idle");
    assert_eq!(json["voice"]["enabled"], false);
}
