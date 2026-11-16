//! 真实 Windows 串口环境的轻量自检（无需键盘硬件）。
//!
//! 只验证枚举与打开失败的映射；真机收数/拔插属 P2a 硬件验收项。

#![cfg(windows)]

use kb_transport::{list_ports, open_port, Transport, TransportCfg, UsbCfg, UsbTransport};

#[test]
fn enumerate_ports_is_sorted_and_never_fails() {
    let ports = list_ports().expect("枚举串口不应失败");
    let mut sorted = ports.clone();
    sorted.sort_by(|left, right| left.device.cmp(&right.device));
    assert_eq!(ports, sorted);
    println!(
        "enumerated {} port(s): {:?}",
        ports.len(),
        ports.iter().map(|port| port.pretty()).collect::<Vec<_>>()
    );
}

#[test]
fn opening_missing_port_maps_to_port_not_found() {
    match open_port("COM199") {
        Ok(_) => panic!("COM199 不应存在"),
        Err(error) => assert_eq!(error.code(), "E_PORT_NOT_FOUND", "{error}"),
    }
}

#[test]
fn explicit_missing_port_through_transport() {
    let mut transport = UsbTransport::new();
    let config = TransportCfg::Usb(UsbCfg {
        port: Some("COM199".to_string()),
        ..UsbCfg::default()
    });
    let error = transport.open(&config).expect_err("打开不存在的端口应失败");
    assert_eq!(error.code(), "E_PORT_NOT_FOUND", "{error}");
    assert!(!transport.is_connected());
}
