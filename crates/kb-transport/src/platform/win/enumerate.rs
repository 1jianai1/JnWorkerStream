//! 端口枚举（SetupAPI）：`PortName` + 友好名 + `VID_/PID_`。

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use super::ffi;
use crate::error::TransportError;
use crate::usb::UsbPortInfo;

/// 枚举当前系统全部串口，按 `device` 排序。
pub fn list_ports() -> Result<Vec<UsbPortInfo>, TransportError> {
    let set = unsafe {
        ffi::SetupDiGetClassDevsW(&ffi::GUID_DEVCLASS_PORTS, null(), 0, ffi::DIGCF_PRESENT)
    };
    if set == ffi::INVALID_HANDLE_VALUE {
        let code = unsafe { ffi::GetLastError() };
        return Err(TransportError::io(format!(
            "枚举串口失败（SetupDiGetClassDevs {code}: {}）",
            ffi::last_error_message(code)
        )));
    }

    let mut ports = Vec::new();
    let mut index = 0u32;
    loop {
        let mut info: ffi::SpDevinfoData = unsafe { zeroed() };
        info.cb_size = size_of::<ffi::SpDevinfoData>() as u32;
        let ok = unsafe { ffi::SetupDiEnumDeviceInfo(set, index, &mut info) };
        if ok == 0 {
            break;
        }
        if let Some(port) = read_port(set, &info) {
            ports.push(port);
        }
        index += 1;
        if index > 1_024 {
            break;
        }
    }
    unsafe {
        ffi::SetupDiDestroyDeviceInfoList(set);
    }

    ports.sort_by(|left, right| left.device.cmp(&right.device));
    Ok(ports)
}

fn read_port(set: isize, info: &ffi::SpDevinfoData) -> Option<UsbPortInfo> {
    let key = unsafe {
        ffi::SetupDiOpenDevRegKey(
            set,
            info,
            ffi::DICS_FLAG_GLOBAL,
            0,
            ffi::DIREG_DEV,
            ffi::KEY_READ,
        )
    };
    if key == ffi::INVALID_HANDLE_VALUE {
        return None;
    }
    let device = query_string_value(key, "PortName");
    unsafe {
        ffi::RegCloseKey(key);
    }
    let device = device?;
    if device.trim().is_empty() {
        return None;
    }

    let description = property_text(set, info, ffi::SPDRP_FRIENDLYNAME)
        .or_else(|| property_text(set, info, ffi::SPDRP_DEVICEDESC))
        .unwrap_or_default();
    let hardware_ids = property_multi_text(set, info, ffi::SPDRP_HARDWAREID).unwrap_or_default();
    let vid = parse_id(&hardware_ids, "VID_");
    let pid = parse_id(&hardware_ids, "PID_");

    Some(UsbPortInfo {
        device,
        description,
        vid,
        pid,
        serial_number: None,
    })
}

fn query_string_value(key: isize, name: &str) -> Option<String> {
    let name = ffi::wide(name);
    let mut value_type: u32 = 0;
    let mut size: u32 = 1_024;
    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        ffi::RegQueryValueExW(
            key,
            name.as_ptr(),
            null_mut(),
            &mut value_type,
            buffer.as_mut_ptr(),
            &mut size,
        )
    };
    if status != 0 || value_type != ffi::REG_SZ {
        return None;
    }
    buffer.truncate(size as usize);
    Some(decode_utf16(&buffer, false))
}

fn property_raw(set: isize, info: &ffi::SpDevinfoData, property: u32) -> Option<(u32, Vec<u8>)> {
    let mut value_type: u32 = 0;
    let mut required: u32 = 0;
    let mut buffer = vec![0u8; 2_048];
    let ok = unsafe {
        ffi::SetupDiGetDeviceRegistryPropertyW(
            set,
            info,
            property,
            &mut value_type,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut required,
        )
    };
    if ok == 0 {
        return None;
    }
    if required > 0 && (required as usize) <= buffer.len() {
        buffer.truncate(required as usize);
    }
    Some((value_type, buffer))
}

fn property_text(set: isize, info: &ffi::SpDevinfoData, property: u32) -> Option<String> {
    let (value_type, buffer) = property_raw(set, info, property)?;
    if value_type != ffi::REG_SZ {
        return None;
    }
    let text = decode_utf16(&buffer, false);
    (!text.trim().is_empty()).then_some(text)
}

fn property_multi_text(set: isize, info: &ffi::SpDevinfoData, property: u32) -> Option<String> {
    let (value_type, buffer) = property_raw(set, info, property)?;
    if value_type != ffi::REG_MULTI_SZ {
        return None;
    }
    Some(decode_utf16(&buffer, true))
}

fn decode_utf16(bytes: &[u8], multi: bool) -> String {
    let mut units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect();
    if multi {
        units
            .split(|value| *value == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect::<Vec<_>>()
            .join(";")
    } else {
        if let Some(end) = units.iter().position(|value| *value == 0) {
            units.truncate(end);
        }
        String::from_utf16_lossy(&units)
    }
}

fn parse_id(text: &str, prefix: &str) -> Option<u16> {
    let upper = text.to_ascii_uppercase();
    let mut from = 0usize;
    while let Some(found) = upper[from..].find(prefix) {
        let start = from + found + prefix.len();
        let candidate: String = upper[start..]
            .chars()
            .take_while(|value| value.is_ascii_hexdigit())
            .take(4)
            .collect();
        if candidate.len() == 4 {
            if let Ok(value) = u16::from_str_radix(&candidate, 16) {
                return Some(value);
            }
        }
        from = start;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_id;

    #[test]
    fn parses_usb_vid_pid() {
        let hardware = "USB\\VID_303A&PID_1001&MI_00;USB\\VID_303A&PID_1001";
        assert_eq!(parse_id(hardware, "VID_"), Some(0x303A));
        assert_eq!(parse_id(hardware, "PID_"), Some(0x1001));
        assert_eq!(parse_id("USB\\VID_123", "VID_"), None);
        assert_eq!(parse_id("", "VID_"), None);
    }
}
