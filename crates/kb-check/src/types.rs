//! 校验统计与单帧结果（供 `Checker` / `Recorder` / 命令层复用）。

/// 校验统计（`docs/interfaces.md` §2.2 `check`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckStats {
    pub recv: u64,
    pub lost: u64,
    pub dup: u64,
    pub order: u64,
    pub pattern_err: u64,
    pub length_err: u64,
    pub length_changes: u64,
    pub malformed: u64,
    pub echo_frames: u64,
}

/// 单帧校验结果。
#[derive(Debug, Clone)]
pub struct PayloadInfo {
    pub seq: u16,
    pub dev_ms: u32,
    pub flags: u8,
    pub data: Vec<u8>,
    pub payload_len: usize,
    pub data_len: usize,
    pub sha256: String,
    pub is_echo: bool,
    pub pattern: &'static str,
    pub pattern_ok: bool,
    pub length_ok: bool,
}
