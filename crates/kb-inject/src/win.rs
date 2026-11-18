//! Windows 注入实现：`SendInput`（KEYEVENTF_UNICODE）与剪贴板兜底。
//!
//! 只声明本项目用到的 user32/kernel32 函数；结构体宽度按 x64/x86 对齐
//! （`INPUT` = 4 B 类型 + 4 B 对齐 + 32 B union = 40 B）。

#![allow(non_camel_case_types, non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use std::thread::sleep;
use std::time::Duration;

use crate::{InjectOutcome, InjectPlan, ProbeMode, ProbeReport};

type BOOL = i32;
type UINT = u32;
type DWORD = u32;
type WORD = u16;
type HWND = isize;
type HGLOBAL = isize;
type HANDLE = isize;
type LPVOID = *mut u8;

const INPUT_KEYBOARD: DWORD = 1;
const KEYEVENTF_KEYUP: DWORD = 0x0002;
const KEYEVENTF_UNICODE: DWORD = 0x0004;
const VK_CONTROL: WORD = 0x11;
const VK_V: WORD = 0x56;
const VK_RETURN: WORD = 0x0D;
const CF_UNICODETEXT: UINT = 13;
const GMEM_MOVEABLE: UINT = 0x0002;
const SW_RESTORE: i32 = 9;
const PM_NOREMOVE: UINT = 0x0000;
const SMTO_ABORTIFHUNG: UINT = 0x0002;
const PROCESS_QUERY_LIMITED_INFORMATION: DWORD = 0x1000;
const TOKEN_QUERY: DWORD = 0x0008;
const TOKEN_INTEGRITY_LEVEL: DWORD = 25;

#[repr(C)]
#[derive(Clone, Copy)]
struct KEYBDINPUT {
    w_vk: WORD,
    w_scan: WORD,
    dw_flags: DWORD,
    time: DWORD,
    dw_extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MOUSEINPUT {
    dx: i32,
    dy: i32,
    mouse_data: DWORD,
    dw_flags: DWORD,
    time: DWORD,
    dw_extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HARDWAREINPUT {
    u_msg: DWORD,
    w_param_l: WORD,
    w_param_h: WORD,
}

#[repr(C)]
#[derive(Clone, Copy)]
union INPUT_UNION {
    ki: KEYBDINPUT,
    mi: MOUSEINPUT,
    hi: HARDWAREINPUT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct INPUT {
    kind: DWORD,
    data: INPUT_UNION,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GUITHREADINFO {
    cb_size: DWORD,
    flags: DWORD,
    hwnd_active: HWND,
    hwnd_focus: HWND,
    hwnd_capture: HWND,
    hwnd_menu_owner: HWND,
    hwnd_move_size: HWND,
    hwnd_caret: HWND,
    rc_caret: RECT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: *mut u8,
    attributes: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TOKEN_MANDATORY_LABEL {
    label: SID_AND_ATTRIBUTES,
}

const WM_GETTEXT: UINT = 0x000D;
const WM_GETTEXTLENGTH: UINT = 0x000E;
const WM_CHAR: UINT = 0x0102;

// `INPUT`/`KEYBDINPUT` 的宽度必须与 Windows SDK 一致，否则 SendInput 会静默失败或写坏栈。
const _: () = assert!(
    std::mem::size_of::<INPUT>()
        == if cfg!(target_pointer_width = "64") {
            40
        } else {
            28
        },
    "INPUT 大小不符合 Win32 ABI"
);
const _: () = assert!(
    std::mem::size_of::<KEYBDINPUT>()
        == if cfg!(target_pointer_width = "64") {
            24
        } else {
            16
        },
    "KEYBDINPUT 大小不符合 Win32 ABI"
);
// GUITHREADINFO：x64 为 72 B（4+4+6×8+16），x86 为 64 B。
const _: () = assert!(
    std::mem::size_of::<GUITHREADINFO>()
        == if cfg!(target_pointer_width = "64") {
            72
        } else {
            64
        },
    "GUITHREADINFO 大小不符合 Win32 ABI"
);

#[link(name = "user32")]
extern "system" {
    fn SendInput(inputs: UINT, out: *const INPUT, size: i32) -> UINT;
    fn GetForegroundWindow() -> HWND;
    fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    fn BringWindowToTop(hwnd: HWND) -> BOOL;
    fn AttachThreadInput(attach: DWORD, attach_to: DWORD, attach_flag: BOOL) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, process_id: *mut DWORD) -> DWORD;
    fn ShowWindow(hwnd: HWND, command: i32) -> BOOL;
    fn IsIconic(hwnd: HWND) -> BOOL;
    fn GetWindowTextW(hwnd: HWND, text: *mut u16, max_count: i32) -> i32;
    fn GetWindowTextLengthW(hwnd: HWND) -> i32;
    fn GetClassNameW(hwnd: HWND, text: *mut u16, max_count: i32) -> i32;
    fn GetGUIThreadInfo(thread_id: DWORD, info: *mut GUITHREADINFO) -> BOOL;
    fn SendMessageTimeoutW(
        hwnd: HWND,
        msg: UINT,
        w_param: usize,
        l_param: isize,
        flags: UINT,
        timeout: UINT,
        result: *mut usize,
    ) -> isize;
    fn PeekMessageW(msg: *mut u8, hwnd: HWND, min: UINT, max: UINT, remove: UINT) -> BOOL;
    fn OpenClipboard(owner: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn IsClipboardFormatAvailable(format: UINT) -> BOOL;
    fn GetClipboardData(format: UINT) -> HGLOBAL;
    fn SetClipboardData(format: UINT, mem: HGLOBAL) -> HGLOBAL;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: HANDLE, access: DWORD, token: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(
        token: HANDLE,
        class: DWORD,
        info: *mut u8,
        len: DWORD,
        returned: *mut DWORD,
    ) -> BOOL;
    fn GetSidSubAuthorityCount(sid: *mut u8) -> *mut u8;
    fn GetSidSubAuthority(sid: *mut u8, index: DWORD) -> *mut DWORD;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
    fn GetCurrentProcessId() -> DWORD;
    fn GetCurrentThreadId() -> DWORD;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcess(access: DWORD, inherit: BOOL, pid: DWORD) -> HANDLE;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GlobalAlloc(flags: UINT, bytes: usize) -> HGLOBAL;
    fn GlobalFree(mem: HGLOBAL) -> HGLOBAL;
    fn GlobalLock(mem: HGLOBAL) -> LPVOID;
    fn GlobalUnlock(mem: HGLOBAL) -> BOOL;
}

fn key_input(scan: WORD, flags: DWORD) -> INPUT {
    INPUT {
        kind: INPUT_KEYBOARD,
        data: INPUT_UNION {
            ki: KEYBDINPUT {
                w_vk: 0,
                w_scan: scan,
                dw_flags: flags,
                time: 0,
                dw_extra_info: 0,
            },
        },
    }
}

fn vk_input(vk: WORD, up: bool) -> INPUT {
    INPUT {
        kind: INPUT_KEYBOARD,
        data: INPUT_UNION {
            ki: KEYBDINPUT {
                w_vk: vk,
                w_scan: 0,
                dw_flags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dw_extra_info: 0,
            },
        },
    }
}

fn send_inputs(inputs: &[INPUT]) -> bool {
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as UINT,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    sent as usize == inputs.len()
}

fn send_units(units: &[u16]) -> bool {
    let mut inputs = Vec::with_capacity(units.len() * 2);
    for unit in units {
        inputs.push(key_input(*unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(*unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send_inputs(&inputs)
}

fn send_key(vk: WORD, up: bool) -> bool {
    send_inputs(&[vk_input(vk, up)])
}

fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

/// 供 `Injector::begin_segment` 在 PTT 按下时捕获目标窗口。
pub(crate) fn foreground_window() -> Option<HWND> {
    let hwnd = foreground();
    if hwnd == 0 { None } else { Some(hwnd) }
}

/// 当前前台窗口是否属于本进程（也就是上位机自己）。
fn is_own_window(hwnd: HWND) -> bool {
    if hwnd == 0 {
        return false;
    }
    let mut pid: DWORD = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid != 0 && pid == GetCurrentProcessId()
    }
}

fn window_title(hwnd: HWND) -> String {
    if hwnd == 0 {
        return "无".to_string();
    }
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return "（无标题）".to_string();
    }
    let mut buffer = vec![0u16; len as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if written <= 0 {
        return "（无标题）".to_string();
    }
    String::from_utf16_lossy(&buffer[..written as usize])
}

fn window_class(hwnd: HWND) -> String {
    if hwnd == 0 {
        return "无".to_string();
    }
    let mut buffer = [0u16; 256];
    let written = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if written <= 0 {
        return "（无类名）".to_string();
    }
    String::from_utf16_lossy(&buffer[..written as usize])
}

fn process_id(hwnd: HWND) -> u32 {
    let mut pid: DWORD = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    pid
}

/// 进程完整性级别 RID：0x1000 Low / 0x2000 Medium / 0x3000 High / 0x4000 System。
fn integrity_level(pid: DWORD) -> Option<u32> {
    let current = unsafe { GetCurrentProcessId() };
    let process = if pid == current {
        unsafe { GetCurrentProcess() }
    } else {
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }
    };
    if process == 0 {
        return None;
    }

    let mut token: HANDLE = 0;
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0;
    if pid != current {
        unsafe {
            CloseHandle(process);
        }
    }
    if !opened {
        return None;
    }

    // 8 字节对齐的缓冲区，足够容纳 TOKEN_MANDATORY_LABEL + SID。
    let mut buffer = [0u64; 64];
    let mut returned: DWORD = 0;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TOKEN_INTEGRITY_LEVEL,
            buffer.as_mut_ptr() as *mut u8,
            std::mem::size_of_val(&buffer) as DWORD,
            &mut returned,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    if ok == 0 {
        return None;
    }

    let label = unsafe { &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL) };
    let sid = label.label.sid;
    if sid.is_null() {
        return None;
    }
    let count_ptr = unsafe { GetSidSubAuthorityCount(sid) };
    if count_ptr.is_null() {
        return None;
    }
    let count = unsafe { *count_ptr };
    if count == 0 {
        return None;
    }
    let rid_ptr = unsafe { GetSidSubAuthority(sid, DWORD::from(count - 1)) };
    if rid_ptr.is_null() {
        return None;
    }
    Some(unsafe { *rid_ptr })
}

fn integrity_name(rid: Option<u32>) -> &'static str {
    match rid {
        Some(0x0000) => "Untrusted",
        Some(0x1000) => "Low",
        Some(0x2000) => "Medium",
        Some(0x3000) => "High",
        Some(0x4000) => "System",
        Some(_) => "Other",
        None => "Unknown",
    }
}

/// 当前上位机进程的完整性级别（启动时打印，便于判断 UIPI）。
pub fn current_integrity() -> Option<u32> {
    integrity_level(unsafe { GetCurrentProcessId() })
}

/// 完整性级别的人名（`Low` / `Medium` / `High` / `System`）。
pub fn integrity_label(rid: Option<u32>) -> &'static str {
    integrity_name(rid)
}

fn focused_control(hwnd: HWND) -> HWND {
    if hwnd == 0 {
        return 0;
    }
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) };
    if thread_id == 0 {
        return 0;
    }
    let mut info = GUITHREADINFO {
        cb_size: std::mem::size_of::<GUITHREADINFO>() as DWORD,
        ..GUITHREADINFO::default()
    };
    let ok = unsafe { GetGUIThreadInfo(thread_id, &mut info) };
    if ok == 0 {
        0
    } else {
        info.hwnd_focus
    }
}

/// 读取任意进程控件的文本（`GetWindowText` 跨进程拿不到，改用 `WM_GETTEXT`）。
fn control_text(hwnd: HWND) -> Option<String> {
    if hwnd == 0 {
        return None;
    }
    let mut result: usize = 0;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXTLENGTH,
            0,
            0,
            SMTO_ABORTIFHUNG,
            250,
            &mut result,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = result;
    if len == 0 {
        return Some(String::new());
    }
    let capacity = len.min(20_000) + 1;
    let mut buffer = vec![0u16; capacity];
    let mut copied: usize = 0;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT,
            capacity,
            buffer.as_mut_ptr() as isize,
            SMTO_ABORTIFHUNG,
            250,
            &mut copied,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(
        &buffer[..copied.min(capacity)],
    ))
}

fn unicode_inputs(text: &str) -> Vec<INPUT> {
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(key_input(unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    inputs
}

fn send_ctrl_v() -> (usize, usize, u32) {
    let inputs = [
        vk_input(VK_CONTROL, false),
        vk_input(VK_V, false),
        vk_input(VK_V, true),
        vk_input(VK_CONTROL, true),
    ];
    let requested = inputs.len();
    let sent = unsafe { SendInput(requested as UINT, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    let error = if sent as usize == requested { 0 } else { last_error() };
    (requested, sent as usize, error)
}

/// Phase 0 探针：不经过 Tauri/Core，直接验证 SendInput / Clipboard 是否真落字。
///
/// 调用前由示例程序负责等待用户把焦点放到目标窗口。
pub fn probe(
    text: &str,
    mode: ProbeMode,
    restore_clipboard: bool,
) -> ProbeReport {
    let target = foreground();
    let target_title = window_title(target);
    let target_class = window_class(target);
    let target_pid = process_id(target);
    let current_pid = unsafe { GetCurrentProcessId() };
    let own_process = target != 0 && target_pid == current_pid;
    let current_integrity = integrity_level(current_pid);
    let target_integrity = integrity_level(target_pid);
    let focus_before = focused_control(target);
    let text_before = control_text(focus_before);
    let foreground_before = target;

    println!(
        "[probe] target hwnd=0x{:X} pid={} class={:?} title={:?}",
        target as usize, target_pid, target_class, target_title
    );
    println!(
        "[probe] integrity current={}(0x{:X}) target={}(0x{:X})",
        integrity_name(current_integrity),
        current_integrity.unwrap_or(0),
        integrity_name(target_integrity),
        target_integrity.unwrap_or(0),
    );
    println!(
        "[probe] foreground_before=0x{:X} focus_before=0x{:X} class={:?} text_len={:?}",
        foreground_before as usize,
        focus_before as usize,
        window_class(focus_before),
        text_before.as_ref().map(String::len)
    );

    if own_process {
        println!("[probe] REFUSE: 目标是上位机自身进程，请把焦点放到记事本/浏览器/微信输入框");
        return ProbeReport {
            mode: mode_name(mode),
            ok: false,
            target_hwnd: target,
            target_title,
            target_class,
            target_pid,
            own_process,
            current_integrity,
            target_integrity,
            foreground_before,
            foreground_after: foreground(),
            focus_before,
            focus_after: 0,
            requested_events: 0,
            sent_events: 0,
            last_error: 0,
            elapsed_ms: 0,
            text_before_len: text_before.map(|value| value.len()),
            text_after_len: None,
            verified_inserted: None,
            clipboard_before: None,
            clipboard_restored: false,
            note: Some("own_process".to_string()),
        };
    }

    let started = std::time::Instant::now();
    let (requested_events, sent_events, last_error, clipboard_before, clipboard_restored) = match mode
    {
        ProbeMode::SendInput => {
            let inputs = unicode_inputs(text);
            let requested = inputs.len();
            let sent = unsafe {
                SendInput(
                    requested as UINT,
                    inputs.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                )
            };
            let error = if sent as usize == requested { 0 } else { last_error() };
            println!(
                "[probe] SendInput requested={} sent={} last_error={}",
                requested, sent, error
            );
            (requested, sent as usize, error, None, false)
        }
        ProbeMode::Clipboard => {
            let before = clipboard_text();
            let set_ok = set_clipboard_text(text);
            println!(
                "[probe] clipboard_before={:?} set_ok={}",
                before.as_deref().map(truncate),
                set_ok
            );
            if !set_ok {
                (0, 0, 0, before, false)
            } else {
                let (requested, sent, error) = send_ctrl_v();
                println!(
                    "[probe] Ctrl+V requested={} sent={} last_error={}",
                    requested, sent, error
                );
                sleep(Duration::from_millis(600));
                let mut restored = false;
                if restore_clipboard {
                    if let Some(previous) = &before {
                        restored = set_clipboard_text(previous);
                    }
                }
                (requested, sent, error, before, restored)
            }
        }
        ProbeMode::WmChar => {
            let focus = focused_control(target);
            let mut sent = 0usize;
            let mut requested = 0usize;
            let mut error = 0u32;
            if focus == 0 {
                error = 1;
            } else {
                for unit in text.encode_utf16() {
                    requested += 1;
                    let mut result: usize = 0;
                    let ok = unsafe {
                        SendMessageTimeoutW(
                            focus,
                            WM_CHAR,
                            unit as usize,
                            1,
                            SMTO_ABORTIFHUNG,
                            250,
                            &mut result,
                        )
                    };
                    if ok != 0 {
                        sent += 1;
                    } else {
                        error = last_error();
                        break;
                    }
                }
            }
            println!(
                "[probe] WM_CHAR focus=0x{:X} requested={} sent={} last_error={}",
                focus as usize, requested, sent, error
            );
            (requested, sent, error, None, false)
        }
        ProbeMode::Inspect => {
            println!("[probe] inspect: 不注入，仅检查目标/焦点窗口");
            (0, 0, 0, None, false)
        }
    };
    let elapsed_ms = started.elapsed().as_millis();

    sleep(Duration::from_millis(120));
    let foreground_after = foreground();
    let focus_after = focused_control(foreground_after);
    let text_after = control_text(focus_after);
    let verified_inserted = if mode == ProbeMode::Inspect {
        None
    } else {
        match (&text_before, &text_after) {
            (Some(before), Some(after)) => Some(after.contains(text) && !before.contains(text)),
            _ => None,
        }
    };
    println!(
        "[probe] foreground_after=0x{:X} focus_after=0x{:X} class={:?} text_len={:?}",
        foreground_after as usize,
        focus_after as usize,
        window_class(focus_after),
        text_after.as_ref().map(String::len)
    );
    println!(
        "[probe] elapsed={}ms verified_inserted={:?}",
        elapsed_ms, verified_inserted
    );

    let focus_ok = foreground_after == target;
    let (ok, note) = if mode == ProbeMode::Inspect {
        let ok = target != 0 && !own_process && focus_ok;
        let note = if !ok {
            Some("inspect_target_invalid_or_focus_changed".to_string())
        } else {
            None
        };
        (ok, note)
    } else {
        let sent_ok = requested_events > 0
            && sent_events == requested_events
            && last_error == 0;
        let ok = sent_ok && focus_ok && verified_inserted != Some(false);
        let note = if !focus_ok {
            Some("focus_changed".to_string())
        } else if !sent_ok {
            Some("sendinput_or_clipboard_failed".to_string())
        } else if verified_inserted == Some(false) {
            Some("sent_but_not_verified".to_string())
        } else {
            None
        };
        (ok, note)
    };

    ProbeReport {
        mode: mode_name(mode),
        ok,
        target_hwnd: target,
        target_title,
        target_class,
        target_pid,
        own_process,
        current_integrity,
        target_integrity,
        foreground_before,
        foreground_after,
        focus_before,
        focus_after,
        requested_events,
        sent_events,
        last_error,
        elapsed_ms,
        text_before_len: text_before.map(|value| value.len()),
        text_after_len: text_after.map(|value| value.len()),
        verified_inserted,
        clipboard_before,
        clipboard_restored,
        note,
    }
}

fn mode_name(mode: ProbeMode) -> &'static str {
    match mode {
        ProbeMode::SendInput => "sendinput",
        ProbeMode::Clipboard => "clipboard",
        ProbeMode::WmChar => "wmchar",
        ProbeMode::Inspect => "inspect",
    }
}

fn truncate(value: &str) -> String {
    let mut out: String = value.chars().take(40).collect();
    if value.chars().count() > 40 {
        out.push('…');
    }
    out
}

/// 把目标窗口切到前台。
///
/// 单纯 `SetForegroundWindow` 在“上位机自己当前是前台”时经常被 Windows 的
/// 前台锁拒绝；这里先用 `AttachThreadInput` 挂到当前前台线程的输入队列，
/// 再 `SetForegroundWindow` + `BringWindowToTop`，最后轮询确认。
fn set_foreground(hwnd: HWND) -> bool {
    if hwnd == 0 {
        return false;
    }
    if foreground() == hwnd {
        return true;
    }
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
    }
    sleep(Duration::from_millis(30));

    // AttachThreadInput 要求调用线程有消息队列；PeekMessage 会隐式创建。
    let mut msg = [0u8; 64];
    unsafe {
        PeekMessageW(msg.as_mut_ptr(), 0, 0, 0, PM_NOREMOVE);
    }
    let current_thread = unsafe { GetCurrentThreadId() };
    let foreground_thread = unsafe { GetWindowThreadProcessId(foreground(), std::ptr::null_mut()) };
    let attached = current_thread != 0
        && foreground_thread != 0
        && current_thread != foreground_thread
        && unsafe { AttachThreadInput(current_thread, foreground_thread, 1) } != 0;

    unsafe {
        SetForegroundWindow(hwnd);
        BringWindowToTop(hwnd);
    }

    if attached {
        unsafe {
            AttachThreadInput(current_thread, foreground_thread, 0);
        }
    }

    wait_foreground(hwnd, 600)
}

fn wait_foreground(hwnd: HWND, timeout_ms: u64) -> bool {
    let start = std::time::Instant::now();
    loop {
        if foreground() == hwnd {
            return true;
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return false;
        }
        sleep(Duration::from_millis(25));
    }
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

fn sendinput_error() -> String {
    format!(
        "SendInput 失败（WinError={}），可能被 UIPI/权限拦截",
        last_error()
    )
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

pub(crate) fn inject(
    text: &str,
    plan: InjectPlan,
    per_char_ms: u32,
    focus_guard: bool,
    target: Option<HWND>,
) -> InjectOutcome {
    if plan == InjectPlan::Off {
        return InjectOutcome::failure("none", chars(text), "注入已关闭");
    }
    let hwnd = match resolve_target(target, chars(text)) {
        Ok(hwnd) => hwnd,
        Err(outcome) => return outcome,
    };

    // UIPI 预检：低完整性进程向更高完整性窗口注入时，SendInput 可能“返回成功
    // 但目标收不到”，WM_CHAR 则直接 Access Denied。提前给出明确错误。
    let current_il = integrity_level(unsafe { GetCurrentProcessId() });
    let target_il = integrity_level(process_id(hwnd));
    if let (Some(current), Some(target_il)) = (current_il, target_il) {
        if current < target_il {
            let message = format!(
                "UIPI 完整性不匹配：上位机={}(0x{:X})，目标={}(0x{:X})；请从普通/管理员终端启动上位机",
                integrity_name(Some(current)),
                current,
                integrity_name(Some(target_il)),
                target_il,
            );
            eprintln!("[inject] {message}");
            return InjectOutcome::failure("none", chars(text), message);
        }
    }

    // 诊断用：目标窗口/当前前台窗口不一致时，能直接看出文字去了哪里。
    eprintln!(
        "[inject] mode={:?} target=0x{:X} '{}' foreground=0x{:X} '{}' chars={}",
        plan,
        hwnd as usize,
        window_title(hwnd),
        foreground() as usize,
        window_title(foreground()),
        chars(text),
    );
    // 刚切完前台时目标窗口可能还在处理激活消息，稍等再敲字。
    sleep(Duration::from_millis(60));

    let outcome = match plan {
        InjectPlan::Off => unreachable!("上面已处理"),
        InjectPlan::SendInput => {
            let first = sendinput(text, per_char_ms, focus_guard, hwnd);
            if first.ok {
                first
            } else {
                let fallback = clipboard(text, focus_guard, hwnd);
                if fallback.ok {
                    InjectOutcome {
                        error: first.error,
                        ..fallback
                    }
                } else {
                    InjectOutcome::failure(
                        "sendinput",
                        chars(text),
                        format!(
                            "{}；剪贴板兜底也失败：{}",
                            first.error.unwrap_or_default(),
                            fallback.error.unwrap_or_default()
                        ),
                    )
                }
            }
        }
        InjectPlan::Clipboard => {
            let first = clipboard(text, focus_guard, hwnd);
            if first.ok {
                first
            } else {
                let fallback = sendinput(text, per_char_ms, focus_guard, hwnd);
                if fallback.ok {
                    InjectOutcome {
                        error: first.error,
                        ..fallback
                    }
                } else {
                    InjectOutcome::failure(
                        "clipboard",
                        chars(text),
                        format!(
                            "{}；SendInput 兜底也失败：{}",
                            first.error.unwrap_or_default(),
                            fallback.error.unwrap_or_default()
                        ),
                    )
                }
            }
        }
    };

    eprintln!(
        "[inject] result ok={} method={} chars={} error={:?} foreground=0x{:X} '{}'",
        outcome.ok,
        outcome.method,
        outcome.chars,
        outcome.error,
        foreground() as usize,
        window_title(foreground()),
    );
    outcome
}

/// 决定这一次文字要打到哪个窗口。
///
/// 规则（优先级从高到低）：
///
/// 1. PTT 开始时捕获的外部窗口，且当前焦点仍是它 → 用它；
/// 2. 当前前台是**另一个外部窗口** → 用它（用户说话后主动换了光标，跟随最新光标）；
/// 3. 当前前台是上位机自己、PTT 开始时捕获的是外部窗口 → 尝试把焦点切回捕获窗口，
///    再注入（避免识别结果被敲进上位机自己）；
/// 4. 两者都是上位机自己 → 明确拒绝，提示先把光标放到目标输入框。
fn resolve_target(stored: Option<HWND>, text_chars: usize) -> Result<HWND, InjectOutcome> {
    let current = foreground();
    let stored = stored.filter(|hwnd| *hwnd != 0);

    if let Some(target) = stored {
        if !is_own_window(target) {
            if current != 0 && !is_own_window(current) && current != target {
                return Ok(current);
            }
            if current != target && !set_foreground(target) {
                return Err(InjectOutcome::failure(
                    "sendinput",
                    text_chars,
                    format!(
                        "无法把焦点切回说话时的目标窗口（当前：{}；目标：{}）；请先把光标放到目标输入框再按 PTT",
                        window_title(foreground()),
                        window_title(target),
                    ),
                ));
            }
            return Ok(target);
        }
    }

    if current != 0 && !is_own_window(current) {
        return Ok(current);
    }

    Err(InjectOutcome::failure(
        "none",
        text_chars,
        "当前前台是上位机自己，文字没有可输入的光标；请先把光标放到目标输入框再按 PTT",
    ))
}

fn sendinput(text: &str, per_char_ms: u32, focus_guard: bool, hwnd: HWND) -> InjectOutcome {
    let mut inserted = 0usize;
    let mut pending = text.chars().peekable();
    while let Some(ch) = pending.next() {
        if ch == '\r' && pending.peek() == Some(&'\n') {
            continue;
        }
        if ch == '\r' || ch == '\n' {
            if focus_guard && foreground() != hwnd {
                return InjectOutcome::failure(
                    "sendinput",
                    inserted,
                    "注入过程中焦点窗口发生变化，已中止",
                );
            }
            if !send_key(VK_RETURN, false) || !send_key(VK_RETURN, true) {
                return InjectOutcome::failure("sendinput", inserted, sendinput_error());
            }
        } else {
            let mut buffer = [0u16; 2];
            let units = ch.encode_utf16(&mut buffer);
            if !send_units(units) {
                return InjectOutcome::failure("sendinput", inserted, sendinput_error());
            }
        }
        inserted += 1;
        if per_char_ms > 0 {
            sleep(Duration::from_millis(u64::from(per_char_ms)));
        }
        if focus_guard && inserted.is_multiple_of(8) && foreground() != hwnd {
            return InjectOutcome::failure(
                "sendinput",
                inserted,
                "注入过程中焦点窗口发生变化，已中止",
            );
        }
    }

    if focus_guard && foreground() != hwnd {
        return InjectOutcome::failure(
            "sendinput",
            inserted,
            "注入后焦点窗口发生变化，文字可能没有完整进入目标窗口",
        );
    }
    InjectOutcome::success("sendinput", chars(text))
}

fn clipboard(text: &str, focus_guard: bool, hwnd: HWND) -> InjectOutcome {
    let previous = clipboard_text();
    if !set_clipboard_text(text) {
        return InjectOutcome::failure("clipboard", 0, "写入剪贴板失败（可能被其他程序占用）");
    }

    let outcome = paste(hwnd, text, focus_guard);

    // 给目标程序留出读取时间后尽量恢复原剪贴板；原来是图片等非文本时不动。
    sleep(Duration::from_millis(300));
    if let Some(previous) = previous {
        set_clipboard_text(&previous);
    }
    outcome
}

fn paste(hwnd: HWND, text: &str, focus_guard: bool) -> InjectOutcome {
    sleep(Duration::from_millis(250));
    if focus_guard && foreground() != hwnd {
        return InjectOutcome::failure("clipboard", 0, "粘贴前焦点窗口发生变化，已中止");
    }
    if !send_key(VK_CONTROL, false) {
        return InjectOutcome::failure("clipboard", 0, "发送 Ctrl 失败");
    }
    if !send_key(VK_V, false) {
        send_key(VK_CONTROL, true);
        return InjectOutcome::failure("clipboard", 0, "发送 V 失败");
    }
    send_key(VK_V, true);
    send_key(VK_CONTROL, true);
    sleep(Duration::from_millis(600));
    if focus_guard && foreground() != hwnd {
        return InjectOutcome::failure(
            "clipboard",
            0,
            "粘贴后焦点窗口发生变化，文字可能进入了错误窗口",
        );
    }
    InjectOutcome::success("clipboard", chars(text))
}

fn clipboard_text() -> Option<String> {
    for _ in 0..5 {
        unsafe {
            if OpenClipboard(0) == 0 {
                sleep(Duration::from_millis(20));
                continue;
            }
            let mut value = None;
            if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
                let handle = GetClipboardData(CF_UNICODETEXT);
                if handle != 0 {
                    let ptr = GlobalLock(handle) as *const u16;
                    if !ptr.is_null() {
                        let mut len = 0usize;
                        while *ptr.add(len) != 0 && len < 1_000_000 {
                            len += 1;
                        }
                        let slice = std::slice::from_raw_parts(ptr, len);
                        value = Some(String::from_utf16_lossy(slice));
                        GlobalUnlock(handle);
                    }
                }
            }
            CloseClipboard();
            return value;
        }
    }
    None
}

fn set_clipboard_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();

    for _ in 0..5 {
        unsafe {
            if OpenClipboard(0) == 0 {
                sleep(Duration::from_millis(20));
                continue;
            }
            let mut ok = false;
            if EmptyClipboard() != 0 {
                let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
                if handle != 0 {
                    let ptr = GlobalLock(handle);
                    if ptr.is_null() {
                        GlobalFree(handle);
                    } else {
                        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr, bytes);
                        GlobalUnlock(handle);
                        // 成功后内存归系统所有，不能 GlobalFree。
                        ok = SetClipboardData(CF_UNICODETEXT, handle) != 0;
                        if !ok {
                            GlobalFree(handle);
                        }
                    }
                }
            }
            CloseClipboard();
            return ok;
        }
    }
    false
}
