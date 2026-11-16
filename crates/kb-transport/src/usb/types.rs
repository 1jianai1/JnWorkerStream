//! USB CDC 端口描述与选择条件。

/// 打开 USB CDC 的条件（对应 `UsbCfg`）；`port` 为空时按其余字段过滤。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbCfg {
    pub port: Option<String>,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub description: Option<String>,
}

impl UsbCfg {
    /// 是否设置了 `port` 之外的过滤条件。
    pub fn has_filter(&self) -> bool {
        self.vid.is_some()
            || self.pid.is_some()
            || self.serial_number.is_some()
            || self
                .description
                .as_deref()
                .is_some_and(|text| !text.is_empty())
    }

    /// 端口是否匹配（不含 `port` 精确匹配）。
    pub fn matches(&self, info: &UsbPortInfo) -> bool {
        if let Some(vid) = self.vid {
            if info.vid != Some(vid) {
                return false;
            }
        }
        if let Some(pid) = self.pid {
            if info.pid != Some(pid) {
                return false;
            }
        }
        if let Some(serial) = &self.serial_number {
            if info.serial_number.as_deref() != Some(serial.as_str()) {
                return false;
            }
        }
        if let Some(needle) = &self.description {
            let needle = needle.to_lowercase();
            if !needle.is_empty() {
                let haystack = format!(
                    "{} {} {}",
                    info.description.to_lowercase(),
                    info.device.to_lowercase(),
                    info.pretty().to_lowercase()
                );
                if !haystack.contains(&needle) {
                    return false;
                }
            }
        }
        true
    }

    /// 用于错误消息的过滤条件描述。
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(port) = &self.port {
            parts.push(format!("port={port}"));
        }
        if let Some(vid) = self.vid {
            parts.push(format!("vid=0x{vid:04X}"));
        }
        if let Some(pid) = self.pid {
            parts.push(format!("pid=0x{pid:04X}"));
        }
        if let Some(serial) = &self.serial_number {
            parts.push(format!("sn={serial}"));
        }
        if let Some(description) = &self.description {
            parts.push(format!("desc={description}"));
        }
        if parts.is_empty() {
            "（无过滤条件）".to_string()
        } else {
            parts.join(" ")
        }
    }
}

/// 枚举到的 USB CDC / 串口端口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbPortInfo {
    pub device: String,
    pub description: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
}

impl UsbPortInfo {
    /// 日志/错误消息里的单行描述。
    pub fn pretty(&self) -> String {
        let mut text = self.device.clone();
        if let (Some(vid), Some(pid)) = (self.vid, self.pid) {
            text.push_str(&format!(" [{vid:04X}:{pid:04X}]"));
        }
        if !self.description.is_empty() {
            text.push(' ');
            text.push_str(&self.description);
        }
        if let Some(serial) = &self.serial_number {
            text.push_str(&format!(" sn={serial}"));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{UsbCfg, UsbPortInfo};

    fn port() -> UsbPortInfo {
        UsbPortInfo {
            device: "COM3".to_string(),
            description: "USB Serial Device".to_string(),
            vid: Some(0x303A),
            pid: Some(0x1001),
            serial_number: Some("ABC123".to_string()),
        }
    }

    #[test]
    fn matches_by_vid_pid_and_description() {
        let info = port();
        assert!(UsbCfg::default().matches(&info));
        assert!(UsbCfg {
            vid: Some(0x303A),
            pid: Some(0x1001),
            ..Default::default()
        }
        .matches(&info));
        assert!(UsbCfg {
            description: Some("serial".to_string()),
            ..Default::default()
        }
        .matches(&info));
        assert!(!UsbCfg {
            vid: Some(0x1234),
            ..Default::default()
        }
        .matches(&info));
        assert!(!UsbCfg {
            serial_number: Some("ZZZ".to_string()),
            ..Default::default()
        }
        .matches(&info));
    }

    #[test]
    fn pretty_includes_ids_and_description() {
        assert_eq!(
            port().pretty(),
            "COM3 [303A:1001] USB Serial Device sn=ABC123"
        );
    }
}
