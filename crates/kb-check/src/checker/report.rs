//! `Checker` 的文本报告：判定与多行摘要（与参考实现输出一致）。

use super::Checker;

impl Checker {
    /// `pass = recv>0 && lost==dup==order==pattern_err==length_err==malformed==0`。
    pub fn pass_fail(&self) -> (bool, Vec<String>) {
        let mut reasons = Vec::new();
        if self.stats.recv == 0 {
            reasons.push("no frames".to_string());
        }
        for (name, value) in [
            ("lost", self.stats.lost),
            ("dup", self.stats.dup),
            ("order", self.stats.order),
            ("pattern_err", self.stats.pattern_err),
            ("length_err", self.stats.length_err),
            ("malformed", self.stats.malformed),
        ] {
            if value != 0 {
                reasons.push(format!("{name}={value}"));
            }
        }
        (reasons.is_empty(), reasons)
    }

    /// 与参考实现一致的多行摘要（日志用）。
    pub fn summary_lines(&mut self) -> Vec<String> {
        self.finish();
        let stats = self.stats;
        let mut lines = vec![format!(
            "SUMMARY frames={} payload_bytes={} lost={} dup={} order={} pattern_err={} \
             length_err={} malformed={} echo={}",
            stats.recv,
            self.payload_bytes,
            stats.lost,
            stats.dup,
            stats.order,
            stats.pattern_err,
            stats.length_err,
            stats.malformed,
            stats.echo_frames
        )];
        lines.push(format!(
            "SUMMARY seq first={:?} last={:?} max={:?} duration={:.3}s rate={:.2}fps \
             payload={:.2}KB/s",
            self.first_seq,
            self.last_seq,
            self.max_seq,
            self.duration_s(),
            self.rate_fps(),
            self.throughput_bps() / 1024.0
        ));
        if !self.intervals_ms.is_empty() {
            let jitter = self
                .jitter_ms()
                .map(|value| format!("{value:.3}ms"))
                .unwrap_or_else(|| "n/a".to_string());
            lines.push(format!(
                "SUMMARY interval mean={:.3}ms jitter={} samples={}",
                self.mean_interval_ms().unwrap_or(0.0),
                jitter,
                self.intervals_ms.len()
            ));
        }
        if !self.lengths.is_empty() {
            let hist = self
                .lengths
                .iter()
                .take(8)
                .map(|(len, count)| format!("{len}B:{count}"))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(format!("SUMMARY sizes {hist}"));
        }
        if !self.patterns_seen.is_empty() {
            let seen = self
                .patterns_seen
                .iter()
                .map(|(name, count)| format!("{name}:{count}"))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(format!(
                "SUMMARY patterns {seen} expected={}",
                self.expected_pattern().unwrap_or("none")
            ));
        }
        if !self.lost_ranges.is_empty() {
            let ranges = self
                .lost_ranges
                .iter()
                .take(12)
                .map(|(start, count)| format!("{start}(+{count})"))
                .collect::<Vec<_>>()
                .join(", ");
            let more = if self.lost_ranges.len() > 12 {
                format!(" ... (+{})", self.lost_ranges.len() - 12)
            } else {
                String::new()
            };
            lines.push(format!("SUMMARY lost_at {ranges}{more}"));
        }
        lines.push(format!(
            "SUMMARY digest sha256={}",
            self.digest_final.as_deref().unwrap_or("")
        ));
        lines
    }
}
