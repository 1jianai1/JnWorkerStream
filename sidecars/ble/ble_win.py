"""Windows BLE GATT 连接（bleak + WinRT）。

只做“连接/MTU/Notify/Write/字节转发”，不解析 `A5 5A` 帧。
设备被系统 HID 占用、不再广播时，按 Windows 配对记录直连（skill 要求）。
"""

from __future__ import annotations

import asyncio
import time
from typing import Any, Callable, Dict, Optional

from bleak import BleakClient
from bleak.backends.device import BLEDevice

try:
    from .ble_scan import (
        DATA_RX_UUID,
        DATA_TX_UUID,
        BleError,
        find_paired,
        norm_uuid,
        scan_async,
    )
except ImportError:  # 直接以脚本运行
    from ble_scan import (  # type: ignore[no-redef]
        DATA_RX_UUID,
        DATA_TX_UUID,
        BleError,
        find_paired,
        norm_uuid,
        scan_async,
    )

Log = Callable[[str], None]


def _find_characteristic(services: Any, uuid: str) -> Any:
    try:
        char = services.get_characteristic(uuid)
        if char is not None:
            return char
    except Exception:
        pass
    want = norm_uuid(uuid)
    try:
        for service in services:
            for char in getattr(service, "characteristics", []):
                if norm_uuid(char.uuid) == want:
                    return char
    except Exception:
        pass
    return None


class BleLink:
    """一次 BLE GATT 连接的生命周期。"""

    def __init__(
        self,
        on_bytes: Callable[[bytes], None],
        on_disconnect: Callable[[str], None],
        log: Log,
    ) -> None:
        self._on_bytes = on_bytes
        self._on_disconnect = on_disconnect
        self._log = log
        self._client: Optional[BleakClient] = None
        self._address: Optional[str] = None
        self._name: str = ""
        self._mtu = 23
        self._notified = False
        self._connected = False
        self._address_type: Optional[str] = None

    @property
    def connected(self) -> bool:
        return self._connected and self._client is not None

    @property
    def mtu(self) -> int:
        return max(23, int(self._mtu))

    @property
    def max_payload(self) -> int:
        return max(0, self.mtu - 14)

    def _handle_disconnect(self, _client: Any) -> None:
        was_connected = self._connected
        self._connected = False
        self._notified = False
        if was_connected:
            self._log("BLE link lost")
            try:
                self._on_disconnect("link_lost")
            except Exception as exc:  # 事件失败不影响进程
                self._log(f"disconnect callback error: {exc!r}")

    def _handle_notify(self, _sender: Any, data: bytearray) -> None:
        if not data:
            return
        try:
            self._on_bytes(bytes(data))
        except Exception as exc:
            self._log(f"rx callback error: {exc!r}")

    async def _open_client(self, device: Any, timeout: float) -> BleakClient:
        winrt_args: Dict[str, Any] = {"use_cached_services": False}
        if self._address_type in ("public", "random"):
            winrt_args["address_type"] = self._address_type
        try:
            client = BleakClient(
                device,
                disconnected_callback=self._handle_disconnect,
                timeout=max(10.0, timeout),
                winrt=winrt_args,
            )
        except TypeError:
            winrt_args.pop("use_cached_services", None)
            client = BleakClient(
                device,
                disconnected_callback=self._handle_disconnect,
                timeout=max(10.0, timeout),
                winrt=winrt_args,
            )
        await asyncio.wait_for(client.connect(), timeout=timeout)
        return client

    async def _services(self, client: BleakClient) -> Any:
        services = getattr(client, "services", None)
        if asyncio.iscoroutine(services):
            services = await services
        if services:
            return services
        getter = getattr(client, "get_services", None)
        if getter is not None:
            return await getter()
        return services

    async def _resolve_device(
        self, address: Optional[str], name: Optional[str], timeout: float
    ) -> Any:
        if address:
            meta = await find_paired(address, None)
            if meta is not None:
                self._address_type = meta.get("address_type")
                self._log(
                    f"direct paired {meta['name']} @ {meta['address']} "
                    f"({self._address_type or 'auto'})"
                )
                return BLEDevice(meta["address"], meta["name"] or meta["address"], None)
            self._log(f"direct address {address} (no paired record)")
            return BLEDevice(address, name or address, None)

        meta = await find_paired(None, name)
        if meta is not None:
            self._address_type = meta.get("address_type")
            self._log(
                f"paired record {meta['name']} @ {meta['address']} "
                f"({self._address_type or 'auto'})"
            )
            return BLEDevice(meta["address"], meta["name"] or meta["address"], None)

        self._log(f"scanning {timeout:.0f}s for name={name!r} ...")
        devices = await scan_async(timeout=min(timeout, 10.0), name_filter=name)
        if not devices:
            raise BleError("E_BLE_DEVICE_NOT_FOUND", "未扫描到匹配设备")
        best = devices[0]
        self._address_type = best.get("address_type")
        self._log(f"found {best['name']} @ {best['address']}")
        return BLEDevice(best["address"], best["name"] or best["address"], None)

    async def connect(
        self,
        address: Optional[str] = None,
        name: Optional[str] = None,
        paired_only: bool = False,
        timeout: float = 30.0,
    ) -> Dict[str, Any]:
        if self.connected:
            return self.info()
        if paired_only and not (address or name):
            raise BleError("E_INVALID_ARG", "paired_only 需要同时指定 address 或 name")

        device = await self._resolve_device(address, name, timeout)
        client = await self._open_client(device, timeout)
        try:
            services = await self._services(client)
            tx_char = _find_characteristic(services, DATA_TX_UUID)
            rx_char = _find_characteristic(services, DATA_RX_UUID)
            if tx_char is None or rx_char is None:
                # 重连时句柄可能未稳定：清掉再试一次。
                try:
                    await client.disconnect()
                except Exception:
                    pass
                await asyncio.sleep(0.8)
                client = await self._open_client(device, timeout)
                services = await self._services(client)
                tx_char = _find_characteristic(services, DATA_TX_UUID)
                rx_char = _find_characteristic(services, DATA_RX_UUID)
            if tx_char is None or rx_char is None:
                available = []
                try:
                    for service in services:
                        available.extend(
                            norm_uuid(char.uuid)
                            for char in getattr(service, "characteristics", [])
                        )
                except Exception:
                    pass
                raise BleError(
                    "E_SIDECAR_PROTOCOL",
                    f"GATT 中找不到 DATA_TX/DATA_RX；当前特征: {available or '(无)'}",
                )
            self._client = client
            self._address = getattr(device, "address", None)
            self._name = getattr(device, "name", "") or ""
            self._mtu = self._read_mtu(client)
            await client.start_notify(tx_char.uuid, self._handle_notify)
            self._notified = True
            # Windows 上 MTU 常在订阅后才升到 247。
            for _ in range(6):
                if self._read_mtu(client) > 23:
                    break
                await asyncio.sleep(0.1)
            self._mtu = self._read_mtu(client)
            self._connected = True
            self._log(
                f"connected {self._address} mtu={self._mtu} max_payload={self.max_payload}"
            )
            return self.info()
        except Exception:
            try:
                await client.disconnect()
            except Exception:
                pass
            raise

    @staticmethod
    def _read_mtu(client: BleakClient) -> int:
        mtu = getattr(client, "mtu_size", None)
        return int(mtu) if isinstance(mtu, int) and mtu >= 23 else 23

    def info(self) -> Dict[str, Any]:
        return {
            "address": self._address or "",
            "name": self._name,
            "mtu": self.mtu,
            "address_type": self._address_type,
            "connected_at_ms": int(time.time() * 1000),
        }

    async def set_notify(self, enabled: bool) -> None:
        if self._client is None or not self.connected:
            raise BleError("E_NOT_CONNECTED", "BLE 未连接")
        if enabled and not self._notified:
            await self._client.start_notify(DATA_TX_UUID, self._handle_notify)
            self._notified = True
        elif not enabled and self._notified:
            await self._client.stop_notify(DATA_TX_UUID)
            self._notified = False

    async def write(self, data: bytes) -> int:
        if not self.connected or self._client is None:
            raise BleError("E_NOT_CONNECTED", "BLE 未连接，无法写入")
        if len(data) > self.mtu - 3:
            raise BleError(
                "E_PAYLOAD_TOO_LARGE",
                f"单包 {len(data)} B 超过 ATT MTU-3={self.mtu - 3} B",
            )
        await self._client.write_gatt_char(DATA_RX_UUID, data, response=False)
        return len(data)

    async def disconnect(self) -> None:
        client, self._client = self._client, None
        self._connected = False
        if client is None:
            return
        try:
            if self._notified:
                try:
                    await client.stop_notify(DATA_TX_UUID)
                except Exception:
                    pass
                self._notified = False
            if getattr(client, "is_connected", False):
                await client.disconnect()
        finally:
            self._log("disconnected")
