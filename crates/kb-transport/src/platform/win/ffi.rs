//! 最小 Win32 FFI 绑定（kernel32 / setupapi / advapi32）。
//!
//! 只声明本项目用到的函数与结构；类型宽度与 `windows.h` 对齐（x86/x64）。

#![allow(non_camel_case_types, non_snake_case, dead_code)]
#![allow(clippy::upper_case_acronyms)]

use core::ffi::c_void;

pub type HANDLE = isize;
pub type BOOL = i32;
pub type DWORD = u32;
pub type LSTATUS = i32;

pub const INVALID_HANDLE_VALUE: HANDLE = -1;

pub const GENERIC_READ: DWORD = 0x8000_0000;
pub const GENERIC_WRITE: DWORD = 0x4000_0000;
pub const OPEN_EXISTING: DWORD = 3;
pub const FILE_ATTRIBUTE_NORMAL: DWORD = 0x0000_0080;

pub const MAXDWORD: DWORD = 0xFFFF_FFFF;

pub const PURGE_TXCLEAR: DWORD = 0x0004;
pub const PURGE_RXCLEAR: DWORD = 0x0008;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_SHARING_VIOLATION: u32 = 32;
pub const ERROR_OPERATION_ABORTED: u32 = 995;
pub const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
pub const ERROR_DEVICE_REMOVED: u32 = 1617;
pub const ERROR_NO_MORE_ITEMS: u32 = 259;

pub const DIGCF_PRESENT: DWORD = 0x0000_0002;
pub const DICS_FLAG_GLOBAL: DWORD = 0x0000_0001;
pub const DIREG_DEV: DWORD = 0x0000_0001;
pub const KEY_READ: DWORD = 0x0002_0019;

pub const SPDRP_DEVICEDESC: DWORD = 0x0000_0000;
pub const SPDRP_HARDWAREID: DWORD = 0x0000_0001;
pub const SPDRP_FRIENDLYNAME: DWORD = 0x0000_000C;

pub const REG_SZ: DWORD = 1;
pub const REG_MULTI_SZ: DWORD = 7;

/// `GUID`（16 B）。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }
}

/// `GUID_DEVCLASS_PORTS`（Ports (COM & LPT) 设备类）。
pub const GUID_DEVCLASS_PORTS: GUID = GUID::new(
    0x4D36_E978,
    0xE325,
    0x11CE,
    [0xBF, 0xC1, 0x08, 0x00, 0x2B, 0xE1, 0x03, 0x18],
);

/// `DCB`（28 B）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Dcb {
    pub length: DWORD,
    pub baud_rate: DWORD,
    pub flags: DWORD,
    pub w_reserved: u16,
    pub xon_lim: u16,
    pub xoff_lim: u16,
    pub byte_size: u8,
    pub parity: u8,
    pub stop_bits: u8,
    pub xon_char: u8,
    pub xoff_char: u8,
    pub error_char: u8,
    pub eof_char: u8,
    pub evt_char: u8,
    pub w_reserved1: u16,
}

impl Default for Dcb {
    fn default() -> Self {
        Self {
            length: std::mem::size_of::<Self>() as DWORD,
            baud_rate: 115_200,
            // fBinary=1, fDtrControl=1(ENABLE), fTXContinueOnXoff=1, fRtsControl=1(ENABLE)
            flags: 0x0000_1091,
            w_reserved: 0,
            xon_lim: 2048,
            xoff_lim: 512,
            byte_size: 8,
            parity: 0,
            stop_bits: 0,
            xon_char: 0x11,
            xoff_char: 0x13,
            error_char: 0,
            eof_char: 0,
            evt_char: 0,
            w_reserved1: 0,
        }
    }
}

/// `COMMTIMEOUTS`（20 B）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CommTimeouts {
    pub read_interval_timeout: DWORD,
    pub read_total_multiplier: DWORD,
    pub read_total_constant: DWORD,
    pub write_total_multiplier: DWORD,
    pub write_total_constant: DWORD,
}

/// `SP_DEVINFO_DATA`。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SpDevinfoData {
    pub cb_size: DWORD,
    pub class_guid: GUID,
    pub dev_inst: DWORD,
    pub reserved: usize,
}

extern "system" {
    pub fn CreateFileW(
        file_name: *const u16,
        desired_access: DWORD,
        share_mode: DWORD,
        security_attributes: *const c_void,
        creation_disposition: DWORD,
        flags_and_attributes: DWORD,
        template_file: HANDLE,
    ) -> HANDLE;
    pub fn ReadFile(
        file: HANDLE,
        buffer: *mut u8,
        bytes_to_read: DWORD,
        bytes_read: *mut DWORD,
        overlapped: *mut c_void,
    ) -> BOOL;
    pub fn WriteFile(
        file: HANDLE,
        buffer: *const u8,
        bytes_to_write: DWORD,
        bytes_written: *mut DWORD,
        overlapped: *mut c_void,
    ) -> BOOL;
    pub fn FlushFileBuffers(file: HANDLE) -> BOOL;
    pub fn CloseHandle(object: HANDLE) -> BOOL;
    pub fn GetLastError() -> u32;
    pub fn GetCommState(file: HANDLE, dcb: *mut Dcb) -> BOOL;
    pub fn SetCommState(file: HANDLE, dcb: *const Dcb) -> BOOL;
    pub fn SetCommTimeouts(file: HANDLE, timeouts: *const CommTimeouts) -> BOOL;
    pub fn PurgeComm(file: HANDLE, flags: DWORD) -> BOOL;
    pub fn CancelIoEx(file: HANDLE, overlapped: *const c_void) -> BOOL;
}

#[link(name = "setupapi")]
extern "system" {
    pub fn SetupDiGetClassDevsW(
        class_guid: *const GUID,
        enumerator: *const u16,
        parent: isize,
        flags: DWORD,
    ) -> HANDLE;
    pub fn SetupDiEnumDeviceInfo(
        device_info_set: HANDLE,
        member_index: DWORD,
        device_info_data: *mut SpDevinfoData,
    ) -> BOOL;
    pub fn SetupDiDestroyDeviceInfoList(device_info_set: HANDLE) -> BOOL;
    pub fn SetupDiOpenDevRegKey(
        device_info_set: HANDLE,
        device_info_data: *const SpDevinfoData,
        scope: DWORD,
        hw_profile: DWORD,
        key_type: DWORD,
        sam_desired: DWORD,
    ) -> HANDLE;
    pub fn SetupDiGetDeviceRegistryPropertyW(
        device_info_set: HANDLE,
        device_info_data: *const SpDevinfoData,
        property: DWORD,
        property_reg_data_type: *mut DWORD,
        property_buffer: *mut u8,
        property_buffer_size: DWORD,
        required_size: *mut DWORD,
    ) -> BOOL;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn RegQueryValueExW(
        key: HANDLE,
        value_name: *const u16,
        reserved: *mut DWORD,
        value_type: *mut DWORD,
        data: *mut u8,
        data_size: *mut DWORD,
    ) -> LSTATUS;
    pub fn RegCloseKey(key: HANDLE) -> LSTATUS;
}

/// UTF-16 + NUL 缓冲区。
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Win32 错误码转可读文本。
pub fn last_error_message(code: u32) -> String {
    std::io::Error::from_raw_os_error(code as i32).to_string()
}
