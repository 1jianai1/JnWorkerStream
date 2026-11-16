//! 端到端手验工具：WAV → 真实 ASR sidecar → 文本注入。
//!
//! ```powershell
//! cargo run -p kb-voice --example voice_e2e -- target/hw/seg5_05.wav `
//!     models/faster-whisper-small <ready-marker>
//! ```
//!
//! `<ready-marker>` 可选：出现该文件后再注入（用于等目标窗口拿到焦点）。

use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use kb_inject::{InjectMode, Injector};
use kb_voice::{AsrClient, AsrLaunch};

fn read_wav(path: &Path) -> Vec<i16> {
    let bytes = fs::read(path).expect("读取 WAV 失败");
    assert!(bytes.len() > 44, "WAV 太短");
    assert_eq!(&bytes[0..4], b"RIFF", "不是 RIFF/WAV");
    assert_eq!(&bytes[8..12], b"WAVE", "不是 WAVE");
    let pcm = &bytes[44..];
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let wav = Path::new(
        args.get(1)
            .map(String::as_str)
            .unwrap_or("target/hw/seg5_05.wav"),
    );
    let model = args.get(2).cloned();
    let marker = args.get(3).map(String::from);

    let pcm = read_wav(wav);
    let audio_ms = pcm.len() as u64 * 1000 / 16_000;
    println!(
        "wav={} samples={} audio_ms={audio_ms}",
        wav.display(),
        pcm.len()
    );

    let python = std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".to_string());
    let script = std::env::var("JNWS_ASR_SCRIPT").unwrap_or_else(|_| {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.pop();
        path.join("sidecars")
            .join("asr")
            .join("main.py")
            .display()
            .to_string()
    });
    let launch = AsrLaunch::new(python, script);
    let mut client = AsrClient::start(launch).expect("启动 ASR sidecar 失败");

    let load_ms = client.load(model.as_deref(), 8).expect("asr.load 失败");
    let warmup_ms = client.warmup().expect("asr.warmup 失败");
    println!("load_ms={load_ms} warmup_ms={warmup_ms}");

    let started = Instant::now();
    let result = client.transcribe(&pcm).expect("asr.transcribe 失败");
    println!(
        "text={:?} lang={} audio_ms={} latency_ms={} wall_ms={}",
        result.text,
        result.language,
        result.audio_ms,
        result.latency_ms,
        started.elapsed().as_millis()
    );
    client.stop();

    if result.text.trim().is_empty() {
        println!("inject=skipped(empty_text)");
        return;
    }

    if let Some(marker) = marker {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !Path::new(&marker).exists() && Instant::now() < deadline {
            sleep(Duration::from_millis(50));
        }
        sleep(Duration::from_millis(400));
    }

    let mut injector = Injector::new().with_per_char_ms(2);
    let outcome = injector.inject(&result.text, InjectMode::Auto);
    println!(
        "inject=ok:{} method={} chars={} error={:?}",
        outcome.ok, outcome.method, outcome.chars, outcome.error
    );
}
