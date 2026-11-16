"""传输实现：USB CDC、BLE GATT、模拟链路。"""

from __future__ import annotations

from .base import (
    DisconnectCallback,
    EventCallback,
    RxCallback,
    Transport,
    TransportError,
)
from .ble_tx import (
    CTRL_UUID,
    DATA_RX_UUID,
    DATA_TX_UUID,
    SERVICE_UUID,
    BleTransport,
    list_paired_ble_devices,
    scan_ble_devices,
)
from .serial_tx import SerialPortInfo, SerialTransport, list_serial_ports
from .sim_tx import SimTransport

__all__ = [
    "Transport",
    "TransportError",
    "RxCallback",
    "EventCallback",
    "DisconnectCallback",
    "SerialTransport",
    "SerialPortInfo",
    "list_serial_ports",
    "BleTransport",
    "scan_ble_devices",
    "list_paired_ble_devices",
    "SERVICE_UUID",
    "DATA_TX_UUID",
    "DATA_RX_UUID",
    "CTRL_UUID",
    "SimTransport",
]
