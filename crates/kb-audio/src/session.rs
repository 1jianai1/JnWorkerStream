//! PTT 分段会话：把 `AUDIO_CTL + AUDIO_DATA` 还原成一段段录音。

use std::collections::BTreeMap;
use std::path::Path;

use kb_protocol::{
    Frame, AUDIO_FMT_ADPCM, BLOCK_BYTES, BLOCK_SAMPLES, TYPE_AUDIO_CTL, TYPE_AUDIO_DATA,
};

use crate::adpcm::AdpcmDecoder;
use crate::payload::parse_audio_payload;

/// 内存中最多保留的段数（含当前未封口段）。
pub const MAX_SEGMENTS: usize = 8;

/// 一次按键对应的录音段。
#[derive(Debug, Clone)]
pub struct AudioSegment {
    pub seg_id: i32,
    pub blocks: BTreeMap<u16, Vec<u8>>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub complete: bool,
    pub interrupted: bool,
}

impl AudioSegment {
    pub fn new(seg_id: i32, started_at_ms: u64) -> Self {
        Self {
            seg_id,
            blocks: BTreeMap::new(),
            started_at_ms,
            ended_at_ms: None,
            complete: false,
            interrupted: false,
        }
    }

    pub fn received(&self) -> usize {
        self.blocks.len()
    }

    /// 应有块数：`max(block_seq) + 1`。
    pub fn total(&self) -> usize {
        self.blocks
            .keys()
            .next_back()
            .map_or(0, |last| usize::from(*last) + 1)
    }

    pub fn missing(&self) -> Vec<u16> {
        let total = self.total();
        (0..total as u16)
            .filter(|seq| !self.blocks.contains_key(seq))
            .collect()
    }

    pub fn duration_ms(&self) -> u64 {
        self.total() as u64 * 20
    }

    pub fn missing_ratio(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            self.missing().len() as f64 / total as f64
        }
    }
}

/// 从 `seq=0` 顺序解码；缺块插入静音并返回缺失列表。
pub fn decode_blocks(blocks: &BTreeMap<u16, Vec<u8>>) -> (Vec<i16>, Vec<u16>) {
    let Some(last) = blocks.keys().next_back() else {
        return (Vec::new(), Vec::new());
    };
    let end = usize::from(*last) + 1;
    let mut decoder = AdpcmDecoder::new();
    let mut pcm = Vec::with_capacity(end * BLOCK_SAMPLES);
    let mut missing = Vec::new();
    for index in 0..end {
        let seq = index as u16;
        match blocks.get(&seq) {
            Some(chunk) => decoder.decode_block_into(chunk, &mut pcm),
            None => {
                missing.push(seq);
                pcm.extend(std::iter::repeat_n(0i16, BLOCK_SAMPLES));
            }
        }
    }
    (pcm, missing)
}

/// 解码一段。
pub fn decode_segment(segment: &AudioSegment) -> (Vec<i16>, Vec<u16>) {
    decode_blocks(&segment.blocks)
}

/// 按段生成 WAV 文件名：`out.wav` → `out_seg3.wav`；`multi=false` 时原样返回。
pub fn segment_filename(base: &str, seg_id: i32, multi: bool) -> String {
    if !multi {
        return base.to_string();
    }
    let path = Path::new(base);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wav".to_string());
    let name = format!("{stem}_seg{seg_id}.{ext}");
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name).display().to_string(),
        _ => name,
    }
}

/// 按设备控制帧管理录音段，并提供实时 PCM 给播放。
#[derive(Debug)]
pub struct VoiceSession {
    segments: Vec<AudioSegment>,
    open: bool,
    ptt_mode: bool,
    decoder: AdpcmDecoder,
    next_seq: u16,
    bad: u64,
    interrupted: u64,
    last_ctl_ms: u64,
    last_ctl_event: Option<u8>,
}

impl Default for VoiceSession {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceSession {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
            open: false,
            ptt_mode: false,
            decoder: AdpcmDecoder::new(),
            next_seq: 0,
            bad: 0,
            interrupted: 0,
            last_ctl_ms: 0,
            last_ctl_event: None,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn segments(&self) -> &[AudioSegment] {
        &self.segments
    }

    /// 当前未封口段。
    pub fn current(&self) -> Option<&AudioSegment> {
        if self.open {
            self.segments.last()
        } else {
            None
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn bad(&self) -> u64 {
        self.bad
    }

    pub fn interrupted(&self) -> u64 {
        self.interrupted
    }

    pub fn last_ctl_event(&self) -> Option<u8> {
        self.last_ctl_event
    }

    /// 最近一次 `AUDIO_CTL` 的时间（毫秒）。
    pub fn last_ctl_ms(&self) -> u64 {
        self.last_ctl_ms
    }

    /// 处理 `AUDIO_CTL`；`ev=1` 返回刚封好的段。
    pub fn on_ctl(&mut self, event: u8, seg_id: u16, now_ms: u64) -> Option<AudioSegment> {
        self.last_ctl_ms = now_ms;
        self.last_ctl_event = Some(event);
        self.ptt_mode = true;

        match event {
            0 => {
                if self.open {
                    // 新的按压：上一段未封口，按规则丢弃（绝不拼接）。
                    self.interrupted += 1;
                    self.segments.pop();
                }
                self.segments
                    .push(AudioSegment::new(i32::from(seg_id), now_ms));
                self.open = true;
                self.decoder.reset();
                self.next_seq = 0;
                self.trim();
                None
            }
            1 => {
                if !self.open {
                    return None;
                }
                let segment = self.segments.last_mut().expect("open 段必然存在");
                segment.ended_at_ms = Some(now_ms);
                segment.complete = true;
                self.open = false;
                self.decoder.reset();
                self.next_seq = 0;
                Some(segment.clone())
            }
            _ => None,
        }
    }

    /// 链路断开/停止收听时丢弃未封口段。
    pub fn discard_open(&mut self, _reason: &str) -> Option<AudioSegment> {
        if !self.open {
            return None;
        }
        let mut segment = self.segments.pop().expect("open 段必然存在");
        segment.interrupted = true;
        self.interrupted += 1;
        self.open = false;
        self.decoder.reset();
        self.next_seq = 0;
        Some(segment)
    }

    /// 没有 `ev=1` 时（旧 `audio save/send`）封口当前段。
    pub fn finalize_current(&mut self, now_ms: u64) -> Option<AudioSegment> {
        if !self.open {
            return None;
        }
        let segment = self.segments.last_mut().expect("open 段必然存在");
        segment.ended_at_ms = Some(now_ms);
        segment.complete = true;
        self.open = false;
        self.decoder.reset();
        self.next_seq = 0;
        Some(segment.clone())
    }

    /// 收一个 `AUDIO_DATA` 帧，返回实时播放 PCM（重复/坏帧返回空）。
    pub fn feed(&mut self, frame: &Frame, now_ms: u64) -> Vec<i16> {
        if frame.ty != TYPE_AUDIO_DATA {
            return Vec::new();
        }
        let Some(payload) = parse_audio_payload(&frame.payload) else {
            self.bad += 1;
            return Vec::new();
        };
        if payload.fmt != AUDIO_FMT_ADPCM || payload.data.len() != BLOCK_BYTES {
            self.bad += 1;
            return Vec::new();
        }

        if !self.open {
            if self.ptt_mode {
                // PTT 模式下 ev=1 后的迟到块不应再开一段。
                return Vec::new();
            }
            self.start_legacy(now_ms);
        }

        let seq = payload.block_seq;
        if seq < self.next_seq {
            if seq != 0 {
                return Vec::new();
            }
            if self.ptt_mode {
                return Vec::new();
            }
            // 旧模式的第二遍录音：封上一段，开新段。
            self.finalize_current(now_ms);
            self.start_legacy(now_ms);
        }

        let segment = self.segments.last_mut().expect("已确保存在当前段");
        segment.blocks.insert(seq, payload.data.to_vec());
        self.next_seq = seq.wrapping_add(1);
        self.decoder.decode_block(payload.data)
    }

    /// 收一个 `AUDIO_CTL` 帧（便捷入口）。
    pub fn on_ctl_frame(&mut self, frame: &Frame, now_ms: u64) -> Option<AudioSegment> {
        if frame.ty != TYPE_AUDIO_CTL || frame.payload.len() < 3 {
            self.bad += 1;
            return None;
        }
        let event = frame.payload[0];
        let seg_id = u16::from_le_bytes([frame.payload[1], frame.payload[2]]);
        self.on_ctl(event, seg_id, now_ms)
    }

    /// 跨段汇总：`(已收块数, 应有块数, 缺失 block_seq)`。
    pub fn stats(&self) -> (u64, u64, Vec<u16>) {
        let mut received = 0u64;
        let mut total = 0u64;
        let mut missing = Vec::new();
        for segment in &self.segments {
            if segment.blocks.is_empty() {
                continue;
            }
            received += segment.received() as u64;
            let end = segment.total() as u64;
            total += end;
            missing.extend(segment.missing());
        }
        (received, total, missing)
    }

    /// 全部（或仅已封口）段的 PCM。
    pub fn pcm(&self, include_current: bool) -> Vec<i16> {
        let mut out = Vec::new();
        for (index, segment) in self.segments.iter().enumerate() {
            if !include_current && self.open && index + 1 == self.segments.len() {
                continue;
            }
            if segment.blocks.is_empty() {
                continue;
            }
            let (pcm, _missing) = decode_segment(segment);
            out.extend_from_slice(&pcm);
        }
        out
    }

    pub fn duration_s(&self) -> f64 {
        let (_received, total, _missing) = self.stats();
        total as f64 * 0.02
    }

    /// 最近一段（可要求已封口）。
    pub fn last_segment(&self, complete_only: bool) -> Option<&AudioSegment> {
        self.segments
            .iter()
            .rev()
            .find(|segment| (!complete_only || segment.complete) && !segment.blocks.is_empty())
    }

    fn start_legacy(&mut self, now_ms: u64) {
        self.segments.push(AudioSegment::new(-1, now_ms));
        self.open = true;
        self.decoder.reset();
        self.next_seq = 0;
        self.trim();
    }

    fn trim(&mut self) {
        // 与参考实现一致：列表含当前未封口段，最多保留 MAX_SEGMENTS 段。
        while self.segments.len() > MAX_SEGMENTS {
            self.segments.remove(0);
        }
    }
}
