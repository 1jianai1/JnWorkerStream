//! 真实 BLE 硬件验收（默认忽略）。
//!
//! 运行前：键盘开机并处于 BLE 模式；设置 `JNWS_PYTHON` 指向带 bleak 的 Python。
//!
//! ```powershell
//! $env:JNWS_PYTHON='D:\Miniconda\envs\kbhost\python.exe'
//! cargo test -p kb-transport --test ble_hardware -- --ignored --nocapture
//! ```
//!
//! 验证：已配对直连（HID 占用不广播也能连）、MTU、DATA_TX Notify、DATA_RX Write、
//! `TEST_DATA` 回显（`A5 5A` 帧与数据逐字节一致）。
#![cfg(windows)]

use std::path::Path;
use std::time::{Duration, Instant};

use kb_audio::{decode_segment, write_wav, AudioSegment, VoiceSession};
use kb_protocol::{
    build_frame, build_test_payload, FrameParser, MAX_PAYLOAD, TYPE_AUDIO_CTL, TYPE_AUDIO_DATA,
    TYPE_TEST_DATA,
};
use kb_transport::{
    BleCfg, BleTransport, ReconnectPolicy, SidecarLaunch, Transport, TransportCfg, TransportEvent,
};

fn address() -> String {
    std::env::var("JNWS_BLE_ADDRESS").unwrap_or_else(|_| "5C:FD:52:80:0C:3C".to_string())
}

fn name() -> String {
    std::env::var("JNWS_BLE_NAME").unwrap_or_else(|_| "SF32-KB-0C3C".to_string())
}

#[test]
#[ignore = "需要真实 BLE 设备，且 JNWS_PYTHON 指向带 bleak 的 Python"]
fn real_ble_echo_roundtrip() {
    let mut transport = BleTransport::new(SidecarLaunch::default());
    let cfg = BleCfg {
        address: Some(address()),
        name: Some(name()),
        paired_only: true,
        scan_timeout_ms: 6_000,
    };
    let link = transport
        .open(&TransportCfg::Ble(cfg))
        .expect("连接失败：确认键盘已开机并在 BLE 模式，且 JNWS_PYTHON 正确");
    println!(
        "connected address={} mtu={} max_payload={}",
        link.address, link.mtu, link.max_payload
    );
    assert!(link.mtu >= 23, "MTU 非法: {}", link.mtu);
    if link.mtu <= 23 {
        println!("WARN: ATT MTU 仅 23，payload 上限 9 B");
    }

    let data: Vec<u8> = (0..32)
        .map(|index| ((7 * 7 + index) & 0xFF) as u8)
        .collect();
    let raw = build_frame(TYPE_TEST_DATA, 0, 7, &build_test_payload(0, &data, 0)).expect("组帧");
    transport.write(&raw).expect("写入 DATA_RX");

    let deadline = Instant::now() + Duration::from_secs(4);
    let mut received = Vec::new();
    while Instant::now() < deadline && received.is_empty() {
        for event in transport.events() {
            match event {
                TransportEvent::Bytes(chunk) => received.extend_from_slice(&chunk),
                TransportEvent::Disconnected { reason, detail } => {
                    panic!("收到断连: {reason:?} {detail:?}");
                }
                TransportEvent::Error(error) => panic!("链路错误: {error}"),
                TransportEvent::Reconnecting { attempt, delay_ms } => {
                    println!("reconnecting attempt={attempt} delay_ms={delay_ms}");
                }
                TransportEvent::Reconnected { link } => {
                    println!("reconnected mtu={} payload={}", link.mtu, link.max_payload);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !received.is_empty(),
        "未收到回显：确认固件 TEST_DATA 回显已启用；也可手动按键看 AUDIO_CTL"
    );
    println!("rx {} B: {}", received.len(), hex(&received));

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let frames = parser.feed(&received);
    assert!(!frames.is_empty(), "回显字节不是完整帧: {}", hex(&received));
    let frame = &frames[0];
    assert!(frame.is_echo(), "回显帧未置 ECHO 标志");
    assert_eq!(frame.seq, 7);
    assert_eq!(frame.data(), data.as_slice());

    let stats = transport.stats();
    println!(
        "stats rx={} tx={} errors={} dropped={}",
        stats.rx_bytes, stats.tx_bytes, stats.errors, stats.dropped
    );
    transport.close().expect("关闭");
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 只监听：用于手动按住设备 PTT，确认 `AUDIO_CTL` / `AUDIO_DATA` 到达。
#[test]
#[ignore = "需要真实 BLE 设备与人工按键"]
fn real_ble_listen_for_ptt() {
    let mut transport = BleTransport::new(SidecarLaunch::default());
    let cfg = BleCfg {
        address: Some(address()),
        name: Some(name()),
        paired_only: true,
        scan_timeout_ms: 6_000,
    };
    let started = Instant::now();
    let link = transport
        .open(&TransportCfg::Ble(cfg))
        .expect("连接失败：确认键盘已开机并在 BLE 模式");
    println!(
        "connected at {:?}: {} mtu={} max_payload={}",
        started.elapsed(),
        link.address,
        link.mtu,
        link.max_payload
    );

    // 先发一帧 TEST_DATA，确认写路径与回显。
    let probe: Vec<u8> = (0..32).map(|index| (index as u8).wrapping_mul(3)).collect();
    let probe_frame =
        build_frame(TYPE_TEST_DATA, 0, 7, &build_test_payload(0, &probe, 0)).expect("组帧");
    match transport.write(&probe_frame) {
        Ok(()) => println!(
            "wrote TEST_DATA {} B at {:?}",
            probe_frame.len(),
            started.elapsed()
        ),
        Err(error) => println!("write failed: {error}"),
    }

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let mut frames_total = 0usize;
    let mut bytes_total = 0usize;
    let listen_secs = std::env::var("JNWS_LISTEN_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(45);
    let deadline = Instant::now() + Duration::from_secs(listen_secs);
    let mut next_report = Instant::now() + Duration::from_secs(5);
    println!("listening {listen_secs}s — 请按住设备 PTT 说 2 秒再松开");
    while Instant::now() < deadline {
        for event in transport.events() {
            match event {
                TransportEvent::Bytes(chunk) => {
                    bytes_total += chunk.len();
                    println!(
                        "rx {} B at {:?}: {}",
                        chunk.len(),
                        started.elapsed(),
                        hex(&chunk)
                    );
                    for frame in parser.feed(&chunk) {
                        frames_total += 1;
                        println!(
                            "  frame type=0x{:02X} {} seq={} flags=0x{:02X} payload={}",
                            frame.ty,
                            frame.type_name().unwrap_or("?"),
                            frame.seq,
                            frame.flags,
                            frame.payload.len()
                        );
                    }
                }
                TransportEvent::Disconnected { reason, detail } => {
                    println!(
                        "disconnected at {:?}: {reason:?} {detail:?}",
                        started.elapsed()
                    );
                }
                TransportEvent::Error(error) => {
                    println!("error at {:?}: {error}", started.elapsed());
                }
                TransportEvent::Reconnecting { attempt, delay_ms } => {
                    println!(
                        "reconnecting attempt={attempt} delay_ms={delay_ms} at {:?}",
                        started.elapsed()
                    );
                }
                TransportEvent::Reconnected { link } => {
                    println!(
                        "reconnected mtu={} payload={} at {:?}",
                        link.mtu,
                        link.max_payload,
                        started.elapsed()
                    );
                }
            }
        }
        if Instant::now() >= next_report {
            next_report += Duration::from_secs(5);
            println!(
                "alive={} at {:?} frames={frames_total} bytes={bytes_total}",
                transport.is_connected(),
                started.elapsed()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "summary frames={frames_total} bytes={bytes_total} parser={:?}",
        parser.stats()
    );
    assert!(
        frames_total > 0,
        "{listen_secs} s 内未收到任何帧：确认按住设备 PTT，且设备在 BLE 模式"
    );
}

/// 端到端：真实 BLE → 解析 → `VoiceSession` 分段 → ADPCM 解码 → WAV。
#[test]
#[ignore = "需要真实 BLE 设备与人工按键"]
fn real_ble_voice_session_wav() {
    let mut transport = BleTransport::new(SidecarLaunch::default());
    let cfg = BleCfg {
        address: Some(address()),
        name: Some(name()),
        paired_only: true,
        scan_timeout_ms: 6_000,
    };
    let started = Instant::now();
    let link = transport
        .open(&TransportCfg::Ble(cfg))
        .expect("连接失败：确认键盘已开机并在 BLE 模式");
    println!(
        "connected at {:?}: {} mtu={}",
        started.elapsed(),
        link.address,
        link.mtu
    );

    // 设备端的自定义链路需要主机先发一帧（与已验证成功的 listener 用例一致）。
    let probe_seq = 0u16;
    send_probe(&mut transport, probe_seq, &started);

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("hw");
    std::fs::create_dir_all(&out_dir).expect("创建 target/hw");
    let listen_secs = std::env::var("JNWS_LISTEN_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let mut session = VoiceSession::new();
    let mut frames = 0usize;
    let mut segments = 0usize;
    let mut wavs: Vec<std::path::PathBuf> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(listen_secs);
    println!("listening {listen_secs}s — 请按住设备 PTT 说 2 秒再松开");

    while Instant::now() < deadline {
        for event in transport.events() {
            let TransportEvent::Bytes(chunk) = event else {
                continue;
            };
            for frame in parser.feed(&chunk) {
                frames += 1;
                let elapsed = started.elapsed().as_millis() as u64;
                match frame.ty {
                    TYPE_AUDIO_CTL => {
                        if let Some(segment) = session.on_ctl_frame(&frame, elapsed) {
                            segments += 1;
                            if let Some(path) =
                                save_segment(&segment, &out_dir, segments, &mut wavs)
                            {
                                println!("wav: {}", path.display());
                            }
                        }
                    }
                    TYPE_AUDIO_DATA => {
                        let _ = session.feed(&frame, elapsed);
                    }
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    if let Some(discarded) = session.discard_open("listen timeout") {
        println!(
            "discarded open segment seg={} blocks={}",
            discarded.seg_id,
            discarded.received()
        );
    }
    let (received, total, missing) = session.stats();
    println!(
        "summary frames={frames} segments={segments} blocks={received}/{total} \
         missing={} interrupted={} parser={:?}",
        missing.len(),
        session.interrupted(),
        parser.stats()
    );
    assert!(frames > 0, "未收到任何帧");
    assert!(segments > 0, "未完成任何语音段（是否按下 PTT？）");
    assert!(!wavs.is_empty(), "没有写出 WAV");
    for path in &wavs {
        let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        assert!(size > 44, "WAV 只有文件头：{}", path.display());
        println!("WAV {} bytes: {}", size, path.display());
    }
}

/// 断线自动重连：连接成功后请关闭设备（或断电/让设备离开范围），再重新开机。
///
/// 预期事件序列：`Disconnected` → `Reconnecting{attempt,delay_ms}` → … → `Reconnected`。
#[test]
#[ignore = "需要真实 BLE 设备与人工断电/重启"]
fn real_ble_auto_reconnect() {
    let mut transport = BleTransport::new(SidecarLaunch::default());
    transport.set_reconnect_policy(ReconnectPolicy::default());
    let cfg = BleCfg {
        address: Some(address()),
        name: Some(name()),
        paired_only: true,
        scan_timeout_ms: 6_000,
    };
    let wait_secs = std::env::var("JNWS_RECONNECT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(180);

    let started = Instant::now();
    let link = transport
        .open(&TransportCfg::Ble(cfg))
        .expect("连接失败：确认键盘已开机并在 BLE 模式");
    println!(
        "connected at {:?}: {} mtu={} max_payload={}",
        started.elapsed(),
        link.address,
        link.mtu,
        link.max_payload
    );
    println!("请现在关闭设备（或断电），再重新开机；最多等待 {wait_secs} s 观察自动重连");

    let deadline = Instant::now() + Duration::from_secs(wait_secs);
    let mut drops = 0usize;
    let mut reconnected = None;
    while Instant::now() < deadline {
        for event in transport.events() {
            match event {
                TransportEvent::Bytes(_) => {}
                TransportEvent::Disconnected { reason, detail } => {
                    drops += 1;
                    println!(
                        "disconnected#{drops} at {:?}: {reason:?} {detail:?}",
                        started.elapsed()
                    );
                }
                TransportEvent::Reconnecting { attempt, delay_ms } => {
                    println!(
                        "retry#{attempt} in {delay_ms} ms at {:?}",
                        started.elapsed()
                    );
                }
                TransportEvent::Reconnected { link } => {
                    println!(
                        "reconnected at {:?}: mtu={} payload={}",
                        started.elapsed(),
                        link.mtu,
                        link.max_payload
                    );
                    reconnected = Some(link);
                }
                TransportEvent::Error(error) => {
                    println!("error at {:?}: {error}", started.elapsed());
                }
            }
        }
        if reconnected.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(drops > 0, "未观察到断连：确认已关闭设备或让设备离开范围");
    let link = reconnected.expect("超时内未自动重连：确认设备已重新开机");
    assert!(transport.is_connected());
    assert!(link.mtu >= 23, "重连后 MTU 非法: {}", link.mtu);
    assert_eq!(
        transport.reconnect_status().attempt,
        0,
        "重连成功后应清零重试计数"
    );
    println!("stats: {:?}", transport.stats());
    transport.close().expect("关闭");
}

/// 30 分钟链路长稳：只验证「连上后不掉线」，不需要说话或按键。
///
/// 期间持续调用 `events()`（自动重连也由它驱动），每 30 s 打印一次存活与计数。
#[test]
#[ignore = "需要真实 BLE 设备，默认跑 30 分钟"]
fn real_ble_soak() {
    let mut transport = BleTransport::new(SidecarLaunch::default());
    let cfg = BleCfg {
        address: Some(address()),
        name: Some(name()),
        paired_only: true,
        scan_timeout_ms: 6_000,
    };
    let secs = std::env::var("JNWS_LISTEN_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_800);

    let started = Instant::now();
    let link = transport
        .open(&TransportCfg::Ble(cfg))
        .expect("连接失败：确认键盘已开机并在 BLE 模式");
    println!(
        "connected at {:?}: {} mtu={}",
        started.elapsed(),
        link.address,
        link.mtu
    );
    println!("soak {secs}s — 不需要按键，保持设备开机即可");

    let mut parser = FrameParser::new(MAX_PAYLOAD);
    let mut frames = 0usize;
    let mut bytes = 0usize;
    let mut drops = 0usize;
    let mut last_report = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(secs);

    while Instant::now() < deadline {
        for event in transport.events() {
            match event {
                TransportEvent::Bytes(chunk) => {
                    bytes += chunk.len();
                    frames += parser.feed(&chunk).len();
                }
                TransportEvent::Disconnected { reason, detail } => {
                    drops += 1;
                    println!(
                        "disconnected#{drops} at {:?}: {reason:?} {detail:?}",
                        started.elapsed()
                    );
                }
                TransportEvent::Reconnecting { attempt, delay_ms } => {
                    println!(
                        "retry#{attempt} in {delay_ms} ms at {:?}",
                        started.elapsed()
                    );
                }
                TransportEvent::Reconnected { link } => {
                    println!("reconnected at {:?}: mtu={}", started.elapsed(), link.mtu);
                }
                TransportEvent::Error(error) => {
                    println!("error at {:?}: {error}", started.elapsed());
                }
            }
        }
        if last_report.elapsed() >= Duration::from_secs(30) {
            last_report = Instant::now();
            println!(
                "alive={} at {:?} frames={frames} bytes={bytes} drops={drops}",
                transport.is_connected(),
                started.elapsed()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let stats = transport.stats();
    println!(
        "soak summary elapsed={:?} frames={frames} bytes={bytes} drops={drops} \
         parser={:?} stats={stats:?}",
        started.elapsed(),
        parser.stats()
    );
    transport.close().expect("关闭");
    assert!(
        stats.reconnects == 0,
        "长稳期间发生了 {} 次重连：链路不稳定",
        stats.reconnects
    );
    assert_eq!(drops, 0, "长稳期间出现断连事件");
}

fn send_probe(transport: &mut BleTransport, seq: u16, started: &Instant) {
    let payload = [0u8; 8];
    let frame =
        build_frame(TYPE_TEST_DATA, 0, seq, &build_test_payload(0, &payload, 0)).expect("组帧");
    match transport.write(&frame) {
        Ok(()) => println!("probe seq={seq} sent at {:?}", started.elapsed()),
        Err(error) => println!("probe failed at {:?}: {error}", started.elapsed()),
    }
}

fn save_segment(
    segment: &AudioSegment,
    out_dir: &Path,
    index: usize,
    wavs: &mut Vec<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let (pcm, missing) = decode_segment(segment);
    println!(
        "segment seg={} blocks={}/{} missing={:?} duration={} ms pcm={} samples",
        segment.seg_id,
        segment.received(),
        segment.total(),
        missing,
        segment.duration_ms(),
        pcm.len()
    );
    if pcm.is_empty() {
        return None;
    }
    let path = out_dir.join(format!("seg{}_{:02}.wav", segment.seg_id, index));
    match write_wav(&path, &pcm) {
        Ok(_) => {
            wavs.push(path.clone());
            Some(path)
        }
        Err(error) => {
            println!("write wav failed: {error}");
            None
        }
    }
}
