//! USB CDC 传输的可重复测试（不需要真实硬件）。
//!
//! 用 `SerialPort` 替身模拟回显、分片、脏字节、CRC 错、拔出与重连；
//! 真实 COM 端口只在 `tests/transport_smoke.rs` 里做枚举/打开失败验证。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kb_protocol::{
    build_frame, build_test_payload, FrameParser, MAX_PAYLOAD, TEST_FLAG_ECHO, TYPE_TEST_DATA,
};

use super::*;
use crate::BleCfg;

#[derive(Debug)]
struct MockPort {
    rx: Mutex<VecDeque<u8>>,
    tx: Mutex<Vec<u8>>,
    read_error: Mutex<Option<PortError>>,
    echo: AtomicBool,
    closed: AtomicBool,
    chunk: usize,
}

impl MockPort {
    fn new() -> Self {
        Self {
            rx: Mutex::new(VecDeque::new()),
            tx: Mutex::new(Vec::new()),
            read_error: Mutex::new(None),
            echo: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            chunk: 64,
        }
    }

    fn with_echo(self) -> Self {
        self.echo.store(true, Ordering::Release);
        self
    }

    fn scripted(data: &[u8], chunk: usize) -> Self {
        let mut port = Self::new();
        port.chunk = chunk;
        port.rx = Mutex::new(data.iter().copied().collect());
        port
    }

    fn with_read_error(error: PortError) -> Self {
        let mut port = Self::new();
        port.read_error = Mutex::new(Some(error));
        port
    }
}

impl SerialPort for MockPort {
    fn read(&self, buf: &mut [u8]) -> Result<usize, PortError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PortError::Cancelled);
        }
        if let Some(error) = self
            .read_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            return Err(error);
        }
        let mut rx = self
            .rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if rx.is_empty() {
            drop(rx);
            std::thread::sleep(Duration::from_millis(2));
            return Err(PortError::Timeout);
        }
        let count = buf.len().min(self.chunk).min(rx.len());
        for slot in buf.iter_mut().take(count) {
            *slot = rx.pop_front().expect("count <= rx.len()");
        }
        Ok(count)
    }

    fn write_all(&self, data: &[u8]) -> Result<(), PortError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PortError::Cancelled);
        }
        self.tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(data);
        if self.echo.load(Ordering::Acquire) {
            self.rx
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(data.iter().copied());
        }
        Ok(())
    }

    fn cancel(&self) {}

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

fn test_frame(seq: u16) -> Vec<u8> {
    let data: Vec<u8> = (0..32)
        .map(|index| ((usize::from(seq) * 7 + index) & 0xFF) as u8)
        .collect();
    build_frame(TYPE_TEST_DATA, 0, seq, &build_test_payload(0, &data, 0)).expect("组帧")
}

/// 等待至少 `expected` 字节到达（或超时），返回累计字节与事件。
fn wait_bytes(
    transport: &mut UsbTransport,
    expected: usize,
    timeout: Duration,
) -> (Vec<u8>, Vec<TransportEvent>) {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut events = Vec::new();
    while Instant::now() < deadline && bytes.len() < expected {
        for event in transport.events() {
            if let TransportEvent::Bytes(chunk) = &event {
                bytes.extend_from_slice(chunk);
            }
            events.push(event);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    (bytes, events)
}

fn wait_disconnect(
    transport: &mut UsbTransport,
    timeout: Duration,
) -> Option<(DisconnectReason, Option<String>)> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for event in transport.events() {
            if let TransportEvent::Disconnected { reason, detail } = event {
                return Some((reason, detail));
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

#[test]
fn mtu_and_payload_budget_match_adr_011() {
    let transport = UsbTransport::new();
    assert_eq!(transport.kind(), TransportKind::Usb);
    assert_eq!(transport.max_payload(), 1013);
    assert_eq!(MTU_USB, 1024);
}

#[test]
fn echo_roundtrip_via_transport() {
    let mut transport = UsbTransport::new();
    transport
        .open_with_port(Box::new(MockPort::new().with_echo()), "MOCK")
        .expect("打开替身端口");
    // 设备回显会重填 dev_ms 并置 ECHO 标志；此处用已带 ECHO 的帧模拟回环。
    let raw = build_frame(
        TYPE_TEST_DATA,
        0,
        7,
        &build_test_payload(0, b"hello echo", TEST_FLAG_ECHO),
    )
    .expect("组帧");
    transport.write_frame(&raw).expect("写入");

    let (bytes, events) = wait_bytes(&mut transport, raw.len(), Duration::from_secs(2));
    assert_eq!(bytes, raw);
    assert!(events
        .iter()
        .all(|event| matches!(event, TransportEvent::Bytes(_))));

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let frames = parser.feed(&bytes);
    assert_eq!(frames.len(), 1);
    assert!(frames[0].is_echo());
    assert_eq!(frames[0].seq, 7);
    assert_eq!(frames[0].data(), b"hello echo");
    let stats = transport.stats();
    assert_eq!(stats.tx_bytes, raw.len() as u64);
    assert_eq!(stats.rx_bytes, raw.len() as u64);
    assert_eq!(stats.errors, 0);
    transport.close_link().expect("关闭");
}

#[test]
fn anomalies_still_parse() {
    let mut stream = vec![0x00u8, 0x11, 0x22]; // 脏字节 → resync
    for seq in 0..45u16 {
        let mut raw = test_frame(seq);
        if seq == 10 {
            let last = raw.len() - 1;
            raw[last] ^= 0xFF; // CRC 错
        }
        stream.extend_from_slice(&raw);
    }

    let port = MockPort::scripted(&stream, 17);
    let mut transport = UsbTransport::new();
    transport
        .open_with_port(Box::new(port), "MOCK")
        .expect("打开");
    let (bytes, _events) = wait_bytes(&mut transport, stream.len(), Duration::from_secs(3));
    transport.close_link().expect("关闭");

    let stats = transport.stats();
    assert_eq!(stats.rx_bytes, stream.len() as u64);
    assert_eq!(stats.dropped, 0);

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let frames = parser.feed(&bytes);
    assert!(
        frames.len() >= 25,
        "frames={} stats={:?}",
        frames.len(),
        parser.stats()
    );
    assert!(parser.stats().crc_err >= 1);
    assert!(parser.stats().resync >= 1);
    assert_eq!(frames.last().map(|frame| frame.seq), Some(44));
}

#[test]
fn device_removed_is_reported_once() {
    let port = MockPort::with_read_error(PortError::DeviceRemoved);
    let mut transport = UsbTransport::new();
    transport
        .open_with_port(Box::new(port), "MOCK")
        .expect("打开");

    let (reason, detail) =
        wait_disconnect(&mut transport, Duration::from_secs(2)).expect("应上报断连");
    assert_eq!(reason, DisconnectReason::DeviceRemoved);
    assert!(detail.is_some());
    assert!(!transport.is_connected());
    assert_eq!(transport.stats().errors, 1);

    // 只上报一次。
    std::thread::sleep(Duration::from_millis(30));
    assert!(transport.events().is_empty());
    assert_eq!(
        transport.write_frame(&test_frame(1)),
        Err(TransportError::NotConnected)
    );
}

#[test]
fn write_before_open_is_not_connected() {
    let mut transport = UsbTransport::new();
    assert_eq!(
        transport.write_frame(&test_frame(1)),
        Err(TransportError::NotConnected)
    );
    assert!(transport.stats().last_error.is_none());
}

#[test]
fn reopen_increments_reconnects() {
    let mut transport = UsbTransport::new();
    transport
        .open_with_port(Box::new(MockPort::new()), "COM_TEST")
        .expect("首次打开");
    assert_eq!(transport.stats().reconnects, 0);
    transport.close_link().expect("关闭");
    transport
        .open_with_port(Box::new(MockPort::new()), "COM_TEST")
        .expect("重开");
    assert_eq!(transport.stats().reconnects, 1);
    let link = transport.link().expect("链路信息");
    assert_eq!(link.address, "COM_TEST");
    assert_eq!(link.max_payload, 1013);
}

#[test]
fn ble_open_is_not_implemented() {
    let mut transport = UsbTransport::new();
    let error = transport
        .open(&TransportCfg::Ble(BleCfg::default()))
        .expect_err("BLE 属 P2c");
    assert_eq!(error.code(), "E_NOT_IMPLEMENTED");
}

#[test]
fn filtered_open_reports_available_ports() {
    let mut transport = UsbTransport::new();
    let cfg = UsbCfg {
        vid: Some(0xFFFE),
        pid: Some(0xFFFF),
        ..UsbCfg::default()
    };
    match transport.open(&TransportCfg::Usb(cfg)) {
        Err(TransportError::PortNotFound { detail, .. }) => {
            assert!(detail.contains("vid=0xFFFE"), "detail={detail}");
        }
        other => panic!("应返回 PortNotFound，实际 {other:?}"),
    }
}

#[test]
fn event_queue_drops_oldest() {
    let queue = EventQueue::new(2);
    queue.push(TransportEvent::Bytes(vec![1]));
    queue.push(TransportEvent::Bytes(vec![2]));
    queue.push(TransportEvent::Bytes(vec![3]));
    let drained = queue.drain();
    assert_eq!(drained.len(), 2);
    assert!(matches!(&drained[0], TransportEvent::Bytes(bytes) if bytes == &[2]));
    assert!(matches!(&drained[1], TransportEvent::Bytes(bytes) if bytes == &[3]));
    assert_eq!(queue.dropped(), 1);
}
