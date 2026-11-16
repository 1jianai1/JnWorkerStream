//! Windows 注入实现：`SendInput`（KEYEVENTF_UNICODE）与剪贴板兜底。
//!
//! 只声明本项目用到的 user32/kernel32 函数；结构体宽度按 x64/x86 对齐
//! （`INPUT` = 4 B 类型 + 4 B 对齐 + 32 B union = 40 B）。

#![allow(non_camel_case_types, non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use std::thread::sleep;
use std::time::Duration;

use crate::{InjectOutcome, InjectPlan};

type BOOL = i32;
type UINT = u32;
type DWORD = u32;
type WORD = u16;
type HWND = isize;
type HGLOBAL = isize;
type LPVOID = *mut u8;

const INPUT_KEYBOARD: DWORD = 1;
const KEYEVENTF_KEYUP: DWORD = 0x0002;
const KEYEVENTF_UNICODE: DWORD = 0x0004;
const VK_CONTROL: WORD = 0x11;
const VK_V: WORD = 0x56;
const VK_RETURN: WORD = 0x0D;
const CF_UNICODETEXT: UINT = 13;
const GMEM_MOVEABLE: UINT = 0x0002;

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

#[link(name = "user32")]
extern "system" {
    fn SendInput(inputs: UINT, out: *const INPUT, size: i32) -> UINT;
    fn GetForegroundWindow() -> HWND;
    fn OpenClipboard(owner: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn IsClipboardFormatAvailable(format: UINT) -> BOOL;
    fn GetClipboardData(format: UINT) -> HGLOBAL;
    fn SetClipboardData(format: UINT, mem: HGLOBAL) -> HGLOBAL;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
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
) -> InjectOutcome {
    match plan {
        InjectPlan::Off => InjectOutcome::failure("none", chars(text), "注入已关闭"),
        InjectPlan::SendInput => {
            let first = sendinput(text, per_char_ms, focus_guard);
            if first.ok {
                return first;
            }
            let fallback = clipboard(text, focus_guard);
            if fallback.ok {
                return InjectOutcome {
                    error: first.error,
                    ..fallback
                };
            }
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
        InjectPlan::Clipboard => {
            let first = clipboard(text, focus_guard);
            if first.ok {
                return first;
            }
            let fallback = sendinput(text, per_char_ms, focus_guard);
            if fallback.ok {
                return InjectOutcome {
                    error: first.error,
                    ..fallback
                };
            }
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

fn sendinput(text: &str, per_char_ms: u32, focus_guard: bool) -> InjectOutcome {
    let hwnd = foreground();
    if hwnd == 0 {
        return InjectOutcome::failure("sendinput", 0, "没有可注入的前台窗口");
    }

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

fn clipboard(text: &str, focus_guard: bool) -> InjectOutcome {
    let hwnd = foreground();
    if hwnd == 0 {
        return InjectOutcome::failure("clipboard", 0, "没有可注入的前台窗口");
    }
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
