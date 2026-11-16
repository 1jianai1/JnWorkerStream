//! `SidecarClient` 的集成测试：用标准库 fake sidecar 验证握手/关联/事件/超时/崩溃/版本。
//!
//! 需要本机有可用的 `python`（可用 `JNWS_PYTHON` 指定，例如 kbhost 环境）。

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use kb_sidecar::{SidecarClient, SidecarConfig, SidecarError, SidecarStatus};
use serde_json::{json, Map, Value};

fn python() -> String {
    std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".to_string())
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_sidecar.py")
}

fn config() -> SidecarConfig {
    let mut config = SidecarConfig::ble(python(), fixture().display().to_string());
    config.hello_timeout = Duration::from_secs(5);
    config.request_timeout = Duration::from_secs(3);
    config
}

fn python_available() -> bool {
    Command::new(python())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn start(config: SidecarConfig) -> Option<SidecarClient> {
    if !python_available() {
        eprintln!("skip: python 不可用（设置 JNWS_PYTHON 指定解释器）");
        return None;
    }
    Some(SidecarClient::start(config).expect("启动 fake sidecar"))
}

fn params(pairs: &[(&str, Value)]) -> Map<String, Value> {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.insert((*key).to_string(), value.clone());
    }
    map
}

#[test]
fn hello_and_ping() {
    let Some(client) = start(config()) else {
        return;
    };
    assert!(client.is_alive());
    assert_eq!(client.status(), SidecarStatus::Ready);

    let pong = client.call("ping", Map::new()).expect("ping");
    assert_eq!(pong.header.get("pong"), Some(&json!(true)));
}

#[test]
fn response_body_is_delivered() {
    let Some(client) = start(config()) else {
        return;
    };
    let frame = client
        .call("echo", params(&[("text", json!("你好, sidecar"))]))
        .expect("echo");
    assert_eq!(frame.body, "你好, sidecar".as_bytes());
}

#[test]
fn events_are_queued() {
    let Some(client) = start(config()) else {
        return;
    };
    client.call("emit", Map::new()).expect("emit");
    let event = client
        .next_event(Duration::from_secs(2))
        .expect("应收到事件");
    assert_eq!(event.event(), Some("tick"));
    assert_eq!(event.body(), b"hello");
    assert!(client.try_events().is_empty());
}

#[test]
fn timeout_is_reported_and_process_survives() {
    let Some(client) = start(config()) else {
        return;
    };
    let error = client
        .request_timeout(
            "sleep_ms",
            params(&[("ms", json!(1500))]),
            Duration::from_millis(200),
        )
        .expect_err("应超时");
    assert!(matches!(error, SidecarError::Timeout { .. }));
    assert_eq!(error.code(), "E_IO");

    // 迟到的响应不应影响后续请求。
    let pong = client.call("ping", Map::new()).expect("超时后仍可用");
    assert_eq!(pong.header.get("pong"), Some(&json!(true)));
}

#[test]
fn crash_is_reported_to_pending_requests() {
    let Some(client) = start(config()) else {
        return;
    };
    let error = client
        .request("crash", Map::new())
        .expect_err("进程退出应报错");
    assert!(matches!(error, SidecarError::Exited { .. }));

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while client.is_alive() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!client.is_alive());
    assert!(matches!(client.status(), SidecarStatus::Exited { .. }));
    assert_eq!(
        client.request("ping", Map::new()).unwrap_err().code(),
        "E_SIDECAR_UNAVAILABLE"
    );
}

#[test]
fn version_mismatch_is_rejected() {
    if !python_available() {
        eprintln!("skip: python 不可用");
        return;
    }
    let error = SidecarClient::start(config().with_env("FAKE_PROTO", "2"))
        .expect_err("版本不匹配应拒绝启动");
    assert!(matches!(
        error,
        SidecarError::VersionMismatch { got: 2, want: 1 }
    ));
    assert_eq!(error.code(), "E_SIDECAR_PROTOCOL");
}

#[test]
fn role_mismatch_is_rejected() {
    if !python_available() {
        eprintln!("skip: python 不可用");
        return;
    }
    let error = SidecarClient::start(config().with_env("FAKE_ROLE", "asr"))
        .expect_err("角色不匹配应拒绝启动");
    assert!(matches!(error, SidecarError::RoleMismatch { .. }));
}

#[test]
fn stderr_is_captured() {
    let Some(client) = start(config()) else {
        return;
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while client.stderr_tail().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(client
        .stderr_tail()
        .iter()
        .any(|line| line.contains("[fake] started")));
}

#[test]
fn stop_is_clean_and_idempotent() {
    let Some(mut client) = start(config()) else {
        return;
    };
    client.stop();
    assert!(!client.is_alive());
    assert_eq!(client.exit_code(), Some(0));
    client.stop();
    assert!(!client.is_alive());
}

#[test]
fn restart_after_stop() {
    let Some(mut client) = start(config()) else {
        return;
    };
    client.restart().expect("重启");
    assert!(client.is_alive());
    client.call("ping", Map::new()).expect("重启后 ping");
}
