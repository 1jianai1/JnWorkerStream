//! 流式帧解析器。
//!
//! 语义与 `py_example/frame.py::FrameParser` 逐条对齐（`docs/interfaces.md` §3.4）：
//!
//! - 支持拆包/粘包：一帧可跨多次 [`FrameParser::feed`]，一次 `feed` 也可含多帧；
//! - 错帧重同步：版本不认、超长、CRC 错时**只丢 1 字节**再找同步头；
//! - 未知类型：整帧消费、只计 `unknown_type`，不影响后续帧；
//! - 解析器不是线程安全的：同一实例只应由一个任务持有。

use crate::frame::{
    is_known_type, Frame, HEADER_LEN, MAX_BUFFER, MAX_PAYLOAD, OVERHEAD, PROTO_VER, SYNC1, SYNC2,
};

/// 解析统计（`docs/interfaces.md` §3.4 的 7 个键）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParserStats {
    pub frames: u64,
    pub bytes: u64,
    pub crc_err: u64,
    pub oversize: u64,
    pub unknown_type: u64,
    pub resync: u64,
    pub overflow: u64,
}

/// 环形缓冲 + 游标的流式帧解析器。
#[derive(Debug)]
pub struct FrameParser {
    max_payload: usize,
    max_buffer: usize,
    buf: Vec<u8>,
    pos: usize,
    stats: ParserStats,
}

impl Default for FrameParser {
    fn default() -> Self {
        Self::new(MAX_PAYLOAD)
    }
}

impl FrameParser {
    /// 以协议上限创建解析器。
    ///
    /// `max_payload` 必须为正数（同参考实现），且会被裁到 [`MAX_PAYLOAD`]。
    pub fn new(max_payload: usize) -> Self {
        Self::with_limits(max_payload, MAX_BUFFER)
    }

    /// 指定链路 payload 上限与缓冲上限。
    pub fn with_limits(max_payload: usize, max_buffer: usize) -> Self {
        assert!(max_payload > 0, "max_payload 必须为正数");
        Self {
            max_payload: max_payload.min(MAX_PAYLOAD),
            max_buffer,
            buf: Vec::new(),
            pos: 0,
            stats: ParserStats::default(),
        }
    }

    /// 投喂任意长度的字节流，返回本次解析出的帧。
    pub fn feed(&mut self, data: &[u8]) -> Vec<Frame> {
        if data.is_empty() {
            return Vec::new();
        }
        self.stats.bytes += data.len() as u64;
        self.buf.extend_from_slice(data);
        self.parse()
    }

    /// 清空未完成的半帧（重连/换链路时调用），**不清统计**。
    pub fn reset(&mut self) {
        self.buf.clear();
        self.pos = 0;
    }

    /// 当前解析统计快照。
    pub fn stats(&self) -> ParserStats {
        self.stats
    }

    /// 缓冲中尚未解析的字节数。
    pub fn pending(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// 链路 payload 上限。
    pub fn max_payload(&self) -> usize {
        self.max_payload
    }

    fn drop_byte(&mut self, counter: fn(&mut ParserStats) -> &mut u64) {
        *counter(&mut self.stats) += 1;
        self.pos += 1;
    }

    fn parse(&mut self) -> Vec<Frame> {
        let mut frames = Vec::new();
        loop {
            let avail = self.buf.len() - self.pos;
            if avail < 2 {
                break;
            }

            let search_from = self.pos;
            let found = self.buf[search_from..]
                .windows(2)
                .position(|w| w[0] == SYNC1 && w[1] == SYNC2);

            let Some(rel) = found else {
                // 没有同步头：仅当末字节可能是半帧头时保留 1 B，其余计入 resync。
                let keep = usize::from(*self.buf.last().expect("avail >= 2") == SYNC1);
                let drop = avail - keep;
                if drop > 0 {
                    self.stats.resync += drop as u64;
                    self.pos += drop;
                }
                break;
            };

            if rel > 0 {
                self.stats.resync += rel as u64;
                self.pos += rel;
            }

            let avail = self.buf.len() - self.pos;
            if avail < HEADER_LEN {
                break;
            }

            let p = self.pos;
            let ver = self.buf[p + 2];
            let ty = self.buf[p + 3];
            let flags = self.buf[p + 4];
            let seq = u16::from_le_bytes([self.buf[p + 5], self.buf[p + 6]]);
            let length = u16::from_le_bytes([self.buf[p + 7], self.buf[p + 8]]) as usize;

            if ver != PROTO_VER {
                self.drop_byte(|s| &mut s.resync);
                continue;
            }
            if length > self.max_payload {
                self.drop_byte(|s| &mut s.oversize);
                continue;
            }

            let total = OVERHEAD + length;
            if avail < total {
                break;
            }

            let body = &self.buf[p + 2..p + HEADER_LEN + length];
            let got_crc = u16::from_le_bytes([
                self.buf[p + HEADER_LEN + length],
                self.buf[p + HEADER_LEN + length + 1],
            ]);
            let calc_crc = crate::crc::crc16(body);
            if got_crc != calc_crc {
                self.drop_byte(|s| &mut s.crc_err);
                continue;
            }

            let payload = self.buf[p + HEADER_LEN..p + HEADER_LEN + length].to_vec();
            self.pos += total;
            self.stats.frames += 1;

            if !is_known_type(ty) {
                self.stats.unknown_type += 1;
                continue;
            }

            frames.push(Frame {
                ver,
                ty,
                flags,
                seq,
                payload,
            });
        }

        self.compact();
        frames
    }

    fn compact(&mut self) {
        let pending = self.buf.len() - self.pos;
        if pending > self.max_buffer {
            // 正常情况下每次 feed 都会把完整帧全部消费掉；超限说明流已不可信。
            self.stats.overflow += 1;
            self.stats.resync += pending as u64;
            self.buf.clear();
            self.pos = 0;
            return;
        }
        if self.pos == 0 {
            return;
        }
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        } else if self.pos >= 4096 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{build_frame, build_test_payload, TYPE_TEST_DATA};

    fn frame(seq: u16, size: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..size)
            .map(|i| ((usize::from(seq) * 7 + i) & 0xFF) as u8)
            .collect();
        build_frame(TYPE_TEST_DATA, 0, seq, &build_test_payload(0, &data, 0)).unwrap()
    }

    #[test]
    fn split_into_three() {
        let raw = frame(1, 64);
        let mut parser = FrameParser::default();
        let mut frames = Vec::new();
        let cuts = [0, raw.len() / 3, 2 * raw.len() / 3, raw.len()];
        for pair in cuts.windows(2) {
            frames.extend(parser.feed(&raw[pair[0]..pair[1]]));
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].seq, 1);
        assert_eq!(parser.stats().frames, 1);
        assert_eq!(parser.stats().crc_err, 0);
    }

    #[test]
    fn three_frames_one_feed() {
        let mut raw = Vec::new();
        for i in 0..3 {
            raw.extend_from_slice(&frame(i, 64));
        }
        let mut parser = FrameParser::default();
        let frames = parser.feed(&raw);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(parser.stats().frames, 3);
    }

    #[test]
    fn one_dirty_byte() {
        let mut raw = vec![0x00];
        raw.extend_from_slice(&frame(7, 64));
        raw.extend_from_slice(&frame(8, 64));
        let mut parser = FrameParser::default();
        let frames = parser.feed(&raw);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [7, 8]);
        assert_eq!(parser.stats().resync, 1);
    }

    #[test]
    fn corrupt_payload_byte() {
        let mut bad = frame(3, 64);
        bad[10] ^= 0xFF;
        let mut raw = bad;
        raw.extend_from_slice(&frame(4, 64));
        let mut parser = FrameParser::default();
        let frames = parser.feed(&raw);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [4]);
        assert_eq!(parser.stats().crc_err, 1);
        assert_eq!(parser.stats().oversize, 0);
    }

    #[test]
    fn corrupt_crc() {
        let mut bad = frame(9, 64);
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        let mut raw = bad;
        raw.extend_from_slice(&frame(10, 64));
        let mut parser = FrameParser::default();
        let frames = parser.feed(&raw);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [10]);
        assert_eq!(parser.stats().crc_err, 1);
    }

    #[test]
    fn oversize_len() {
        let header = [0xA5, 0x5A, 0x01, 0x60, 0x00, 0x00, 0x00, 0x01, 0x04];
        let mut raw = header.to_vec();
        raw.extend_from_slice(&frame(11, 64));
        let mut parser = FrameParser::default();
        let frames = parser.feed(&raw);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [11]);
        assert_eq!(parser.stats().oversize, 1);
    }

    #[test]
    fn unknown_type_skipped() {
        let unknown = build_frame(0x61, 0, 1, b"xyz").unwrap();
        let mut raw = unknown;
        raw.extend_from_slice(&frame(2, 64));
        let mut parser = FrameParser::default();
        let frames = parser.feed(&raw);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [2]);
        assert_eq!(parser.stats().unknown_type, 1);
        assert_eq!(parser.stats().frames, 2);
    }

    #[test]
    fn garbage_only() {
        let mut parser = FrameParser::default();
        let garbage = [0x00u8, 0x11, 0x22, 0x33].repeat(100);
        let frames = parser.feed(&garbage);
        assert!(frames.is_empty());
        assert_eq!(parser.stats().frames, 0);
        assert!(parser.stats().resync > 0);
        assert_eq!(parser.pending(), 0);
    }

    #[test]
    fn partial_header_kept() {
        let raw = frame(1, 64);
        let mut parser = FrameParser::default();
        assert!(parser.feed(&raw[..8]).is_empty());
        assert_eq!(parser.stats().frames, 0);
        let frames = parser.feed(&raw[8..]);
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn incomplete_frame_waits() {
        let raw = frame(1, 64);
        let mut parser = FrameParser::default();
        assert!(parser.feed(&raw[..raw.len() - 1]).is_empty());
        assert_eq!(parser.pending(), raw.len() - 1);
        let frames = parser.feed(&raw[raw.len() - 1..]);
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn buffer_overflow_resyncs() {
        // 半帧待补齐时 pending 超过 max_buffer：整体丢弃并计 overflow + resync。
        let header = [0xA5, 0x5A, 0x01, 0x60, 0x00, 0x00, 0x00, 0x00, 0x04];
        let mut parser = FrameParser::with_limits(MAX_PAYLOAD, 8);
        let frames = parser.feed(&header);
        assert!(frames.is_empty());
        assert_eq!(parser.stats().overflow, 1);
        assert_eq!(parser.stats().resync, header.len() as u64);
        assert_eq!(parser.pending(), 0);
    }

    #[test]
    fn usb_payload_boundary_1013() {
        // USB CDC：max_payload = 1024 - 11 = 1013。TEST_DATA 载荷 = 5 B 头 + data，
        // 因此 data=1008 时 payload=1013（可解析），data=1009 时 payload=1014（oversize）。
        let ok = frame(1, 1008);
        let too_big = frame(2, 1009);
        let tail = frame(3, 8);

        let mut parser = FrameParser::with_limits(1013, MAX_BUFFER);
        let mut raw = Vec::new();
        raw.extend_from_slice(&ok);
        raw.extend_from_slice(&too_big);
        raw.extend_from_slice(&tail);
        let frames = parser.feed(&raw);

        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(parser.stats().oversize, 1);
        assert_eq!(parser.stats().frames, 2);
    }

    #[test]
    fn thousand_frames_random_slices() {
        let mut raw = Vec::new();
        for i in 0..1000u16 {
            raw.extend_from_slice(&frame(i, 64));
        }
        let mut parser = FrameParser::default();
        let mut got = Vec::new();
        for chunk in raw.chunks(137) {
            got.extend(parser.feed(chunk));
        }
        assert_eq!(got.len(), 1000);
        assert_eq!(parser.stats().frames, 1000);
        assert_eq!(parser.stats().crc_err, 0);
        assert_eq!(parser.stats().resync, 0);
        assert_eq!(parser.stats().overflow, 0);
    }
}
