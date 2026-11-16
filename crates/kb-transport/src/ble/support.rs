//! BLE 传输的辅助类型与转换：sidecar 启动参数、设备列表解析、错误映射、Base64。

use std::path::PathBuf;

use kb_sidecar::SidecarError;
use serde_json::{Map, Value};

use crate::error::TransportError;

/// 枚举到的 BLE 设备（对应 `docs/interfaces.md` §2.2 `BleDevice`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleDeviceInfo {
    pub id: String,
    pub address: String,
    pub name: String,
    pub rssi: Option<i32>,
    pub paired: bool,
    pub address_type: Option<String>,
}

/// sidecar 启动参数（开发态默认 `python sidecars/ble/main.py`）。
#[derive(Debug, Clone)]
pub struct SidecarLaunch {
    pub program: String,
    pub script: String,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Default for SidecarLaunch {
    fn default() -> Self {
        let program = std::env::var("JNWS_PYTHON").unwrap_or_else(|_| "python".to_string());
        let script = std::env::var("JNWS_BLE_SIDECAR").unwrap_or_else(|_| default_script());
        Self {
            program,
            script,
            cwd: None,
            env: Vec::new(),
        }
    }
}

fn default_script() -> String {
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("sidecars")
        .join("ble")
        .join("main.py");
    if candidate.exists() {
        candidate.display().to_string()
    } else {
        "sidecars/ble/main.py".to_string()
    }
}

pub(crate) fn parse_devices(header: &Map<String, Value>) -> Vec<BleDeviceInfo> {
    let Some(items) = header.get("devices").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let address = item.get("address")?.as_str()?.to_string();
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(&address)
                .to_string();
            Some(BleDeviceInfo {
                id,
                address,
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                rssi: item
                    .get("rssi")
                    .and_then(Value::as_i64)
                    .map(|value| value as i32),
                paired: item.get("paired").and_then(Value::as_bool).unwrap_or(false),
                address_type: item
                    .get("address_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

pub(crate) fn map_remote(code: &str, message: &str) -> TransportError {
    match code {
        "E_BLE_DEVICE_NOT_FOUND" => TransportError::DeviceNotFound {
            detail: message.to_string(),
        },
        "E_BLE_CONNECT_TIMEOUT" => TransportError::ConnectTimeout {
            detail: message.to_string(),
        },
        "E_NOT_CONNECTED" => TransportError::NotConnected,
        "E_PAYLOAD_TOO_LARGE" => TransportError::PayloadTooLarge { len: 0, max: 0 },
        "E_WRITE_FAILED" => TransportError::WriteFailed {
            detail: message.to_string(),
        },
        "E_INVALID_ARG" => TransportError::InvalidArg {
            detail: message.to_string(),
        },
        "E_NOT_IMPLEMENTED" => TransportError::NotImplemented {
            what: "BLE sidecar 方法",
        },
        _ => TransportError::Sidecar {
            detail: format!("{code}: {message}"),
        },
    }
}

pub(crate) fn map_sidecar_error(error: SidecarError) -> TransportError {
    match error {
        SidecarError::Remote { code, message } => map_remote(code, &message),
        SidecarError::Timeout { method, timeout_ms } => TransportError::Io {
            detail: format!("sidecar {method} 超时（{timeout_ms} ms）"),
        },
        SidecarError::Protocol { detail } => TransportError::Protocol { detail },
        SidecarError::VersionMismatch { got, want } => TransportError::Protocol {
            detail: format!("sidecar 协议版本 {got} != {want}"),
        },
        SidecarError::RoleMismatch { got, want } => TransportError::Protocol {
            detail: format!("sidecar 角色 {got:?} != {want:?}"),
        },
        other => TransportError::Sidecar {
            detail: other.to_string(),
        },
    }
}

/// 标准 Base64（带 padding），用于 sidecar 的 `data_b64`。
pub(crate) fn encode_base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let value = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((value >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((value >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((value >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(value & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode_base64;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode_base64(&[0x00, 0xFF, 0x10]), "AP8Q");
    }
}
