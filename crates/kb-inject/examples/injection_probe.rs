//! Phase 0 注入探针：在真实 Windows 桌面上单独验证 SendInput / 剪贴板。
//!
//! 不经过 Tauri、不经过 ASR，只做一件事：
//! 等待用户把焦点放到目标窗口 → 记录 HWND → 注入 → 打印实际发送数和落字验证。
//!
//! 用法（在工程根目录）：
//!
//! ```powershell
//! cargo run -p kb-inject --example injection_probe -- --mode sendinput --delay 5
//! cargo run -p kb-inject --example injection_probe -- --mode clipboard --delay 5
//! cargo run -p kb-inject --example injection_probe -- --text "你好ABC😀" --mode sendinput
//! ```
//!
//! 运行后 5 秒内点进记事本/浏览器/微信的输入框，保持光标闪烁。

#[cfg(not(windows))]
fn main() {
    eprintln!("injection_probe 仅支持 Windows");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::env;
    use std::process::ExitCode;
    use std::thread::sleep;
    use std::time::Duration;

    use kb_inject::{probe, ProbeMode};

    let mut mode = ProbeMode::SendInput;
    let mut text = "注入探针 Probe 123，中文😀".to_string();
    let mut delay_secs = 5u64;
    let mut restore_clipboard = true;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                let value = args.next().unwrap_or_default();
                mode = match value.as_str() {
                    "sendinput" => ProbeMode::SendInput,
                    "clipboard" => ProbeMode::Clipboard,
                    "wmchar" => ProbeMode::WmChar,
                    "inspect" | "dry-run" => ProbeMode::Inspect,
                    other => {
                        eprintln!("未知 mode: {other}（可选 sendinput / clipboard / wmchar / inspect）");
                        std::process::exit(2);
                    }
                };
            }
            "--dry-run" => {
                mode = ProbeMode::Inspect;
            }
            "--text" => {
                text = args.next().unwrap_or_default();
            }
            "--delay" => {
                let value = args.next().unwrap_or_default();
                delay_secs = value.parse().unwrap_or_else(|_| {
                    eprintln!("--delay 需要整数秒，收到: {value}");
                    std::process::exit(2);
                });
            }
            "--no-restore" => {
                restore_clipboard = false;
            }
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }

    println!("=== JnWorkerStream injection probe (Phase 0) ===");
    println!(
        "mode={:?} text={:?} chars={} delay={}s restore_clipboard={}",
        mode,
        text,
        text.chars().count(),
        delay_secs,
        restore_clipboard
    );
    println!("请在这段时间内点进目标输入框，让光标闪烁……");

    for remaining in (1..=delay_secs).rev() {
        println!("  {remaining}s");
        sleep(Duration::from_secs(1));
    }

    let report = probe(&text, mode, restore_clipboard);

    println!();
    println!(
        "RESULT mode={} ok={} target=0x{:X} pid={} class={:?} title={:?}",
        report.mode,
        report.ok,
        report.target_hwnd as usize,
        report.target_pid,
        report.target_class,
        report.target_title
    );
    println!(
        "RESULT integrity current={:?} target={:?}",
        report.current_integrity, report.target_integrity
    );
    println!(
        "RESULT foreground_before=0x{:X} foreground_after=0x{:X} focus_before=0x{:X} focus_after=0x{:X}",
        report.foreground_before as usize,
        report.foreground_after as usize,
        report.focus_before as usize,
        report.focus_after as usize
    );
    println!(
        "RESULT requested_events={} sent_events={} last_error={} elapsed={}ms",
        report.requested_events, report.sent_events, report.last_error, report.elapsed_ms
    );
    println!(
        "RESULT text_len_before={:?} text_len_after={:?} verified_inserted={:?}",
        report.text_before_len, report.text_after_len, report.verified_inserted
    );
    println!(
        "RESULT clipboard_before={:?} clipboard_restored={} note={:?}",
        report.clipboard_before, report.clipboard_restored, report.note
    );
    if let Some(note) = &report.note {
        println!("HINT {note}");
    }

    if report.ok {
        println!("PASS");
        ExitCode::SUCCESS
    } else {
        println!("FAIL");
        ExitCode::from(1)
    }
}

#[cfg(windows)]
fn print_help() {
    println!(
        "injection_probe — Windows 文本注入 Phase 0 探针\n\
         \n\
         参数：\n\
           --mode sendinput|clipboard|wmchar|inspect   注入方式（默认 sendinput）\n\
           --dry-run                    等同 --mode inspect：只看窗口，不注入\n\
           --text <TEXT>                测试文本（默认含中文/英文/数字/Emoji）\n\
           --delay <SECONDS>            等待用户聚焦的秒数（默认 5）\n\
           --no-restore                 剪贴板模式不恢复原剪贴板文本\n\
           -h, --help                   显示帮助\n\
         \n\
         建议：先在记事本测；再测浏览器输入框、微信聊天框、多行编辑框。\n\
         看 RESULT 里的 sent_events / last_error / verified_inserted。"
    );
}
