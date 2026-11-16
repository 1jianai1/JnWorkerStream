//! `A5 5A` 线上帧协议（P2a）。
//!
//! 与参考实现 `py_example/frame.py` 逐条对齐，语义冻结于 `docs/interfaces.md` §3：
//! 组帧、CRC-16/CCITT-FALSE、流式解析（粘包/半包/脏字节重同步）、7 项解析统计。
//!
//! 本 crate 只做字节与帧的转换，不依赖任何异步运行时、平台 API 或序列化库；
//! 传输层（`kb-transport`）只把它当作“整帧字节”的构建者与消费者。

#![deny(missing_debug_implementations)]

pub mod crc;
pub mod frame;
pub mod parser;

pub use crc::crc16;
pub use frame::{
    build_frame, build_test_payload, is_known_type, Frame, ProtocolError, AUDIO_FMT_ADPCM,
    AUDIO_FMT_PCM16, BLOCK_BYTES, BLOCK_SAMPLES, CHANNELS, FRAME_FLAG_ACK, FRAME_FLAG_RETRANSMIT,
    HEADER_LEN, KNOWN_TYPES, MAX_BUFFER, MAX_PAYLOAD, OVERHEAD, PROTO_VER, SAMPLE_RATE, SYNC1,
    SYNC2, TEST_FLAG_ECHO, TYPE_AUDIO_CTL, TYPE_AUDIO_DATA, TYPE_TEST_DATA,
};
pub use parser::{FrameParser, ParserStats};
