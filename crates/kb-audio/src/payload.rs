//! `AUDIO_DATA` / `AUDIO_CTL` 载荷解析（`docs/interfaces.md` §3.2）。

/// `AUDIO_DATA` 载荷视图：`fmt(1) + block_seq(u16 LE) + data[n]`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioPayload<'a> {
    pub fmt: u8,
    pub block_seq: u16,
    pub data: &'a [u8],
}

/// 解析 `AUDIO_DATA` 载荷；不足 3 B 返回 `None`。
pub fn parse_audio_payload(payload: &[u8]) -> Option<AudioPayload<'_>> {
    if payload.len() < 3 {
        return None;
    }
    Some(AudioPayload {
        fmt: payload[0],
        block_seq: u16::from_le_bytes([payload[1], payload[2]]),
        data: &payload[3..],
    })
}

/// `AUDIO_CTL` 载荷：`ev(1) + seg_id(u16 LE)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCtl {
    pub event: u8,
    pub seg_id: u16,
}

/// 解析 `AUDIO_CTL` 载荷；不足 3 B 返回 `None`。
pub fn parse_audio_ctl(payload: &[u8]) -> Option<AudioCtl> {
    if payload.len() < 3 {
        return None;
    }
    Some(AudioCtl {
        event: payload[0],
        seg_id: u16::from_le_bytes([payload[1], payload[2]]),
    })
}

/// 构造 `AUDIO_CTL` 载荷（测试/下发用）。
pub fn build_audio_ctl(event: u8, seg_id: u16) -> [u8; 3] {
    let bytes = seg_id.to_le_bytes();
    [event, bytes[0], bytes[1]]
}

/// 构造 `AUDIO_DATA` 载荷。
pub fn build_audio_payload(fmt: u8, block_seq: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + data.len());
    out.push(fmt);
    out.extend_from_slice(&block_seq.to_le_bytes());
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_payload_roundtrip() {
        let raw = build_audio_payload(1, 7, &[0xAB; 160]);
        let parsed = parse_audio_payload(&raw).expect("应可解析");
        assert_eq!(parsed.fmt, 1);
        assert_eq!(parsed.block_seq, 7);
        assert_eq!(parsed.data.len(), 160);
        assert_eq!(parsed.data[0], 0xAB);
    }

    #[test]
    fn short_payloads_are_rejected() {
        assert!(parse_audio_payload(&[1, 0]).is_none());
        assert!(parse_audio_ctl(&[0, 0]).is_none());
    }

    #[test]
    fn audio_ctl_roundtrip() {
        let raw = build_audio_ctl(1, 0x1234);
        let parsed = parse_audio_ctl(&raw).expect("应可解析");
        assert_eq!(parsed.event, 1);
        assert_eq!(parsed.seg_id, 0x1234);
    }
}
