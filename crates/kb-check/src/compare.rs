//! 跨链路 CSV 比较（帧数、seq 序列/集合、sha256 序列、payload 长度分布）。

use std::collections::BTreeMap;

use crate::error::CheckError;
use crate::recorder::{load_records, Record};

/// 一处不一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub kind: &'static str,
    pub index: usize,
    pub a: String,
    pub b: String,
}

/// 比较报告（`docs/interfaces.md` §3.5 `CompareReport`）。
#[derive(Debug, Clone)]
pub struct CompareReport {
    pub ok: bool,
    pub frames_a: usize,
    pub frames_b: usize,
    pub mismatches: Vec<Mismatch>,
    pub first_divergence_index: Option<usize>,
    /// 人类可读摘要行（写入日志/控制台）。
    pub lines: Vec<String>,
}

/// 读取两份 CSV 并比较。
pub fn compare_files(path_a: &str, path_b: &str) -> Result<CompareReport, CheckError> {
    let a = load_records(path_a)?;
    let b = load_records(path_b)?;
    Ok(compare_records(&a, &b, path_a, path_b))
}

/// 比较两条已加载的记录序列。
pub fn compare_records(a: &[Record], b: &[Record], label_a: &str, label_b: &str) -> CompareReport {
    let mut lines = vec![
        format!("{label_a}: frames={}", a.len()),
        format!("{label_b}: frames={}", b.len()),
    ];
    let mut mismatches = Vec::new();
    let mut first_divergence = None;

    if a.is_empty() || b.is_empty() {
        mismatches.push(Mismatch {
            kind: "empty",
            index: 0,
            a: format!("{}", a.len()),
            b: format!("{}", b.len()),
        });
        lines.push("RESULT FAIL (empty record)".to_string());
        return CompareReport {
            ok: false,
            frames_a: a.len(),
            frames_b: b.len(),
            mismatches,
            first_divergence_index: None,
            lines,
        };
    }

    if a.len() != b.len() {
        mismatches.push(Mismatch {
            kind: "frames",
            index: a.len().min(b.len()),
            a: a.len().to_string(),
            b: b.len().to_string(),
        });
        first_divergence = Some(a.len().min(b.len()));
        lines.push(format!("frames  DIFF ({} vs {})", a.len(), b.len()));
    } else {
        lines.push(format!("frames  OK ({})", a.len()));
    }

    let seq_a: Vec<u16> = a.iter().map(|record| record.seq).collect();
    let seq_b: Vec<u16> = b.iter().map(|record| record.seq).collect();
    match first_diff(&seq_a, &seq_b) {
        None => lines.push("seq     OK".to_string()),
        Some(index) => {
            let left = seq_a
                .get(index)
                .map_or_else(|| "<missing>".to_string(), u16::to_string);
            let right = seq_b
                .get(index)
                .map_or_else(|| "<missing>".to_string(), u16::to_string);
            mismatches.push(Mismatch {
                kind: "seq",
                index,
                a: left.clone(),
                b: right.clone(),
            });
            first_divergence = Some(first_divergence.map_or(index, |current| current.min(index)));
            lines.push(format!("seq     DIFF at index {index} ({left} vs {right})"));
        }
    }

    let set_a: std::collections::BTreeSet<u16> = seq_a.iter().copied().collect();
    let set_b: std::collections::BTreeSet<u16> = seq_b.iter().copied().collect();
    if set_a != set_b {
        let only_a: Vec<String> = set_a
            .difference(&set_b)
            .take(10)
            .map(u16::to_string)
            .collect();
        let only_b: Vec<String> = set_b
            .difference(&set_a)
            .take(10)
            .map(u16::to_string)
            .collect();
        mismatches.push(Mismatch {
            kind: "seqset",
            index: 0,
            a: only_a.join(","),
            b: only_b.join(","),
        });
        lines.push(format!(
            "seqset  DIFF only {label_a}=[{left}] only {label_b}=[{right}]",
            left = only_a.join(","),
            right = only_b.join(",")
        ));
    }

    let hash_a: Vec<&str> = a.iter().map(|record| record.sha256.as_str()).collect();
    let hash_b: Vec<&str> = b.iter().map(|record| record.sha256.as_str()).collect();
    match first_diff(&hash_a, &hash_b) {
        None => lines.push("sha256  OK".to_string()),
        Some(index) => {
            let left = hash_a.get(index).copied().unwrap_or("<missing>");
            let right = hash_b.get(index).copied().unwrap_or("<missing>");
            mismatches.push(Mismatch {
                kind: "sha256",
                index,
                a: left.to_string(),
                b: right.to_string(),
            });
            first_divergence = Some(first_divergence.map_or(index, |current| current.min(index)));
            lines.push(format!("sha256  DIFF at index {index}"));
        }
    }

    let lengths_a = length_counts(a);
    let lengths_b = length_counts(b);
    if lengths_a == lengths_b {
        let desc = lengths_a
            .iter()
            .take(8)
            .map(|(len, count)| format!("{len}B:{count}"))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("lengths OK ({desc})"));
    } else {
        mismatches.push(Mismatch {
            kind: "lengths",
            index: 0,
            a: describe_lengths(&lengths_a),
            b: describe_lengths(&lengths_b),
        });
        lines.push(format!(
            "lengths DIFF {} vs {}",
            describe_lengths(&lengths_a),
            describe_lengths(&lengths_b)
        ));
    }

    let ok = mismatches.is_empty();
    lines.push(if ok {
        "RESULT PASS".to_string()
    } else {
        "RESULT FAIL".to_string()
    });
    CompareReport {
        ok,
        frames_a: a.len(),
        frames_b: b.len(),
        mismatches,
        first_divergence_index: first_divergence,
        lines,
    }
}

fn length_counts(records: &[Record]) -> BTreeMap<usize, u64> {
    let mut counts = BTreeMap::new();
    for record in records {
        *counts.entry(record.payload_len).or_insert(0) += 1;
    }
    counts
}

fn describe_lengths(counts: &BTreeMap<usize, u64>) -> String {
    counts
        .iter()
        .map(|(len, count)| format!("{len}B:{count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 返回首个不同下标；长度不同但公共前缀相同时返回较短长度。
fn first_diff<T: PartialEq>(a: &[T], b: &[T]) -> Option<usize> {
    for (index, (left, right)) in a.iter().zip(b.iter()).enumerate() {
        if left != right {
            return Some(index);
        }
    }
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    None
}
