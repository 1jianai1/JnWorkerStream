"""Windows BLE 扫描 / 配对枚举（bleak + WinRT）。

只做设备发现，不建立 GATT 连接。
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional

from bleak import BleakScanner

SERVICE_UUID = "A5A00000-5A5A-4B4E-9C3D-1F2E3D4C5B6A"
DATA_TX_UUID = "A5A00001-5A5A-4B4E-9C3D-1F2E3D4C5B6A"
DATA_RX_UUID = "A5A00002-5A5A-4B4E-9C3D-1F2E3D4C5B6A"
CTRL_UUID = "A5A00003-5A5A-4B4E-9C3D-1F2E3D4C5B6A"


class BleError(RuntimeError):
    """带 E_* 错误码的 BLE 失败。"""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


def format_bt_address(value: int) -> str:
    """``0x5CFD52800C3C`` → ``5C:FD:52:80:0C:3C``。"""
    digits = f"{int(value) & 0xFFFFFFFFFFFF:012X}"
    return ":".join(digits[i : i + 2] for i in range(0, 12, 2))


def normalize_name(name: Optional[str]) -> str:
    return (name or "").replace("\xa0", " ").strip()


def norm_uuid(value: str) -> str:
    return value.strip().lower()


def uuid_in(values: List[str], want: str) -> bool:
    want = norm_uuid(want)
    return any(norm_uuid(value) == want for value in values)


def _winrt_available() -> bool:
    try:
        import winrt.windows.devices.bluetooth  # noqa: F401
        import winrt.windows.devices.enumeration  # noqa: F401
    except ImportError:
        return False
    return True


async def enumerate_paired_async() -> List[Dict[str, Any]]:
    """枚举 Windows 已配对 BLE 设备（含被 HID 占用、不广播的设备）。"""
    if not _winrt_available():
        raise BleError("E_SIDECAR_UNAVAILABLE", "当前环境不支持 WinRT 配对枚举")
    from winrt.windows.devices.bluetooth import BluetoothLEDevice
    from winrt.windows.devices.enumeration import (
        DeviceInformation,
        DeviceInformationKind,
    )

    selector = BluetoothLEDevice.get_device_selector_from_pairing_state(True)
    infos = await DeviceInformation.find_all_async(
        selector, [], DeviceInformationKind.ASSOCIATION_ENDPOINT
    )
    result: List[Dict[str, Any]] = []
    seen = set()
    for info in infos:
        try:
            device = await BluetoothLEDevice.from_id_async(info.id)
        except Exception:
            continue
        if device is None:
            continue
        try:
            address = format_bt_address(device.bluetooth_address)
            if address in seen:
                continue
            seen.add(address)
            try:
                address_type = (
                    "random" if int(device.bluetooth_address_type) == 1 else "public"
                )
            except Exception:
                address_type = "public"
            try:
                connected = int(device.connection_status) == 1
            except Exception:
                connected = False
            result.append(
                {
                    "id": info.id,
                    "name": normalize_name(device.name or info.name or ""),
                    "address": address,
                    "address_type": address_type,
                    "connected": connected,
                    "paired": True,
                }
            )
        finally:
            try:
                device.close()
            except Exception:
                pass
    result.sort(key=lambda item: (not item["connected"], item["name"].casefold()))
    return result


async def scan_async(
    timeout: float = 6.0,
    name_filter: Optional[str] = None,
    service_uuid: str = SERVICE_UUID,
    include_paired: bool = True,
) -> List[Dict[str, Any]]:
    """扫描 + 已配对枚举合并去重。"""
    found = await BleakScanner.discover(timeout=timeout, return_adv=True)
    devices: Dict[str, Dict[str, Any]] = {}
    for address, item in found.items():
        device, adv = item
        name = normalize_name(
            getattr(device, "name", None) or getattr(adv, "local_name", None) or ""
        )
        services = list(getattr(adv, "service_uuids", []) or [])
        devices[address.upper()] = {
            "id": address,
            "name": name,
            "address": address,
            "address_type": None,
            "rssi": getattr(adv, "rssi", None),
            "paired": False,
            "connected": False,
            "services": services,
        }

    if include_paired:
        try:
            for paired in await enumerate_paired_async():
                key = paired["address"].upper()
                existing = devices.get(key)
                if existing is None:
                    devices[key] = {**paired, "rssi": None, "services": []}
                else:
                    existing.update(
                        {
                            "id": paired["id"],
                            "name": existing["name"] or paired["name"],
                            "address_type": paired["address_type"],
                            "paired": True,
                            "connected": paired["connected"],
                        }
                    )
        except Exception:
            pass

    needle = (name_filter or "").strip().casefold()
    result = []
    for item in devices.values():
        if needle and needle not in item["name"].casefold():
            continue
        services = item.pop("services", [])
        if service_uuid and services and not uuid_in(services, service_uuid):
            continue
        item["rssi"] = item.get("rssi")
        result.append(item)
    result.sort(
        key=lambda item: (
            not item["paired"],
            item["rssi"] is None,
            -(item["rssi"] or -999),
            item["name"].casefold(),
        )
    )
    return result


async def find_paired(
    address: Optional[str], name_filter: Optional[str]
) -> Optional[Dict[str, Any]]:
    """在系统配对记录里按地址/名称找设备（不依赖广播）。"""
    try:
        devices = await enumerate_paired_async()
    except Exception:
        return None
    want_address = (address or "").strip().upper()
    want_name = (name_filter or "").strip().casefold()
    candidates = []
    for device in devices:
        if want_address and device["address"].upper() != want_address:
            continue
        if want_name and want_name not in device["name"].casefold():
            continue
        if not want_address and not want_name:
            continue
        candidates.append(device)
    if not candidates:
        return None
    candidates.sort(key=lambda item: (not item["connected"], item["name"].casefold()))
    return candidates[0]
