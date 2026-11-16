//! 真机端到端（默认忽略）：按住设备 PTT 说话 → ADPCM 分段 → 真实 ASR → 文本。
//!
//! 不需要窗口焦点，因此可在任意终端跑；注入结果只打印、不真正落盘到别的程序。
//!
//! ```powershell
//! $env:JNWS_PYTHON='D:\Miniconda\envs\kbhost\python.exe'
//! $env:JNWS_ASR_MODEL="$PWD\models\faster-whisper-small"
//! $env:JNWS_LISTEN_SECS='60'
//! cargo test -p kb-voice --test voice_hardware -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::time::{Duration, Instant};

use kb_audio::VoiceSession;
use kb_inject::InjectOutcome;
use kb_protocol::{
    build_frame, build_test_payload, FrameParser, MAX_PAYLOAD, TYPE_AUDIO_CTL, TYPE_AUDIO_DATA,
    TYPE_TEST_DATA,
};
use kb_transport::{BleCfg, BleTransport, SidecarLaunch, Transport, TransportCfg, TransportEvent};
use kb_voice::{AsrClient, AsrLaunch, TextInjector, VoicePipeline};

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

fn listen_secs() -> u64 {
    std::env::var("JNWS_LISTEN_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60)
}

/// `cargo test` 的工作目录是 crate 目录，脚本路径必须按清单目录拼成绝对路径。
fn repo_path(relative: &str) -> String {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.join(relative).display().to_string()
}

/// 只记录、不向任何窗口发送按键；真机验证时避免干扰当前终端。
#[derive(Debug, Default)]
struct NoopInjector {
    texts: Vec<String>,
}

impl TextInjector for NoopInjector {
    fn inject_text(&mut self, text: &str) -> InjectOutcome {
        self.texts.push(text.to_string());
        InjectOutcome::success("none", text.chars().count())
    }
}

#[test]
#[ignore = "需要真实 BLE 设备、人工按 PTT，且本机有 ASR 模型"]
fn real_ptt_to_text() {
    let model = std::env::var("JNWS_ASR_MODEL").ok();
    let script =
        std::env::var("JNWS_ASR_SCRIPT").unwrap_or_else(|_| repo_path("sidecars/asr/main.py"));
    assert!(
        std::path::Path::new(&script).is_file(),
        "ASR 脚本不存在：{script}（cargo test 的 cwd 是 crate 目录，别用相对路径）"
    );

    let mut client = AsrClient::start(AsrLaunch::new(
        std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".into()),
        script,
    ))
    .expect("启动 ASR sidecar 失败");
    let load_ms = client.load(model.as_deref(), 8).expect("asr.load 失败");
    let warmup_ms = client.warmup().expect("asr.warmup 失败");
    println!("asr load={load_ms}ms warmup={warmup_ms}ms");

    let mut transport = BleTransport::new(SidecarLaunch::default());
    let cfg = BleCfg {
        address: Some(env_or("JNWS_BLE_ADDRESS", "5C:FD:52:80:0C:3C")),
        name: Some(env_or("JNWS_BLE_NAME", "SF32-KB-0C3C")),
        paired_only: true,
        scan_timeout_ms: 6_000,
    };
    let started = Instant::now();
    let link = transport
        .open(&TransportCfg::Ble(cfg))
        .expect("连接失败：确认键盘已开机并在 BLE 模式");
    println!(
        "connected at {:?}: {} mtu={}",
        started.elapsed(),
        link.address,
        link.mtu
    );

    // 设备自定义链路需要主机先发一帧（与 listener 用例一致）。
    let probe =
        build_frame(TYPE_TEST_DATA, 0, 0, &build_test_payload(0, &[0u8; 8], 0)).expect("组帧");
    let _ = transport.write(&probe);

    let mut pipeline = VoicePipeline::new(client, NoopInjector::default());
    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let mut session = VoiceSession::new();
    let mut segments = 0usize;
    let mut texts: Vec<String> = Vec::new();

    let wait = listen_secs();
    println!("listening {wait}s — 请按住 PTT 说一句话再松开");
    let deadline = Instant::now() + Duration::from_secs(wait);
    while Instant::now() < deadline {
        for event in transport.events() {
            let TransportEvent::Bytes(chunk) = event else {
                continue;
            };
            for frame in parser.feed(&chunk) {
                let elapsed = started.elapsed().as_millis() as u64;
                match frame.ty {
                    TYPE_AUDIO_CTL => {
                        if let Some(segment) = session.on_ctl_frame(&frame, elapsed) {
                            segments += 1;
                            let result = pipeline.handle_segment(&segment);
                            println!(
                                "segment seg={} audio={}ms text={:?} asr={}ms note={:?}",
                                result.seg_id,
                                result.audio_ms,
                                result.text,
                                result.latency_ms,
                                result.note
                            );
                            if !result.text.is_empty() {
                                texts.push(result.text);
                            }
                        }
                    }
                    TYPE_AUDIO_DATA => {
                        let _ = session.feed(&frame, elapsed);
                    }
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    if let Some(discarded) = session.discard_open("listen timeout") {
        println!(
            "discarded open segment seg={} blocks={}",
            discarded.seg_id,
            discarded.received()
        );
    }
    println!("summary segments={segments} texts={texts:?}");
    assert!(
        segments > 0,
        "{wait} s 内没有完成任何语音段：确认按下并松开了 PTT"
    );
}
