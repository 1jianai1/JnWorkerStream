//! PTT 分段 / 缺块 / 旧模式兼容的等价断言（对应 `py_example/selftest.py` 语音段部分）。

use kb_audio::{
    build_audio_payload, decode_segment, segment_filename, wav_bytes, write_wav, VoiceSession,
};
use kb_protocol::{Frame, TYPE_AUDIO_DATA};

fn audio_frame(seq: u16) -> Frame {
    Frame {
        ver: 1,
        ty: TYPE_AUDIO_DATA,
        flags: 0,
        seq,
        payload: build_audio_payload(1, seq, &[0u8; 160]),
    }
}

#[test]
fn ptt_segments_by_ctl() {
    let mut session = VoiceSession::new();
    assert!(session.on_ctl(0, 7, 1_000).is_none());
    for seq in 0..3 {
        session.feed(&audio_frame(seq), 1_010 + u64::from(seq));
    }
    let finished = session.on_ctl(1, 7, 1_100).expect("应封段");
    assert_eq!(finished.seg_id, 7);
    assert_eq!(finished.received(), 3);
    assert!(finished.missing().is_empty());
    assert_eq!(session.stats(), (3, 3, vec![]));

    let (pcm, missing) = decode_segment(&finished);
    assert_eq!(pcm.len(), 3 * 320);
    assert!(missing.is_empty());
}

#[test]
fn new_start_discards_open_segment() {
    let mut session = VoiceSession::new();
    session.on_ctl(0, 1, 0);
    session.feed(&audio_frame(0), 1);
    session.on_ctl(0, 2, 2); // 上一段没有 ev=1，应丢弃
    assert_eq!(session.interrupted(), 1);

    session.feed(&audio_frame(0), 3);
    let finished = session.on_ctl(1, 2, 4).expect("应封段");
    assert_eq!(finished.seg_id, 2);
    assert_eq!(session.stats().0, 1);
}

#[test]
fn discard_open_on_disconnect() {
    let mut session = VoiceSession::new();
    session.on_ctl(0, 3, 0);
    session.feed(&audio_frame(0), 1);
    let discarded = session.discard_open("link closed").expect("应丢弃未封段");
    assert_eq!(discarded.seg_id, 3);
    assert!(discarded.interrupted);
    assert!(session.current().is_none());
    assert_eq!(session.stats().0, 0);
}

#[test]
fn legacy_without_ctl() {
    let mut session = VoiceSession::new();
    session.feed(&audio_frame(0), 0);
    session.feed(&audio_frame(1), 1);
    let legacy = session.finalize_current(2).expect("应封段");
    assert_eq!(legacy.seg_id, -1);
    assert_eq!(session.stats(), (2, 2, vec![]));
}

#[test]
fn missing_block_is_silence_and_reported() {
    let mut session = VoiceSession::new();
    session.on_ctl(0, 1, 0);
    session.feed(&audio_frame(0), 1);
    session.feed(&audio_frame(2), 2);
    let finished = session.on_ctl(1, 1, 3).expect("应封段");
    assert_eq!(finished.missing(), vec![1]);
    let (pcm, missing) = decode_segment(&finished);
    assert_eq!(pcm.len(), 3 * 320);
    assert_eq!(missing, vec![1]);
    assert_eq!(&pcm[320..640], &[0i16; 320]);
}

#[test]
fn duplicate_block_ignored() {
    let mut session = VoiceSession::new();
    session.on_ctl(0, 1, 0);
    let first = session.feed(&audio_frame(0), 1);
    assert_eq!(first.len(), 320);
    assert!(session.feed(&audio_frame(0), 2).is_empty());
    assert_eq!(session.stats().0, 1);
}

#[test]
fn segment_filename_matches_reference() {
    assert_eq!(segment_filename("out.wav", 3, true), "out_seg3.wav");
    assert_eq!(
        segment_filename("logs/voice/seg.wav", 12, true).replace('\\', "/"),
        "logs/voice/seg_seg12.wav"
    );
    assert_eq!(segment_filename("out.wav", 3, false), "out.wav");
    assert_eq!(segment_filename("out", 1, true), "out_seg1.wav");
}

#[test]
fn wav_written_to_disk_is_valid_riff() {
    let path = std::env::temp_dir().join(format!("jnws-audio-{}.wav", std::process::id()));
    let pcm = vec![123i16; 320];
    let bytes = wav_bytes(&pcm);
    let written = write_wav(&path, &pcm).expect("写 WAV");
    assert_eq!(written, bytes.len() as u64);
    let read = std::fs::read(&path).expect("读 WAV");
    assert_eq!(read, bytes);
    assert_eq!(&read[0..4], b"RIFF");
    let _ = std::fs::remove_file(&path);
}
