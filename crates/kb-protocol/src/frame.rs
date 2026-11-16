//! 帧常量、帧结构、组帧与载荷辅助函数。

use std::fmt;

use crate::crc::crc16;

/// 帧同步字第一字节。
pub const SYNC1: u8 = 0xA5;
/// 帧同步字第二字节。
pub const SYNC2: u8 = 0x5A;
/// 协议版本，当前只接受 `0x01`。
pub const PROTO_VER: u8 = 0x01;

/// 帧头长度：`SYNC(2) + ver(1) + type(1) + flags(1) + seq(2) + len(2)`。
pub const HEADER_LEN: usize = 9;
/// CRC 长度（`u16 LE`）。
pub const CRC_LEN: usize = 2;
/// 整帧固定开销：`HEADER_LEN + CRC_LEN`。
pub const OVERHEAD: usize = HEADER_LEN + CRC_LEN;

/// 协议层 payload 绝对上限；链路层上限见 ADR-011。
pub const MAX_PAYLOAD: usize = 1024;
/// 解析缓冲上限，防止脏数据把内存吃满。
pub const MAX_BUFFER: usize = 64 * 1024;

/// 音频数据帧（设备 → 主机）。
pub const TYPE_AUDIO_DATA: u8 = 0x01;
/// 音频控制帧（双向，PTT）。
pub const TYPE_AUDIO_CTL: u8 = 0x02;
/// 回环测试帧（双向）。
pub const TYPE_TEST_DATA: u8 = 0x60;

/// 解析器认识的帧类型；其余类型整帧消费后只计 `unknown_type`。
pub const KNOWN_TYPES: [u8; 3] = [TYPE_AUDIO_DATA, TYPE_AUDIO_CTL, TYPE_TEST_DATA];

/// 帧头 `flags`：需要 ACK。
pub const FRAME_FLAG_ACK: u8 = 0x01;
/// 帧头 `flags`：重传。
pub const FRAME_FLAG_RETRANSMIT: u8 = 0x04;
/// `TEST_DATA` 载荷内 `flags`：设备回显。
pub const TEST_FLAG_ECHO: u8 = 0x01;

// 音频参数（`docs/interfaces.md` §3.2）。
/// 采样率。
pub const SAMPLE_RATE: u32 = 16_000;
/// 声道数。
pub const CHANNELS: u8 = 1;
/// 每块采样点数（20 ms）。
pub const BLOCK_SAMPLES: usize = 320;
/// 每块 ADPCM 字节数（20 ms）。
pub const BLOCK_BYTES: usize = 160;
/// `AUDIO_DATA.fmt`：PCM16。
pub const AUDIO_FMT_PCM16: u8 = 0;
/// `AUDIO_DATA.fmt`：IMA-ADPCM。
pub const AUDIO_FMT_ADPCM: u8 = 1;

/// 组帧/校验失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    /// payload 超过 [`MAX_PAYLOAD`]；调用方应先切分。
    PayloadTooLarge { len: usize, max: usize },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { len, max } => {
                write!(f, "payload {len} B 超过协议上限 {max} B，应由上层切分")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

/// 一条通过 CRC 校验的帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub ver: u8,
    pub ty: u8,
    pub flags: u8,
    pub seq: u16,
    pub payload: Vec<u8>,
}

impl Frame {
    /// 类型名；未知类型返回 `None`。
    pub fn type_name(&self) -> Option<&'static str> {
        type_name(self.ty)
    }

    /// 是否为 `TEST_DATA` 回显帧（载荷 `flags` bit0）。
    pub fn is_echo(&self) -> bool {
        self.ty == TYPE_TEST_DATA
            && self.payload.len() >= 5
            && self.payload[4] & TEST_FLAG_ECHO != 0
    }

    /// `TEST_DATA` 的设备时间戳（`dev_ms`）。
    pub fn dev_ms(&self) -> Option<u32> {
        if self.ty == TYPE_TEST_DATA && self.payload.len() >= 5 {
            Some(u32::from_le_bytes([
                self.payload[0],
                self.payload[1],
                self.payload[2],
                self.payload[3],
            ]))
        } else {
            None
        }
    }

    /// `TEST_DATA` 的数据段（跳过 `dev_ms + flags`）；其它类型为空。
    pub fn data(&self) -> &[u8] {
        if self.ty == TYPE_TEST_DATA && self.payload.len() >= 5 {
            &self.payload[5..]
        } else {
            &[]
        }
    }

    /// 整帧字节数（含同步字与 CRC）。
    pub fn len_bytes(&self) -> usize {
        OVERHEAD + self.payload.len()
    }
}

/// 类型码对应的名称。
pub fn type_name(ty: u8) -> Option<&'static str> {
    match ty {
        TYPE_AUDIO_DATA => Some("AUDIO_DATA"),
        TYPE_AUDIO_CTL => Some("AUDIO_CTL"),
        TYPE_TEST_DATA => Some("TEST_DATA"),
        _ => None,
    }
}

/// 是否为已知类型。
pub fn is_known_type(ty: u8) -> bool {
    KNOWN_TYPES.contains(&ty)
}

/// 组装一帧完整字节（含同步头与 CRC）。
///
/// 与参考实现一致：多字节小端，CRC 覆盖 `frame[2 .. 9+len)`。
pub fn build_frame(ty: u8, flags: u8, seq: u16, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(ProtocolError::PayloadTooLarge {
            len: payload.len(),
            max: MAX_PAYLOAD,
        });
    }
    let mut out = Vec::with_capacity(OVERHEAD + payload.len());
    out.push(SYNC1);
    out.push(SYNC2);
    out.push(PROTO_VER);
    out.push(ty);
    out.push(flags);
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    out.extend_from_slice(payload);
    let crc = crc16(&out[2..]);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// 组装 `TEST_DATA` 载荷：`dev_ms(4 LE) + flags(1) + data(n)`。
pub fn build_test_payload(dev_ms: u32, data: &[u8], flags: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + data.len());
    out.extend_from_slice(&dev_ms.to_le_bytes());
    out.push(flags);
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::FrameParser;

    fn test_frame(seq: u16, size: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..size)
            .map(|i| ((seq as usize * 7 + i) & 0xFF) as u8)
            .collect();
        build_frame(TYPE_TEST_DATA, 0, seq, &build_test_payload(0, &data, 0)).unwrap()
    }

    #[test]
    fn roundtrip() {
        let payload = build_test_payload(123_456, "你好, 键盘".as_bytes(), 0);
        let raw = build_frame(TYPE_TEST_DATA, 0, 0x1234, &payload).unwrap();
        assert_eq!(raw.len(), OVERHEAD + payload.len());
        assert_eq!(&raw[..2], &[SYNC1, SYNC2]);

        let mut parser = FrameParser::new(MAX_PAYLOAD);
        let frames = parser.feed(&raw);
        assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        assert_eq!(frame.seq, 0x1234);
        assert_eq!(frame.ty, TYPE_TEST_DATA);
        assert_eq!(frame.payload, payload);
        assert_eq!(frame.data(), "你好, 键盘".as_bytes());
        assert_eq!(frame.dev_ms(), Some(123_456));
        assert!(!frame.is_echo());
        assert_eq!(frame.len_bytes(), raw.len());
    }

    #[test]
    fn reject_oversize_payload() {
        let payload = vec![0u8; MAX_PAYLOAD + 1];
        assert_eq!(
            build_frame(TYPE_TEST_DATA, 0, 0, &payload),
            Err(ProtocolError::PayloadTooLarge {
                len: MAX_PAYLOAD + 1,
                max: MAX_PAYLOAD
            })
        );
    }

    #[test]
    fn payload_boundary_at_protocol_limit() {
        let payload = vec![0x5Au8; MAX_PAYLOAD];
        let raw = build_frame(TYPE_TEST_DATA, 0, 1, &payload).expect("1024 B 应可组帧");
        assert_eq!(raw.len(), OVERHEAD + MAX_PAYLOAD);
        let mut parser = FrameParser::new(MAX_PAYLOAD);
        let frames = parser.feed(&raw);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].payload.len(), MAX_PAYLOAD);
    }

    #[test]
    fn echo_flag() {
        let raw = build_frame(
            TYPE_TEST_DATA,
            0,
            5,
            &build_test_payload(1, b"abc", TEST_FLAG_ECHO),
        )
        .unwrap();
        let mut parser = FrameParser::new(MAX_PAYLOAD);
        let frames = parser.feed(&raw);
        assert!(frames[0].is_echo());
        assert_eq!(frames[0].data(), b"abc");
    }

    #[test]
    fn test_frame_helper_is_inc_pattern() {
        let raw = test_frame(3, 8);
        let mut parser = FrameParser::new(MAX_PAYLOAD);
        let frames = parser.feed(&raw);
        assert_eq!(frames[0].data(), &[21, 22, 23, 24, 25, 26, 27, 28]);
    }
}
