//! `BleTransport` 的集成测试：用标准库 fake sidecar 验证 open/write/events/断连/崩溃/重启。
//!
//! 需要本机 `python`（可用 `JNWS_PYTHON` 指定，例如 kbhost 环境）。

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use kb_protocol::{build_frame, build_test_payload, FrameParser, MAX_PAYLOAD, TYPE_TEST_DATA};
use kb_transport::{
    BleCfg, BleTransport, ReconnectPolicy, SidecarLaunch, Transport, TransportCfg, TransportEvent,
    TransportKind,
};

fn python() -> String {
    std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".to_string())
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_ble_sidecar.py")
}

fn python_available() -> bool {
    Command::new(python())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn launch(env: &[(&str, &str)]) -> SidecarLaunch {
    SidecarLaunch {
        program: python(),
        script: fixture().display().to_string(),
        cwd: None,
        env: env
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
    }
}

fn ble_cfg() -> BleCfg {
    BleCfg {
        address: Some("5C:FD:52:80:0C:3C".to_string()),
        name: Some("SF32-KB-0C3C".to_string()),
        paired_only: true,
        scan_timeout_ms: 4_000,
    }
}

fn open(transport: &mut BleTransport) -> kb_transport::LinkInfo {
    transport
        .open(&TransportCfg::Ble(ble_cfg()))
        .expect("打开 fake BLE")
}

fn test_frame() -> Vec<u8> {
    build_frame(
        TYPE_TEST_DATA,
        0,
        7,
        &build_test_payload(0, b"hello ble", 0),
    )
    .expect("组帧")
}

fn wait_bytes(transport: &mut BleTransport, expected: usize, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    while bytes.len() < expected && Instant::now() < deadline {
        for event in transport.events() {
            if let TransportEvent::Bytes(chunk) = event {
                bytes.extend_from_slice(&chunk);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    bytes
}

fn wait_disconnect(
    transport: &mut BleTransport,
    timeout: Duration,
) -> Option<(kb_transport::DisconnectReason, Option<String>)> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for event in transport.events() {
            if let TransportEvent::Disconnected { reason, detail } = event {
                return Some((reason, detail));
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

fn skip() -> bool {
    if python_available() {
        false
    } else {
        eprintln!("skip: python 不可用（设置 JNWS_PYTHON 指定解释器）");
        true
    }
}

/// 收窄退避，便于在测试里秒级观察重连。
fn fast_policy(transport: &mut BleTransport, auto: bool) {
    transport.set_reconnect_policy(ReconnectPolicy {
        auto,
        base_ms: 50,
        max_ms: 200,
    });
}

/// 重连过程快照：重试序号序列 + 是否最终恢复。
#[derive(Default)]
struct RetryTrace {
    attempts: Vec<u32>,
    reconnected: Option<kb_transport::LinkInfo>,
    disconnected: bool,
}

fn pump(transport: &mut BleTransport, trace: &mut RetryTrace) {
    for event in transport.events() {
        match event {
            TransportEvent::Disconnected { .. } => trace.disconnected = true,
            TransportEvent::Reconnecting { attempt, .. } => trace.attempts.push(attempt),
            TransportEvent::Reconnected { link } => trace.reconnected = Some(link),
            _ => {}
        }
    }
}

fn pump_until_reconnected(transport: &mut BleTransport, trace: &mut RetryTrace, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while trace.reconnected.is_none() && Instant::now() < deadline {
        pump(transport, trace);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn opens_with_mtu_and_roundtrips_bytes() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[]));
    let link = open(&mut transport);
    assert_eq!(link.kind, TransportKind::Ble);
    assert_eq!(link.mtu, 247);
    assert_eq!(link.max_payload, 233);
    assert_eq!(transport.max_payload(), 233);
    assert_eq!(transport.kind(), TransportKind::Ble);

    let raw = test_frame();
    transport.write(&raw).expect("写入");
    let bytes = wait_bytes(&mut transport, raw.len(), Duration::from_secs(2));
    assert_eq!(bytes, raw);

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let frames = parser.feed(&bytes);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].seq, 7);
    assert_eq!(frames[0].data(), b"hello ble");

    let stats = transport.stats();
    assert_eq!(stats.tx_bytes, raw.len() as u64);
    assert_eq!(stats.rx_bytes, raw.len() as u64);
    assert_eq!(stats.errors, 0);
    transport.close().expect("关闭");
}

#[test]
fn rejects_frame_over_mtu_budget() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[]));
    open(&mut transport);
    let oversized =
        build_frame(TYPE_TEST_DATA, 0, 1, &build_test_payload(0, &[0u8; 300], 0)).expect("组帧");
    let error = transport.write(&oversized).expect_err("应拒绝超 MTU 帧");
    assert_eq!(error.code(), "E_PAYLOAD_TOO_LARGE");
    assert_eq!(transport.stats().tx_bytes, 0);
}

#[test]
fn disconnect_event_clears_link() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[("FAKE_BLE_DROP_AFTER_WRITE", "1")]));
    open(&mut transport);
    transport.write(&test_frame()).expect("写入");

    let (reason, detail) = wait_disconnect(&mut transport, Duration::from_secs(2)).expect("应断连");
    assert_eq!(reason, kb_transport::DisconnectReason::LinkLost);
    assert!(detail.is_some());
    assert!(transport.link().is_none());
    assert_eq!(
        transport.write(&test_frame()).unwrap_err().code(),
        "E_NOT_CONNECTED"
    );
}

#[test]
fn connect_failure_maps_to_device_not_found() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[("FAKE_BLE_FAIL", "1")]));
    let error = transport
        .open(&TransportCfg::Ble(ble_cfg()))
        .expect_err("应连接失败");
    assert_eq!(error.code(), "E_BLE_DEVICE_NOT_FOUND");
    assert!(!transport.is_connected());
}

#[test]
fn sidecar_crash_is_reported_and_restartable() {
    if skip() {
        return;
    }
    let marker = std::env::temp_dir().join(format!("jnws-ble-crash-{}.marker", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let marker_text = marker.display().to_string();
    let mut transport = BleTransport::new(launch(&[
        ("FAKE_BLE_CRASH_ON_WRITE", "1"),
        ("FAKE_BLE_MARKER", marker_text.as_str()),
    ]));
    open(&mut transport);
    let error = transport.write(&test_frame()).expect_err("sidecar 崩溃");
    assert_eq!(error.code(), "E_SIDECAR_UNAVAILABLE");
    assert!(!transport.is_connected());

    // 事件里应有一个 Error（sidecar 已退出）。
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_error = false;
    while Instant::now() < deadline && !saw_error {
        for event in transport.events() {
            if matches!(event, TransportEvent::Error(_)) {
                saw_error = true;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(saw_error);

    transport.restart_sidecar().expect("重启 sidecar");
    open(&mut transport);
    transport.write(&test_frame()).expect("重启后写入");
    let bytes = wait_bytes(&mut transport, test_frame().len(), Duration::from_secs(2));
    assert_eq!(bytes, test_frame());
}

#[test]
fn scan_returns_fake_device() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[]));
    let devices = transport.scan(2_000, Some("SF32")).expect("扫描");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].address, "5C:FD:52:80:0C:3C");
    assert!(devices[0].paired);
    assert_eq!(devices[0].address_type.as_deref(), Some("public"));
}

#[test]
fn open_is_idempotent() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[]));
    let first = open(&mut transport);
    let second = open(&mut transport);
    assert_eq!(first.address, second.address);
    assert_eq!(transport.stats().reconnects, 0);
}

#[test]
fn auto_reconnects_after_link_loss() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[("FAKE_BLE_DROP_AFTER_WRITE", "1")]));
    fast_policy(&mut transport, true);
    open(&mut transport);
    transport.write(&test_frame()).expect("写入");

    let mut trace = RetryTrace::default();
    pump_until_reconnected(&mut transport, &mut trace, Duration::from_secs(5));
    assert!(trace.disconnected, "应先上报断连");
    assert_eq!(trace.attempts, vec![1], "一次成功只需第 1 次重试");
    let link = trace.reconnected.expect("应自动重连成功");
    assert_eq!(link.mtu, 247);
    assert!(transport.is_connected());
    assert_eq!(transport.stats().reconnects, 1);
    assert_eq!(transport.reconnect_status().attempt, 0);
    assert!(transport.reconnect_status().next_retry_ms.is_none());

    // 重连后链路仍可写：Notify 在 connect 内重新订阅。
    transport.write(&test_frame()).expect("重连后写入");
}

#[test]
fn auto_reconnect_backs_off_until_success() {
    if skip() {
        return;
    }
    let counter =
        std::env::temp_dir().join(format!("jnws-ble-retry-{}.counter", std::process::id()));
    let _ = std::fs::remove_file(&counter);
    let counter_text = counter.display().to_string();
    let mut transport = BleTransport::new(launch(&[
        ("FAKE_BLE_DROP_AFTER_WRITE", "1"),
        ("FAKE_BLE_FAIL_TIMES", "2"),
        ("FAKE_BLE_FAIL_COUNTER", counter_text.as_str()),
    ]));
    fast_policy(&mut transport, true);
    open(&mut transport);
    transport.write(&test_frame()).expect("写入");

    let mut trace = RetryTrace::default();
    pump_until_reconnected(&mut transport, &mut trace, Duration::from_secs(5));
    let _ = std::fs::remove_file(&counter);
    assert_eq!(trace.attempts, vec![1, 2, 3], "失败后应继续退避重试");
    assert!(trace.reconnected.is_some(), "第三次应连上");
    assert_eq!(transport.stats().reconnects, 1);
    assert_eq!(transport.stats().errors, 2, "两次失败各计一次错误");
}

#[test]
fn auto_reconnect_can_be_disabled() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[("FAKE_BLE_DROP_AFTER_WRITE", "1")]));
    fast_policy(&mut transport, false);
    open(&mut transport);
    transport.write(&test_frame()).expect("写入");

    let mut trace = RetryTrace::default();
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        pump(&mut transport, &mut trace);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(trace.disconnected);
    assert!(trace.attempts.is_empty(), "auto=false 不应排期重试");
    assert!(trace.reconnected.is_none());
    assert!(transport.link().is_none());
    assert_eq!(
        transport.write(&test_frame()).unwrap_err().code(),
        "E_NOT_CONNECTED"
    );
}

#[test]
fn manual_retry_bypasses_backoff() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[("FAKE_BLE_DROP_AFTER_WRITE", "1")]));
    transport.set_reconnect_policy(ReconnectPolicy {
        auto: true,
        base_ms: 30_000,
        max_ms: 30_000,
    });
    open(&mut transport);
    transport.write(&test_frame()).expect("写入");

    let mut disconnected = false;
    let mut delay_ms = None;
    let deadline = Instant::now() + Duration::from_secs(2);
    while delay_ms.is_none() && Instant::now() < deadline {
        for event in transport.events() {
            match event {
                TransportEvent::Disconnected { .. } => disconnected = true,
                TransportEvent::Reconnecting { delay_ms: d, .. } => delay_ms = Some(d),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(disconnected);
    assert_eq!(delay_ms, Some(30_000), "应按 30 s 退避排期");
    assert!(transport.retry_now(), "手动重试应生效");

    let mut trace = RetryTrace::default();
    pump_until_reconnected(&mut transport, &mut trace, Duration::from_secs(5));
    assert!(trace.reconnected.is_some(), "手动重试应立即连上");
}

#[test]
fn close_suspends_auto_reconnect() {
    if skip() {
        return;
    }
    let mut transport = BleTransport::new(launch(&[]));
    open(&mut transport);
    assert!(transport.retry_now(), "已 open 时可手动重试");
    transport.close().expect("关闭");
    assert!(!transport.retry_now(), "手动关闭后不再重试");
    assert_eq!(transport.reconnect_status().attempt, 0);
}
