"""BLE GATT 传输（bleak）。

设备侧对应文档 3 的私有服务::

    私有服务   A5A00000-5A5A-4B4E-9C3D-1F2E3D4C5B6A
    DATA_TX    A5A00001-...  Notify（设备 -> 主机）
    DATA_RX    A5A00002-...  Write / Write Without Response（主机 -> 设备）

要点：

- 扫描时按服务 UUID 过滤，也支持 ``--name`` / ``--address``；
- 连接后读取**实际** ``client.mtu_size``，不硬编码 247；
- 断开后指数退避自动重连，重连后重新订阅 ``DATA_TX``；
- 底层只搬字节，不解析帧。

依赖：``pip install bleak``（Windows 需要 Win10 1809+）。
"""

from __future__ import annotations

import asyncio
import sys
import threading
import time
from typing import Any, Dict, List, Optional, Sequence, Tuple

from .base import Transport, TransportError

try:  # bleak 是可选依赖
    from bleak import BleakClient, BleakScanner  # type: ignore
    from bleak.backends.device import BLEDevice  # type: ignore
except ImportError:  # pragma: no cover - 环境相关
    BleakClient = None  # type: ignore[assignment]
    BleakScanner = None  # type: ignore[assignment]
    BLEDevice = None  # type: ignore[assignment]


SERVICE_UUID = "A5A00000-5A5A-4B4E-9C3D-1F2E3D4C5B6A"
DATA_TX_UUID = "A5A00001-5A5A-4B4E-9C3D-1F2E3D4C5B6A"
DATA_RX_UUID = "A5A00002-5A5A-4B4E-9C3D-1F2E3D4C5B6A"
CTRL_UUID = "A5A00003-5A5A-4B4E-9C3D-1F2E3D4C5B6A"


def _norm_uuid(value: str) -> str:
    return value.strip().lower()


def _uuid_in(values: Sequence[str], want: str) -> bool:
    want = _norm_uuid(want)
    return any(_norm_uuid(v) == want for v in values)


def _format_bt_address(value: int) -> str:
    """0x5CFD52800C3C -> ``5C:FD:52:80:0C:3C``。"""
    digits = f"{int(value) & 0xFFFFFFFFFFFF:012X}"
    return ":".join(digits[i : i + 2] for i in range(0, 12, 2))


def _normalize_name(name: str) -> str:
    # 某些设备名里用 NBSP 分隔（如 PMO\xa0P75）
    return (name or "").replace("\xa0", " ").strip()


def _winrt_available() -> bool:
    if sys.platform != "win32":
        return False
    try:
        import winrt.windows.devices.bluetooth  # noqa: F401
        import winrt.windows.devices.enumeration  # noqa: F401
    except ImportError:
        return False
    return True


async def _enumerate_paired_ble_async() -> List[Dict[str, Any]]:
    """枚举 Windows 已配对的 BLE 设备（包括当前被系统 HID 占用、不再广播的设备）。

    ``--scan-ble`` / ``--list-paired`` 与自动回退都会用到。地址类型（public/random）
    一并返回，直连随机地址设备时需要它。
    """
    from winrt.windows.devices.bluetooth import BluetoothLEDevice
    from winrt.windows.devices.enumeration import DeviceInformation, DeviceInformationKind

    selector = BluetoothLEDevice.get_device_selector_from_pairing_state(True)
    infos = await DeviceInformation.find_all_async(
        selector, [], DeviceInformationKind.ASSOCIATION_ENDPOINT
    )
    result: List[Dict[str, Any]] = []
    seen = set()
    for info in infos:
        try:
            ble = await BluetoothLEDevice.from_id_async(info.id)
        except Exception:
            continue
        if ble is None:
            continue
        try:
            address = _format_bt_address(ble.bluetooth_address)
            if address in seen:
                continue
            seen.add(address)
            try:
                is_random = int(ble.bluetooth_address_type) == 1
            except Exception:
                is_random = False
            try:
                connected = int(ble.connection_status) == 1
            except Exception:
                connected = False
            result.append(
                {
                    "name": _normalize_name(ble.name or info.name or ""),
                    "address": address,
                    "address_type": "random" if is_random else "public",
                    "address_value": int(ble.bluetooth_address),
                    "connected": connected,
                    "paired": True,
                    "id": info.id,
                }
            )
        finally:
            try:
                ble.close()
            except Exception:
                pass
    result.sort(key=lambda d: (not d["connected"], d["name"].casefold(), d["address"]))
    return result


def list_paired_ble_devices() -> List[Dict[str, Any]]:
    """同步列出 Windows 已配对 BLE 设备，供 ``--list-paired`` 使用。"""
    if not _winrt_available():
        raise TransportError(
            "当前系统/环境不支持枚举 Windows 已配对 BLE 设备（需 Windows + bleak/winrt）"
        )
    try:
        return asyncio.run(_enumerate_paired_ble_async())
    except Exception as exc:
        raise TransportError(f"枚举 Windows 已配对 BLE 设备失败: {exc}") from exc


def scan_ble_devices(timeout: float = 5.0) -> List[Dict[str, Any]]:
    """同步扫描周围 BLE 设备，供 ``--scan-ble`` 使用。"""
    if BleakScanner is None:
        raise TransportError(
            "未安装 bleak，无法扫描 BLE。请执行: pip install -r host_test/requirements.txt"
        )

    async def _run() -> List[Dict[str, Any]]:
        try:
            found = await BleakScanner.discover(timeout=timeout, return_adv=True)
        except TypeError:  # 旧版 bleak 没有 return_adv
            devices = await BleakScanner.discover(timeout=timeout)
            return [
                {
                    "name": d.name or "",
                    "address": d.address,
                    "rssi": getattr(d, "rssi", None),
                    "services": [],
                }
                for d in devices
            ]
        result: List[Dict[str, Any]] = []
        if isinstance(found, dict):
            iterator = found.values()
        else:
            iterator = [(d, None) for d in found]  # type: ignore[assignment]
        for item in iterator:
            dev, adv = item
            name = (getattr(dev, "name", None) or getattr(adv, "local_name", None) or "")
            result.append(
                {
                    "name": name,
                    "address": dev.address,
                    "rssi": getattr(adv, "rssi", None) if adv is not None else None,
                    "services": list(getattr(adv, "service_uuids", []) or []),
                }
            )
        result.sort(key=lambda d: (d["rssi"] is None, -(d["rssi"] or -999)))
        return result

    return asyncio.run(_run())


class BleTransport(Transport):
    """BLE 私有 GATT 传输。"""

    name = "BLE-GATT"
    auto_reconnect = True

    def __init__(
        self,
        service_uuid: str = SERVICE_UUID,
        tx_uuid: str = DATA_TX_UUID,
        rx_uuid: str = DATA_RX_UUID,
        name_filter: Optional[str] = None,
        address: Optional[str] = None,
        paired: bool = False,
        scan_timeout: float = 6.0,
        connect_timeout: float = 30.0,
        retry_delay: float = 2.0,
        max_backoff: float = 30.0,
        disconnect_callback: Optional[Any] = None,
    ) -> None:
        super().__init__()
        self.service_uuid = service_uuid
        self.tx_uuid = tx_uuid
        self.rx_uuid = rx_uuid
        self.name_filter = name_filter or None
        self.address = address or None
        self.paired = bool(paired)
        self.scan_timeout = float(scan_timeout)
        self.connect_timeout = float(connect_timeout)
        self.retry_delay = max(0.2, float(retry_delay))
        self.max_backoff = max(self.retry_delay, float(max_backoff))

        self._loop: Optional[asyncio.AbstractEventLoop] = None
        self._thread: Optional[threading.Thread] = None
        self._client: Optional[Any] = None
        self._running = False
        self._mtu = 23
        self._connected_evt = threading.Event()
        self._disconnected_evt = threading.Event()
        self._last_error: Optional[str] = None
        self._device_address: Optional[str] = None
        self._notified = False

        # 已配对直连相关
        self._pending_address_type: Optional[str] = None
        self._last_paired_meta: Optional[Dict[str, Any]] = None
        self._paired_direct_failed = False
        self._paired_failures = 0
        self._device_source = "scan"

    # -- 能力 -------------------------------------------------------------

    @property
    def mtu(self) -> int:
        return max(23, int(self._mtu))

    @property
    def max_payload(self) -> int:
        # 文档 3 §4：单帧上限 = ATT_MTU - 3(ATT 头) - 11(帧头+CRC)
        return max(0, self.mtu - 14)

    @property
    def connected(self) -> bool:
        return self._connected and self._client is not None

    # -- 打开/关闭 ---------------------------------------------------------

    def open(self) -> None:
        if BleakClient is None or BleakScanner is None:
            raise TransportError(
                "未安装 bleak，无法使用 BLE。请执行: "
                "pip install -r host_test/requirements.txt"
            )
        if self.paired and not (self.name_filter or self.address):
            raise TransportError("--paired 需要同时指定 --name 或 --address")
        if self._thread is not None and self._thread.is_alive():
            if self.connected:
                return
            self.close()

        self._running = True
        self._connected_evt.clear()
        self._disconnected_evt.clear()
        self._last_error = None
        self._thread = threading.Thread(target=self._thread_main, name="ble-rx", daemon=True)
        self._thread.start()
        if self.address:
            mode = f"address={self.address} (direct, no scan)"
        elif self.paired:
            mode = f"paired devices name={self.name_filter or '*'} (no scan)"
        elif self.name_filter:
            mode = (
                f"name={self.name_filter} (paired-first, then scan "
                f"{self.scan_timeout:.0f}s)"
            )
        else:
            mode = (
                f"scan service={self.service_uuid} name=* "
                f"timeout={self.scan_timeout:.0f}s"
            )
        self._emit(f"LINK  BLE {mode}")
        if not self._connected_evt.wait(self.connect_timeout):
            reason = self._last_error or "扫描/连接超时"
            self.close()
            raise TransportError(
                f"BLE 连接失败: {reason}（可先用 --list-paired 查看系统已配对设备，"
                f"再用 --paired，见 README 故障排查）"
            )

    def close(self) -> None:
        self._running = False
        loop = self._loop
        client = self._client
        if loop is not None and loop.is_running() and client is not None:
            try:
                fut = asyncio.run_coroutine_threadsafe(self._async_disconnect(client), loop)
                fut.result(timeout=3.0)
            except Exception:
                pass
        thread = self._thread
        self._thread = None
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=max(5.0, self.scan_timeout + 2.0))
        self._connected = False
        self._client = None
        self._emit("LINK  closed")

    # -- 后台事件循环 -----------------------------------------------------

    def _thread_main(self) -> None:
        loop = asyncio.new_event_loop()
        self._loop = loop
        try:
            loop.run_until_complete(self._session())
        except Exception as exc:  # 兜底：线程不能静默崩掉
            self._last_error = f"BLE 事件循环异常: {exc}"
            self._emit(self._last_error)
        finally:
            try:
                pending = [t for t in asyncio.all_tasks(loop) if not t.done()]
                for task in pending:
                    task.cancel()
                if pending:
                    loop.run_until_complete(
                        asyncio.gather(*pending, return_exceptions=True)
                    )
                loop.run_until_complete(loop.shutdown_asyncgens())
            except Exception:
                pass
            loop.close()
            self._loop = None

    async def _session(self) -> None:
        backoff = self.retry_delay
        while self._running:
            device = await self._find_device()
            if not self._running:
                break
            if device is None:
                if not self._last_error:
                    self._last_error = "未扫描到匹配设备"
                self._emit(f"BLE  no device, retry in {backoff:.0f}s ...")
                await self._sleep(backoff)
                backoff = min(backoff * 2.0, self.max_backoff)
                continue

            try:
                await self._connect(device)
            except Exception as exc:
                self._last_error = str(exc)
                self._emit(f"BLE  connect failed: {exc}")
                if self._device_source in ("paired", "paired-last"):
                    # 配对记录可能过期（例如设备换了随机地址）；连续失败后退回广播扫描。
                    self._paired_failures += 1
                    if self._paired_failures >= 3:
                        self._paired_direct_failed = True
                await self._cleanup_client()
                await self._sleep(backoff)
                backoff = min(backoff * 2.0, self.max_backoff)
                continue

            backoff = self.retry_delay
            self._disconnected_evt.clear()
            self._connected_evt.set()
            while self._running and not self._disconnected_evt.is_set():
                await asyncio.sleep(0.2)

            if self._running and self._disconnected_evt.is_set():
                self._emit("BLE  link lost, reconnecting ...")
                self._notify_disconnect("BLE link lost")
            await self._cleanup_client()
            if self._running:
                self._emit(f"BLE  retry in {self.retry_delay:.1f}s ...")
                await self._sleep(self.retry_delay)

        await self._cleanup_client()

    async def _sleep(self, seconds: float) -> None:
        loop = asyncio.get_running_loop()
        deadline = loop.time() + seconds
        while self._running:
            remain = deadline - loop.time()
            if remain <= 0:
                return
            await asyncio.sleep(min(0.2, remain))

    # -- 扫描 -------------------------------------------------------------

    async def _scan_once(self, use_filter: bool) -> Any:
        kwargs: Dict[str, Any] = {"timeout": self.scan_timeout, "return_adv": True}
        if use_filter and self.service_uuid:
            try:
                return await BleakScanner.discover(
                    service_uuids=[self.service_uuid], **kwargs
                )
            except TypeError:
                # 旧版 bleak / 平台不支持 service_uuids：退化为全扫描后手动过滤
                pass
            except Exception as exc:
                self._emit(f"BLE  scan with filter failed: {exc}")
        return await BleakScanner.discover(**kwargs)

    @staticmethod
    def _iter_found(found: Any) -> List[Tuple[Any, Any]]:
        if isinstance(found, dict):
            return [tuple(v) for v in found.values()]  # type: ignore[misc]
        return [(d, None) for d in (found or [])]

    def _pick_device(self, found: Any) -> Optional[Any]:
        candidates: List[Tuple[int, Any]] = []
        for dev, adv in self._iter_found(found):
            name = (
                getattr(dev, "name", None)
                or getattr(adv, "local_name", None)
                or ""
            ) if adv is not None else (getattr(dev, "name", None) or "")
            adv_services = list(getattr(adv, "service_uuids", []) or []) if adv is not None else []
            address = getattr(dev, "address", "") or ""

            if self.address and address.lower() != self.address.lower():
                continue
            if self.name_filter and self.name_filter.lower() not in str(name).lower():
                continue
            if self.service_uuid and adv_services and not _uuid_in(adv_services, self.service_uuid):
                continue

            score = 0
            if self.service_uuid and _uuid_in(adv_services, self.service_uuid):
                score += 10
            if self.name_filter and self.name_filter.lower() in str(name).lower():
                score += 5
            rssi = getattr(adv, "rssi", None) if adv is not None else None
            if isinstance(rssi, int):
                score += max(-50, min(0, rssi // 2))
            candidates.append((score, dev))

        if not candidates:
            return None
        candidates.sort(key=lambda x: x[0], reverse=True)
        if len(candidates) > 1:
            names = ", ".join(
                f"{getattr(d, 'name', '') or '?'}@{getattr(d, 'address', '?')}"
                for _, d in candidates[:5]
            )
            self._emit(f"BLE  {len(candidates)} candidates, using best: {names}")
        return candidates[0][1]

    async def _find_paired(self, address: Optional[str] = None) -> Optional[Dict[str, Any]]:
        """在 Windows 已配对设备里找 name/address 匹配项（不依赖广播）。"""
        if not _winrt_available():
            return None
        try:
            devices = await _enumerate_paired_ble_async()
        except Exception as exc:
            self._emit(f"BLE  enumerate paired devices failed: {exc}")
            return None

        want_addr = (address or self.address or "").strip().lower()
        want_name = (self.name_filter or "").strip().casefold()
        candidates: List[Dict[str, Any]] = []
        for d in devices:
            if want_addr and d["address"].lower() != want_addr:
                continue
            if want_name and want_name not in d["name"].casefold():
                continue
            if not want_addr and not want_name:
                continue
            candidates.append(d)

        if not candidates:
            if self.paired:
                available = ", ".join(
                    f"{d['name'] or '?'}@{d['address']}" for d in devices[:8]
                ) or "(无已配对设备)"
                self._emit(f"BLE  paired device not found, available: {available}")
            return None

        if len(candidates) > 1:
            candidates.sort(key=lambda d: (not d["connected"], d["name"].casefold()))
            listed = ", ".join(f"{d['name']}@{d['address']}" for d in candidates[:5])
            self._emit(f"BLE  {len(candidates)} paired candidates, using best: {listed}")
        return candidates[0]

    def _make_direct_device(self, meta: Dict[str, Any], source: str) -> Any:
        if BLEDevice is None:
            raise TransportError("bleak 未正确安装，无法构造直接连接设备")
        self._pending_address_type = meta.get("address_type")
        self._device_source = source
        conn = "connected" if meta.get("connected") else "paired"
        self._emit(
            f"BLE  direct {conn} {meta.get('name') or '?'} @ {meta['address']} "
            f"({self._pending_address_type or 'auto'}, no scan)"
        )
        return BLEDevice(meta["address"], meta.get("name") or meta["address"], None)

    async def _find_device(self) -> Optional[Any]:
        # 1) 重连：先用上次成功的地址直连，避免每次等待扫描。
        if self._last_paired_meta is not None and not self._paired_direct_failed:
            return self._make_direct_device(self._last_paired_meta, "paired-last")

        # 2) 明确指定地址：不做广播扫描。设备被 Windows 作为 HID 占用时不会广播，
        #    但系统配对记录仍能让 BLE 栈复用/建立 ACL 连接。
        if self.address:
            meta = await self._find_paired(address=self.address)
            if meta is not None:
                return self._make_direct_device(meta, "address")
            self._pending_address_type = None
            self._device_source = "address"
            self._emit(f"BLE  direct address {self.address} (no scan)")
            if BLEDevice is None:
                return None
            return BLEDevice(self.address, self.name_filter or self.address, None)

        # 3) --paired：只查系统配对记录，不扫描。
        if self.paired:
            meta = await self._find_paired()
            if meta is not None:
                return self._make_direct_device(meta, "paired")
            self._last_error = "未找到匹配的 Windows 已配对 BLE 设备"
            return None

        # 4) 有同名配对记录时优先直连：
        #    设备作为 HID 被系统占用时不会广播，先扫描只会白等 6 秒。
        if not self._paired_direct_failed and self.name_filter:
            meta = await self._find_paired()
            if meta is not None:
                self._emit("BLE  paired record found, connecting directly (no scan) ...")
                return self._make_direct_device(meta, "paired")

        # 5) 正常广播扫描（先按服务 UUID 过滤）
        found = await self._scan_once(True)
        device = self._pick_device(found)
        if device is not None:
            self._pending_address_type = None
            self._device_source = "scan"
            self._emit(f"BLE  found {(getattr(device, 'name', '') or '')} @ {device.address}")
            return device

        # 6) 扫描不到：回退到 Windows 配对记录（没有 --name 时只能靠广播）。
        #    典型场景：设备已经和主机配对并作为 HID 连接，不再广播。
        if not self._paired_direct_failed:
            meta = await self._find_paired()
            if meta is not None:
                self._emit(
                    "BLE  not advertising (maybe HID-connected), using paired record ..."
                )
                return self._make_direct_device(meta, "paired")

        # 7) 最后再做一次全量扫描（有些适配器广播包里看不到 128-bit UUID）。
        if self.service_uuid:
            self._emit("BLE  no match in filtered scan, scanning all devices ...")
            found = await self._scan_once(False)
            device = self._pick_device(found)
            if device is not None:
                self._pending_address_type = None
                self._device_source = "scan"
                self._emit(f"BLE  found {(getattr(device, 'name', '') or '')} @ {device.address}")
                return device
        return None

    # -- 连接 -------------------------------------------------------------

    def _on_disconnect(self, _client: Any) -> None:
        self._connected = False
        self._notified = False
        self._disconnected_evt.set()

    def _on_notify(self, _sender: Any, data: bytearray) -> None:
        try:
            self._deliver(bytes(data))
        except Exception as exc:  # 不把异常抛回 bleak 回调
            self._emit(f"BLE  rx callback error: {exc}")

    def _find_characteristic(self, services: Any, uuid: str) -> Any:
        want = _norm_uuid(uuid)
        try:
            char = services.get_characteristic(uuid)
            if char is not None:
                return char
        except Exception:
            pass
        try:
            for service in services:
                for char in getattr(service, "characteristics", []):
                    if _norm_uuid(char.uuid) == want:
                        return char
        except Exception:
            pass
        return None

    async def _get_services(self, client: Any) -> Any:
        services = getattr(client, "services", None)
        if services is not None:
            # 某些版本是 coroutine
            if asyncio.iscoroutine(services):
                services = await services
            if services:
                return services
        get_services = getattr(client, "get_services", None)
        if get_services is not None:
            return await get_services()
        return services

    async def _connect(self, device: Any) -> None:
        winrt_args: Dict[str, Any] = {}
        if self._pending_address_type in ("public", "random"):
            winrt_args["address_type"] = self._pending_address_type
        # 强制非缓存发现：Windows 会按设备缓存 GATT 表，固件改过属性表后
        # 缓存可能与设备实际结构不符（表现：连上了却找不到 DATA_TX/DATA_RX，
        # 而系统 HID 仍工作）。bleak 3.x 支持该开关，老版本忽略。
        winrt_args["use_cached_services"] = False

        async def _open() -> Any:
            try:
                c = BleakClient(
                    device,
                    disconnected_callback=self._on_disconnect,
                    timeout=max(10.0, self.scan_timeout),
                    winrt=winrt_args,
                )
            except TypeError:
                winrt_args.pop("use_cached_services", None)
                c = BleakClient(
                    device,
                    disconnected_callback=self._on_disconnect,
                    timeout=max(10.0, self.scan_timeout),
                    winrt=winrt_args,
                )
            await c.connect()
            return c

        client = await _open()
        try:
            if hasattr(client, "is_connected") and not client.is_connected:
                raise TransportError("连接后状态为未连接")
        except AttributeError:
            pass

        self._client = client
        self._device_address = getattr(device, "address", None)
        services = await self._get_services(client)

        tx_char = self._find_characteristic(services, self.tx_uuid) if services else None
        rx_char = self._find_characteristic(services, self.rx_uuid) if services else None

        if tx_char is None or rx_char is None:
            # 刚断线重连时句柄可能还没稳定：清掉重连一次，仍失败才报错
            try:
                await client.disconnect()
            except Exception:
                pass
            await asyncio.sleep(0.8)
            client = await _open()
            self._client = client
            services = await self._get_services(client)
            tx_char = self._find_characteristic(services, self.tx_uuid) if services else None
            rx_char = self._find_characteristic(services, self.rx_uuid) if services else None

        if tx_char is None or rx_char is None:
            available: List[str] = []
            try:
                for service in services:
                    available.extend(
                        _norm_uuid(c.uuid) for c in getattr(service, "characteristics", [])
                    )
            except Exception:
                pass
            await self._cleanup_client()
            raise TransportError(
                "GATT 中找不到 DATA_TX/DATA_RX。请核对 --service/--tx-char/--rx-char；"
                "若设备改过 GATT 表，Windows 会缓存旧结构，需删除配对记录后重配。"
                f" 当前特征: {available or '(未发现服务)'}"
            )

        # 用特征对象里的真实 UUID（大小写可能与参数不同）
        self.tx_uuid = tx_char.uuid
        self.rx_uuid = rx_char.uuid

        mtu = getattr(client, "mtu_size", None)
        self._mtu = int(mtu) if isinstance(mtu, int) and mtu >= 23 else 23

        await client.start_notify(self.tx_uuid, self._on_notify)
        self._notified = True
        self._connected = True

        # Windows 上 MTU 往往在 GATT 会话/订阅建立后才更新（实测 23 -> 247）。
        for _ in range(6):
            mtu = getattr(client, "mtu_size", None)
            if isinstance(mtu, int) and mtu > 23:
                self._mtu = int(mtu)
                break
            await asyncio.sleep(0.1)
        mtu = getattr(client, "mtu_size", None)
        if isinstance(mtu, int) and mtu >= 23:
            self._mtu = int(mtu)

        # 直连成功：记住它，后续断线重连优先复用地址，不用再扫描。
        self._last_paired_meta = {
            "name": getattr(device, "name", None) or self._device_address or "",
            "address": self._device_address,
            "address_type": self._pending_address_type,
            "connected": True,
            "paired": True,
        }
        self._paired_direct_failed = False
        self._paired_failures = 0

        self._emit(
            f"LINK  connected {self._device_address} mtu={self._mtu} "
            f"max_payload={self.max_payload}B source={self._device_source}"
        )
        if self._mtu <= 23:
            self._emit(
                "BLE  WARN: ATT MTU 仅 23（payload 上限 9 B）。"
                "请检查设备端 MTU 交换或 Windows 蓝牙驱动"
            )

    async def _async_disconnect(self, client: Any) -> None:
        try:
            if self._notified:
                try:
                    await client.stop_notify(self.tx_uuid)
                except Exception:
                    pass
                self._notified = False
            if getattr(client, "is_connected", False):
                await client.disconnect()
        except Exception:
            pass

    async def _cleanup_client(self) -> None:
        client, self._client = self._client, None
        self._connected = False
        self._notified = False
        if client is not None:
            await self._async_disconnect(client)

    # -- 发送 -------------------------------------------------------------

    def write(self, data: bytes) -> int:
        payload = bytes(data)
        client = self._client
        if not self.connected or client is None:
            raise TransportError("BLE 未连接，无法写入")
        if len(payload) > self.mtu - 3:
            raise TransportError(
                f"单包 {len(payload)} B 超过 ATT MTU-3={self.mtu - 3} B；"
                "BLE 链路不分片，请减小 payload"
            )
        loop = self._loop
        if loop is None or not loop.is_running():
            raise TransportError("BLE 事件循环未运行")
        try:
            fut = asyncio.run_coroutine_threadsafe(
                client.write_gatt_char(self.rx_uuid, payload, response=False), loop
            )
            fut.result(timeout=5.0)
        except Exception as exc:
            raise TransportError(f"BLE 写入失败: {exc}") from exc
        return len(payload)


def make_ble_transport(**kwargs: Any) -> BleTransport:
    return BleTransport(**kwargs)
