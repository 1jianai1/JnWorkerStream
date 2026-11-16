//! `TEST_DATA` 帧的序号 / 长度 / 模式 / 抖动 / 摘要校验。

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use kb_protocol::{Frame, TEST_FLAG_ECHO};

use crate::pattern::{classify_pattern, pattern_matches, Pattern};
use crate::sha256::Sha256;
use crate::types::{CheckStats, PayloadInfo};

mod report;

/// 校验器配置。
#[derive(Debug, Clone)]
pub struct CheckerConfig {
    pub pattern: Pattern,
    pub expected_data_len: Option<usize>,
    pub jitter_window: usize,
    pub max_gap_records: usize,
    pub ignore_echo_seq: bool,
}

impl Default for CheckerConfig {
    fn default() -> Self {
        Self {
            pattern: Pattern::Auto,
            expected_data_len: None,
            jitter_window: 4096,
            max_gap_records: 256,
            ignore_echo_seq: true,
        }
    }
}

/// 对 `TEST_DATA` 帧做连续性与内容校验。
#[derive(Debug)]
pub struct Checker {
    config: CheckerConfig,
    last_seq: Option<u16>,
    first_seq: Option<u16>,
    max_seq: Option<u16>,
    stats: CheckStats,
    payload_bytes: u64,
    digest: Sha256,
    digest_final: Option<String>,
    lengths: BTreeMap<usize, u64>,
    patterns_seen: BTreeMap<&'static str, u64>,
    lost_ranges: Vec<(u16, u32)>,
    inferred_pattern: Option<&'static str>,
    inferred_len: Option<usize>,
    intervals_ms: VecDeque<f64>,
    last_arrival: Option<f64>,
    started_at: Instant,
    finished_at: Option<Instant>,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    pub fn new() -> Self {
        Self::with_config(CheckerConfig::default())
    }

    pub fn with_config(config: CheckerConfig) -> Self {
        assert!(config.jitter_window > 0, "jitter_window 必须为正数");
        Self {
            config,
            last_seq: None,
            first_seq: None,
            max_seq: None,
            stats: CheckStats::default(),
            payload_bytes: 0,
            digest: Sha256::new(),
            digest_final: None,
            lengths: BTreeMap::new(),
            patterns_seen: BTreeMap::new(),
            lost_ranges: Vec::new(),
            inferred_pattern: None,
            inferred_len: None,
            intervals_ms: VecDeque::new(),
            last_arrival: None,
            started_at: Instant::now(),
            finished_at: None,
        }
    }

    /// 校验一帧；`arrival_ms` 由调用方给出（与 CSV 的 `arrival_ms` 同源）。
    pub fn on_frame(&mut self, frame: &Frame, arrival_ms: f64) -> PayloadInfo {
        self.on_payload(frame.seq, &frame.payload, arrival_ms)
    }

    /// 校验一个 `TEST_DATA` 载荷（`dev_ms + flags + data`）。
    pub fn on_payload(&mut self, seq: u16, payload: &[u8], arrival_ms: f64) -> PayloadInfo {
        if let Some(prev) = self.last_arrival {
            self.intervals_ms.push_back((arrival_ms - prev) * 1000.0);
            while self.intervals_ms.len() > self.config.jitter_window {
                self.intervals_ms.pop_front();
            }
        }
        self.last_arrival = Some(arrival_ms);
        self.stats.recv += 1;

        if payload.len() < 5 {
            self.stats.malformed += 1;
            return PayloadInfo {
                seq,
                dev_ms: 0,
                flags: 0,
                data: payload.to_vec(),
                payload_len: payload.len(),
                data_len: payload.len(),
                sha256: Sha256::digest_hex(payload),
                is_echo: false,
                pattern: "other",
                pattern_ok: false,
                length_ok: false,
            };
        }

        let dev_ms = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let flags = payload[4];
        let data = &payload[5..];
        let is_echo = flags & TEST_FLAG_ECHO != 0;
        let data_len = data.len();

        if is_echo {
            self.stats.echo_frames += 1;
        }
        if !(is_echo && self.config.ignore_echo_seq) {
            self.track_seq(seq);
        }

        self.payload_bytes += data_len as u64;
        self.digest.update(data);
        *self.lengths.entry(data_len).or_insert(0) += 1;

        let length_ok = match self.config.expected_data_len {
            Some(expected) => {
                let ok = data_len == expected;
                if !ok {
                    self.stats.length_err += 1;
                }
                ok
            }
            None => match self.inferred_len {
                None => {
                    self.inferred_len = Some(data_len);
                    true
                }
                Some(expected) => {
                    let ok = data_len == expected;
                    if !ok {
                        self.stats.length_changes += 1;
                    }
                    ok
                }
            },
        };

        // 回显帧的数据应逐字节等于主机发送内容，模式检查对它没有意义。
        let check_pattern = !is_echo;
        let pattern = classify_pattern(data, seq);
        *self.patterns_seen.entry(pattern).or_insert(0) += 1;

        let mut pattern_ok = true;
        if check_pattern && self.config.pattern != Pattern::Any {
            let expected = if self.config.pattern == Pattern::Auto {
                if self.inferred_pattern.is_none() && matches!(pattern, "inc" | "zero" | "ascii") {
                    self.inferred_pattern = Some(pattern);
                }
                self.inferred_pattern
            } else {
                Some(self.config.pattern.as_str())
            };
            if let Some(expected) = expected {
                pattern_ok = pattern_matches(expected, data, seq);
                if !pattern_ok {
                    self.stats.pattern_err += 1;
                }
            }
        }

        PayloadInfo {
            seq,
            dev_ms,
            flags,
            data: data.to_vec(),
            payload_len: payload.len(),
            data_len,
            sha256: Sha256::digest_hex(data),
            is_echo,
            pattern,
            pattern_ok,
            length_ok,
        }
    }

    /// 封存摘要；重复调用无副作用。
    pub fn finish(&mut self) {
        if self.finished_at.is_none() {
            self.finished_at = Some(Instant::now());
            self.digest_final = Some(crate::sha256::to_hex(&self.digest.clone().finish()));
        }
    }

    pub fn digest_hex(&self) -> Option<&str> {
        self.digest_final.as_deref()
    }

    pub fn stats(&self) -> CheckStats {
        self.stats
    }

    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    pub fn first_seq(&self) -> Option<u16> {
        self.first_seq
    }

    pub fn last_seq(&self) -> Option<u16> {
        self.last_seq
    }

    pub fn max_seq(&self) -> Option<u16> {
        self.max_seq
    }

    pub fn duration_s(&self) -> f64 {
        let end = self.finished_at.unwrap_or_else(Instant::now);
        end.duration_since(self.started_at).as_secs_f64().max(0.0)
    }

    pub fn rate_fps(&self) -> f64 {
        let seconds = self.duration_s();
        if seconds > 0.0 {
            self.stats.recv as f64 / seconds
        } else {
            0.0
        }
    }

    pub fn throughput_bps(&self) -> f64 {
        let seconds = self.duration_s();
        if seconds > 0.0 {
            self.payload_bytes as f64 / seconds
        } else {
            0.0
        }
    }

    /// 到达间隔总体标准差（窗口 4096，样本 < 2 返回 `None`）。
    pub fn jitter_ms(&self) -> Option<f64> {
        if self.intervals_ms.len() < 2 {
            return None;
        }
        let n = self.intervals_ms.len() as f64;
        let mean = self.intervals_ms.iter().sum::<f64>() / n;
        let variance = self
            .intervals_ms
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / n;
        Some(variance.sqrt())
    }

    pub fn mean_interval_ms(&self) -> Option<f64> {
        if self.intervals_ms.is_empty() {
            return None;
        }
        Some(self.intervals_ms.iter().sum::<f64>() / self.intervals_ms.len() as f64)
    }

    pub fn expected_pattern(&self) -> Option<&'static str> {
        self.inferred_pattern.or({
            if matches!(self.config.pattern, Pattern::Auto | Pattern::Any) {
                None
            } else {
                Some(self.config.pattern.as_str())
            }
        })
    }

    pub fn lost_ranges(&self) -> &[(u16, u32)] {
        &self.lost_ranges
    }

    pub fn length_histogram(&self) -> &BTreeMap<usize, u64> {
        &self.lengths
    }

    pub fn patterns_seen(&self) -> &BTreeMap<&'static str, u64> {
        &self.patterns_seen
    }

    fn record_gap(&mut self, start: u16, count: u32) {
        if count == 0 {
            return;
        }
        if self.lost_ranges.len() < self.config.max_gap_records {
            self.lost_ranges.push((start, count));
        }
    }

    fn track_seq(&mut self, seq: u16) {
        let Some(last) = self.last_seq else {
            self.first_seq = Some(seq);
            self.last_seq = Some(seq);
            self.max_seq = Some(seq);
            return;
        };

        let delta = seq.wrapping_sub(last);
        if delta == 0 {
            self.stats.dup += 1;
            return;
        }
        if delta < 0x8000 {
            if delta > 1 {
                self.stats.lost += u64::from(delta - 1);
                self.record_gap(last.wrapping_add(1), u32::from(delta - 1));
            }
            self.last_seq = Some(seq);
            if self
                .max_seq
                .is_none_or(|max| seq.wrapping_sub(max) < 0x8000)
            {
                self.max_seq = Some(seq);
            }
        } else {
            // 回退 / 严重乱序：只计数，不推进水位，避免后续被误判为大量丢帧。
            self.stats.order += 1;
        }
    }
}
