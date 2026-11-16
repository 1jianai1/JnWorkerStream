//! `Checker` / CSV / 比较的等价断言（对应 `py_example/selftest.py` 的检查部分）。

use std::path::PathBuf;

use kb_check::{
    compare_files, compare_records, load_records, Checker, CheckerConfig, Pattern, Recorder,
};
use kb_protocol::{
    build_frame, build_test_payload, Frame, FrameParser, MAX_PAYLOAD, TEST_FLAG_ECHO,
    TYPE_TEST_DATA,
};

fn inc_data(seq: u16, size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| ((usize::from(seq) * 7 + i) & 0xFF) as u8)
        .collect()
}

fn raw_frame(seq: u16, size: usize) -> Vec<u8> {
    build_frame(
        TYPE_TEST_DATA,
        0,
        seq,
        &build_test_payload(0, &inc_data(seq, size), 0),
    )
    .unwrap()
}

fn parse_all(count: usize, size: usize) -> Vec<Frame> {
    let mut raw = Vec::new();
    for seq in 0..count as u16 {
        raw.extend_from_slice(&raw_frame(seq, size));
    }
    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let mut frames = Vec::new();
    for chunk in raw.chunks(137) {
        frames.extend(parser.feed(chunk));
    }
    frames
}

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jnws-kb-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir.join(name)
}

fn record(path: &PathBuf, frames: &[Frame], checker: &mut Checker) {
    let mut recorder = Recorder::new(path).expect("创建 CSV");
    for (index, frame) in frames.iter().enumerate() {
        let info = checker.on_frame(frame, index as f64 * 10.0);
        recorder.write(&info, index as f64 * 10.0).expect("写 CSV");
    }
    recorder.flush().expect("flush");
}

#[test]
fn thousand_frames_digest_matches_reference() {
    let frames = parse_all(1000, 64);
    assert_eq!(frames.len(), 1000);

    let mut checker = Checker::new();
    for (index, frame) in frames.iter().enumerate() {
        checker.on_frame(frame, index as f64 * 10.0);
    }
    checker.finish();

    let stats = checker.stats();
    assert_eq!(stats.lost, 0);
    assert_eq!(stats.dup, 0);
    assert_eq!(stats.order, 0);
    assert_eq!(stats.pattern_err, 0);
    assert_eq!(stats.length_err, 0);
    assert_eq!(stats.malformed, 0);
    assert_eq!(checker.payload_bytes(), 64_000);
    // 由 Python hashlib 计算：sha256(concat(inc_data(seq,64) for seq in 0..1000))
    assert_eq!(
        checker.digest_hex(),
        Some("a633bd92d21ad4cc47403f91eacc2fa6a56d85fdd5f54aee24f690159a127d03")
    );
    assert!(checker.pass_fail().0, "{:?}", checker.pass_fail().1);
    assert_eq!(checker.expected_pattern(), Some("inc"));
}

#[test]
fn detects_lost() {
    let frames = parse_all(20, 64);
    let mut checker = Checker::new();
    for (index, frame) in frames.iter().enumerate() {
        if index == 5 {
            continue;
        }
        checker.on_frame(frame, index as f64 * 10.0);
    }
    assert_eq!(checker.stats().lost, 1);
    assert_eq!(checker.lost_ranges(), [(5, 1)]);
    assert!(!checker.pass_fail().0);
}

#[test]
fn detects_dup() {
    let frames = parse_all(20, 64);
    let mut checker = Checker::new();
    for (index, frame) in frames.iter().enumerate() {
        checker.on_frame(frame, index as f64 * 10.0);
        if index == 4 {
            checker.on_frame(frame, index as f64 * 10.0 + 1.0);
        }
    }
    assert_eq!(checker.stats().dup, 1);
    assert!(!checker.pass_fail().0);
}

#[test]
fn detects_order() {
    let frames = parse_all(10, 64);
    let mut checker = Checker::new();
    let order = [0usize, 1, 2, 4, 3, 5, 6, 7, 8, 9];
    for index in order {
        checker.on_frame(&frames[index], index as f64 * 10.0);
    }
    assert_eq!(checker.stats().order, 1);
    // 先出现前向缺口（3 被跳过）才可能回退，故 lost 同步为 1，与参考实现一致。
    assert_eq!(checker.stats().lost, 1);
}

#[test]
fn pattern_mismatch_counted() {
    let frames: Vec<Frame> = (0..3u16)
        .map(|seq| Frame {
            ver: 1,
            ty: TYPE_TEST_DATA,
            flags: 0,
            seq,
            payload: build_test_payload(0, &[0u8; 8], 0),
        })
        .collect();
    let config = CheckerConfig {
        pattern: Pattern::Inc,
        ..CheckerConfig::default()
    };
    let mut checker = Checker::with_config(config);
    for (index, frame) in frames.iter().enumerate() {
        checker.on_frame(frame, index as f64);
    }
    assert_eq!(checker.stats().pattern_err, 3);
}

#[test]
fn echo_ignored_for_seq() {
    let mut frames = Vec::new();
    let mut parser = FrameParser::new(MAX_PAYLOAD);
    frames.extend(parser.feed(&raw_frame(1, 64)));
    let echo = build_frame(
        TYPE_TEST_DATA,
        0,
        0x7777,
        &build_test_payload(0, b"hello", TEST_FLAG_ECHO),
    )
    .unwrap();
    frames.extend(parser.feed(&echo));
    frames.extend(parser.feed(&raw_frame(2, 64)));

    let mut checker = Checker::new();
    for (index, frame) in frames.iter().enumerate() {
        checker.on_frame(frame, index as f64 * 10.0);
    }
    assert_eq!(checker.stats().echo_frames, 1);
    assert_eq!(checker.stats().lost, 0);
    assert_eq!(checker.stats().dup, 0);
}

#[test]
fn malformed_payload_counted() {
    let mut checker = Checker::new();
    let info = checker.on_payload(0, b"abc", 0.0);
    assert_eq!(checker.stats().malformed, 1);
    assert!(!info.pattern_ok);
    assert!(!info.length_ok);
    assert_eq!(info.pattern, "other");
    assert!(!checker.pass_fail().0);
}

#[test]
fn length_changes_counted_without_expected_len() {
    let mut checker = Checker::new();
    checker.on_payload(0, &build_test_payload(0, &[1u8; 8], 0), 0.0);
    checker.on_payload(1, &build_test_payload(0, &[1u8; 9], 0), 10.0);
    assert_eq!(checker.stats().length_changes, 1);
    assert_eq!(checker.stats().length_err, 0);
}

#[test]
fn csv_roundtrip_and_compare() {
    let a = temp_path("roundtrip-a.csv");
    let b = temp_path("roundtrip-b.csv");
    let frames = parse_all(50, 64);
    record(&a, &frames, &mut Checker::new());
    record(&b, &frames, &mut Checker::new());

    let text = std::fs::read_to_string(&a).expect("读取 CSV");
    let header = text.lines().next().unwrap();
    assert_eq!(
        header,
        "seq,sha256,arrival_ms,payload_len,data_len,dev_ms,flags"
    );
    let second = text.lines().nth(1).unwrap();
    let fields: Vec<&str> = second.split(',').collect();
    assert_eq!(fields.len(), 7);
    assert_eq!(fields[0], "0");
    assert_eq!(fields[1].len(), 64);
    assert_eq!(fields[2], "0.000");
    assert_eq!(fields[3], "69");
    assert_eq!(fields[4], "64");

    let rows = load_records(&a).expect("读取记录");
    assert_eq!(rows.len(), 50);
    assert_eq!(rows[0].seq, 0);
    assert_eq!(rows[49].seq, 49);

    let report = compare_files(a.to_str().unwrap(), b.to_str().unwrap()).expect("比较");
    assert!(report.ok, "{:?}", report.mismatches);
    assert!(report.lines.iter().any(|line| line == "RESULT PASS"));
}

#[test]
fn compare_detects_difference() {
    let a = temp_path("diff-a.csv");
    let b = temp_path("diff-b.csv");
    let frames = parse_all(10, 64);
    record(&a, &frames, &mut Checker::new());
    record(&b, &frames, &mut Checker::new());

    let text = std::fs::read_to_string(&b).expect("读取 CSV");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut parts: Vec<String> = lines[2].split(',').map(str::to_string).collect();
    parts[1] = "0".repeat(64);
    lines[2] = parts.join(",");
    std::fs::write(&b, lines.join("\n") + "\n").expect("改写 CSV");

    let report = compare_files(a.to_str().unwrap(), b.to_str().unwrap()).expect("比较");
    assert!(!report.ok);
    assert_eq!(report.first_divergence_index, Some(1));
    assert!(report
        .mismatches
        .iter()
        .any(|item| item.kind == "sha256" && item.index == 1));
}

#[test]
fn compare_empty_is_failure() {
    let a = temp_path("empty-a.csv");
    let b = temp_path("empty-b.csv");
    Recorder::new(&a).expect("创建").flush().expect("flush");
    Recorder::new(&b).expect("创建").flush().expect("flush");
    let report = compare_files(a.to_str().unwrap(), b.to_str().unwrap()).expect("比较");
    assert!(!report.ok);
    assert_eq!(report.mismatches[0].kind, "empty");
}

#[test]
fn compare_detects_frame_count() {
    let a = parse_all(5, 64);
    let b = parse_all(6, 64);
    let mut checker = Checker::new();
    let infos_a: Vec<_> = a
        .iter()
        .enumerate()
        .map(|(i, frame)| checker.on_frame(frame, i as f64))
        .collect();
    let records_a: Vec<kb_check::Record> = infos_a
        .iter()
        .enumerate()
        .map(|(i, info)| kb_check::Record {
            seq: info.seq,
            sha256: info.sha256.clone(),
            arrival_ms: i as f64,
            payload_len: info.payload_len,
            data_len: info.data_len,
            dev_ms: info.dev_ms,
            flags: info.flags,
        })
        .collect();
    let mut checker_b = Checker::new();
    let records_b: Vec<kb_check::Record> = b
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            let info = checker_b.on_frame(frame, i as f64);
            kb_check::Record {
                seq: info.seq,
                sha256: info.sha256.clone(),
                arrival_ms: i as f64,
                payload_len: info.payload_len,
                data_len: info.data_len,
                dev_ms: info.dev_ms,
                flags: info.flags,
            }
        })
        .collect();

    let report = compare_records(&records_a, &records_b, "a", "b");
    assert!(!report.ok);
    assert!(report.mismatches.iter().any(|item| item.kind == "frames"));
}
