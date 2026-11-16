//! `kb-voice` 集成测试：fake ASR sidecar + 记录型注入器，验证「段 → 识别 → 注入」。
//!
//! 需要本机 `python`（可用 `JNWS_PYTHON` 指定，例如 kbhost 环境）。

use std::path::PathBuf;
use std::process::Command;

use kb_audio::AudioSegment;
use kb_inject::InjectOutcome;
use kb_voice::{AsrClient, AsrLaunch, TextInjector, VoicePipeline};

fn python() -> String {
    std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".to_string())
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_asr_sidecar.py")
}

fn python_available() -> bool {
    Command::new(python())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn launch(env: &[(&str, &str)]) -> AsrLaunch {
    let mut launch = AsrLaunch::new(python(), fixture().display().to_string());
    for (key, value) in env {
        launch = launch.with_env(*key, *value);
    }
    launch
}

fn skip() -> bool {
    if python_available() {
        false
    } else {
        eprintln!("skip: python 不可用（设置 JNWS_PYTHON 指定解释器）");
        true
    }
}

#[derive(Debug, Default)]
struct RecordingInjector {
    texts: Vec<String>,
    fail: bool,
}

impl TextInjector for RecordingInjector {
    fn inject_text(&mut self, text: &str) -> InjectOutcome {
        self.texts.push(text.to_string());
        if self.fail {
            return InjectOutcome::failure("none", text.chars().count(), "inject disabled");
        }
        InjectOutcome::success("none", text.chars().count())
    }
}

fn segment(blocks: usize) -> AudioSegment {
    let mut segment = AudioSegment::new(1, 0);
    for seq in 0..blocks {
        segment.blocks.insert(seq as u16, vec![0u8; 160]);
    }
    segment.complete = true;
    segment
}

#[test]
fn client_loads_and_transcribes_pcm() {
    if skip() {
        return;
    }
    let mut client = AsrClient::start(launch(&[])).expect("启动 fake ASR sidecar");
    let load_ms = client.load(Some("fake-model"), 4).expect("asr.load");
    assert_eq!(load_ms, 1);
    assert!(client.is_loaded());

    let warmup_ms = client.warmup().expect("asr.warmup");
    assert_eq!(warmup_ms, 1);

    // 320 采样 = 640 B，fake 回 "len=<bytes>"。
    let result = client.transcribe(&vec![0i16; 320]).expect("asr.transcribe");
    assert_eq!(result.text, "len=640");
    assert_eq!(result.audio_ms, 20);
    assert_eq!(result.language, "zh");

    client.unload().expect("asr.unload");
    assert!(!client.is_loaded());
    client.stop();
}

#[test]
fn pipeline_decodes_and_injects() {
    if skip() {
        return;
    }
    let mut client = AsrClient::start(launch(&[])).expect("启动 fake ASR sidecar");
    client.load(None, 4).expect("asr.load");
    let mut pipeline = VoicePipeline::new(client, RecordingInjector::default());

    // 1 块 = 320 采样 = 640 B；fake 回 "len=640"。
    let result = pipeline.handle_segment(&segment(1));
    assert!(result.is_ok(), "note={:?}", result.note);
    assert_eq!(result.text, "len=640");
    assert_eq!(result.audio_ms, 20);
    assert!(result.injected.as_ref().expect("有注入结果").ok);
    assert_eq!(pipeline.injector_mut().texts, vec!["len=640".to_string()]);
}

#[test]
fn empty_text_is_skipped_without_injecting() {
    if skip() {
        return;
    }
    let mut client = AsrClient::start(launch(&[("FAKE_ASR_EMPTY", "1")])).expect("启动 sidecar");
    client.load(None, 4).expect("asr.load");
    let mut pipeline = VoicePipeline::new(client, RecordingInjector::default());

    let result = pipeline.handle_segment(&segment(2));
    assert!(!result.is_ok());
    assert_eq!(result.note.as_deref(), Some("empty_text"));
    assert!(pipeline.injector_mut().texts.is_empty());
}

#[test]
fn empty_segment_never_reaches_asr() {
    if skip() {
        return;
    }
    let client = AsrClient::start(launch(&[])).expect("启动 sidecar");
    let mut pipeline = VoicePipeline::new(client, RecordingInjector::default());
    let result = pipeline.handle_segment(&AudioSegment::new(9, 0));
    assert_eq!(result.note.as_deref(), Some("empty_audio"));
    assert!(pipeline.injector_mut().texts.is_empty());
}

#[test]
fn inject_failure_is_reported_but_text_survives() {
    if skip() {
        return;
    }
    let mut client = AsrClient::start(launch(&[("FAKE_ASR_TEXT", "你好")])).expect("启动 sidecar");
    client.load(None, 4).expect("asr.load");
    let injector = RecordingInjector {
        texts: Vec::new(),
        fail: true,
    };
    let mut pipeline = VoicePipeline::new(client, injector);

    let result = pipeline.handle_segment(&segment(1));
    assert!(result.is_ok(), "识别成功即 is_ok，注入失败看 injected");
    assert_eq!(result.text, "你好");
    let injected = result.injected.expect("有注入结果");
    assert!(!injected.ok);
    assert!(injected.error.unwrap().contains("inject disabled"));
}

#[test]
fn role_mismatch_is_rejected() {
    if skip() {
        return;
    }
    // 用 BLE 的 fake sidecar 冒充 ASR：role=ble，应被拒绝。
    let ble_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("kb-transport")
        .join("tests")
        .join("fixtures")
        .join("fake_ble_sidecar.py");
    if !ble_fixture.is_file() {
        eprintln!("skip: 找不到 BLE fake sidecar");
        return;
    }
    let launch = AsrLaunch::new(python(), ble_fixture.display().to_string());
    let error = AsrClient::start(launch).expect_err("role 不匹配应拒绝");
    assert_eq!(error.code(), "E_SIDECAR_PROTOCOL");
}
