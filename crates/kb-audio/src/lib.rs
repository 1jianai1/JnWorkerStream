//! 音频核心（P2a）：IMA-ADPCM 解码、WAV 落盘、`AUDIO_CTL` 驱动的 PTT 分段。
//!
//! 与参考实现 `py_example/audio_rx.py`、`py_example/voice/session.py` 对齐：
//!
//! - 16 kHz / 单声道 / 16 bit，20 ms 一块（160 B ADPCM ↔ 320 采样）；
//! - 解码器状态跨块连续，**必须从 `block_seq = 0` 顺解**；
//! - 缺块用 320 采样静音补齐并记录 `missing`，不拼接、不漂移；
//! - 一次按键 = 一个 `seg_id`；新段出现即丢弃未封口段（`interrupted`）。
//!
//! P2c 的 ASR/注入不在本 crate。

pub mod adpcm;
pub mod payload;
pub mod session;
pub mod wav;

pub use adpcm::{AdpcmDecoder, INDEX_TABLE, STEP_TABLE};
pub use payload::{
    build_audio_ctl, build_audio_payload, parse_audio_ctl, parse_audio_payload, AudioCtl,
    AudioPayload,
};
pub use session::{
    decode_blocks, decode_segment, segment_filename, AudioSegment, VoiceSession, MAX_SEGMENTS,
};
pub use wav::{wav_bytes, write_wav};
