//! 文本注入（P2c-2）：把识别结果打进当前焦点窗口。
//!
//! 策略：
//!
//! 1. `SendInput + KEYEVENTF_UNICODE`：短文本 / `auto` 默认，通用性最好；
//! 2. **剪贴板 + `Ctrl+V`**：长文本（> 40 字符）或 SendInput 失败时兜底，尽量恢复原剪贴板文本；
//! 3. 目标窗口完整性更高时 UIPI 会拦截；注入前做完整性预检，返回明确错误而不是静默失败。
//!
//! 2026-11-18 Phase 0 结论：之前“SendInput 静默成功但不落字”的根因是
//! 上位机进程处于 Low 完整性（受限终端/沙箱），目标 Notepad 是 Medium；
//! High/Medium 下 `SendInput` 实测正常。**启动上位机的终端完整性不能低于目标窗口。**
//!
//! 平台实现集中在 [`win`]；非 Windows 平台返回 `ok=false` 的明确错误。

#[cfg(windows)]
mod win;

/// Phase 0 探针模式：单独验证 SendInput / 剪贴板，不经过 Tauri。
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeMode {
    SendInput,
    Clipboard,
    /// 诊断用：直接给焦点控件发 `WM_CHAR`，绕过系统输入队列。
    WmChar,
    /// 只看目标/焦点，不注入；用于确认探针抓到的窗口是否正确。
    Inspect,
}

/// Phase 0 探针结果。字段尽量保留原始 Win32 信息，便于写测试报告。
#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub mode: &'static str,
    pub ok: bool,
    pub target_hwnd: isize,
    pub target_title: String,
    pub target_class: String,
    pub target_pid: u32,
    pub own_process: bool,
    /// 当前进程与目标进程的完整性级别 RID（0x1000 Low / 0x2000 Medium / 0x3000 High）。
    pub current_integrity: Option<u32>,
    pub target_integrity: Option<u32>,
    pub foreground_before: isize,
    pub foreground_after: isize,
    pub focus_before: isize,
    pub focus_after: isize,
    pub requested_events: usize,
    pub sent_events: usize,
    pub last_error: u32,
    pub elapsed_ms: u128,
    pub text_before_len: Option<usize>,
    pub text_after_len: Option<usize>,
    pub verified_inserted: Option<bool>,
    pub clipboard_before: Option<String>,
    pub clipboard_restored: bool,
    pub note: Option<String>,
}

#[cfg(windows)]
pub use win::{current_integrity, integrity_label, probe};

/// 剪贴板兜底的字符阈值（与参考实现一致）。
pub const CLIPBOARD_THRESHOLD: usize = 40;

/// 注入模式（对应 `voice.inject_mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectMode {
    Auto,
    SendInput,
    Clipboard,
    Off,
}

impl InjectMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "sendinput" | "send_input" | "keys" => Some(Self::SendInput),
            "clipboard" | "paste" => Some(Self::Clipboard),
            "off" | "none" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::SendInput => "sendinput",
            Self::Clipboard => "clipboard",
            Self::Off => "off",
        }
    }
}

/// 一次注入的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectOutcome {
    pub ok: bool,
    /// `sendinput` / `clipboard` / `none`。
    pub method: &'static str,
    pub chars: usize,
    pub error: Option<String>,
}

impl InjectOutcome {
    pub fn success(method: &'static str, chars: usize) -> Self {
        Self {
            ok: true,
            method,
            chars,
            error: None,
        }
    }

    pub fn failure(method: &'static str, chars: usize, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            method,
            chars,
            error: Some(error.into()),
        }
    }
}

/// 由模式与文本长度决定的执行方式（纯函数，可单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectPlan {
    Off,
    SendInput,
    Clipboard,
}

/// `auto`：长文本先走剪贴板，短文本先走 SendInput（与参考实现一致）。
///
/// 2026-11-18 Phase 0 复验：之前“SendInput 静默成功但不落字”是
/// **Low 完整性 → Medium Notepad 的 UIPI 拦截**，不是 Unicode 写法问题；
/// High/Medium 下 `SendInput` 实测 38/38、文字完整落入 RichEdit。
/// 因此恢复 SendInput 主路径；UIPI 由 `win::inject` 预检并明确报错。
pub fn plan(mode: InjectMode, chars: usize) -> InjectPlan {
    match mode {
        InjectMode::Off => InjectPlan::Off,
        InjectMode::SendInput => InjectPlan::SendInput,
        InjectMode::Clipboard => InjectPlan::Clipboard,
        InjectMode::Auto if chars > CLIPBOARD_THRESHOLD => InjectPlan::Clipboard,
        InjectMode::Auto => InjectPlan::SendInput,
    }
}

/// 注入器配置。
#[derive(Debug, Clone)]
pub struct Injector {
    per_char_ms: u32,
    focus_guard: bool,
    /// PTT 开始时的前台窗口句柄（`GetForegroundWindow`）；识别结果回来时优先回到它。
    /// 0 / `None` 表示未捕获，按注入当刻的前台窗口处理。
    target: Option<isize>,
}

impl Default for Injector {
    fn default() -> Self {
        Self {
            per_char_ms: 6,
            focus_guard: true,
            target: None,
        }
    }
}

impl Injector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_per_char_ms(mut self, ms: u32) -> Self {
        self.per_char_ms = ms;
        self
    }

    pub fn per_char_ms(&self) -> u32 {
        self.per_char_ms
    }

    pub fn focus_guard(&self) -> bool {
        self.focus_guard
    }

    pub fn set_focus_guard(&mut self, enabled: bool) {
        self.focus_guard = enabled;
    }

    /// PTT 按下（`AUDIO_CTL ev=0`）时调用：记住此时的前台窗口。
    ///
    /// 识别结果通常在松手后 1–3 s 才回来；用户可能已经点回上位机看结果，
    /// 此时直接 `SendInput` 会把文字打进上位机自己。记住目标窗口后，注入前会
    /// 优先切回它（见 `win::resolve_target`）。
    pub fn begin_segment(&mut self) {
        #[cfg(windows)]
        {
            self.target = win::foreground_window();
        }
    }

    /// 最近一次 `begin_segment` 捕获的窗口句柄（诊断/测试用）。
    pub fn target(&self) -> Option<isize> {
        self.target
    }

    /// 按 `mode` 注入文本；空文本视为成功（`method = "none"`）。
    pub fn inject(&mut self, text: &str, mode: InjectMode) -> InjectOutcome {
        let chars = text.chars().count();
        if chars == 0 {
            return InjectOutcome::success("none", 0);
        }
        if mode == InjectMode::Off {
            return InjectOutcome::failure("none", chars, "注入已关闭（voice.inject_mode=off）");
        }
        let plan = plan(mode, chars);
        if plan == InjectPlan::Off {
            return InjectOutcome::failure("none", chars, "注入已关闭（voice.inject_mode=off）");
        }

        #[cfg(windows)]
        {
            win::inject(
                text,
                plan,
                self.per_char_ms,
                self.focus_guard,
                self.target,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = plan;
            InjectOutcome::failure("none", chars, "文本注入仅支持 Windows")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prefers_sendinput_for_short_text() {
        assert_eq!(plan(InjectMode::Auto, 1), InjectPlan::SendInput);
        assert_eq!(plan(InjectMode::Auto, 40), InjectPlan::SendInput);
        assert_eq!(plan(InjectMode::Auto, 41), InjectPlan::Clipboard);
        assert_eq!(plan(InjectMode::Auto, 500), InjectPlan::Clipboard);
    }

    #[test]
    fn explicit_modes_bypass_threshold() {
        assert_eq!(plan(InjectMode::SendInput, 500), InjectPlan::SendInput);
        assert_eq!(plan(InjectMode::Clipboard, 2), InjectPlan::Clipboard);
        assert_eq!(plan(InjectMode::Off, 10), InjectPlan::Off);
    }

    #[test]
    fn parses_config_values() {
        assert_eq!(InjectMode::parse("auto"), Some(InjectMode::Auto));
        assert_eq!(InjectMode::parse("SendInput"), Some(InjectMode::SendInput));
        assert_eq!(InjectMode::parse("paste"), Some(InjectMode::Clipboard));
        assert_eq!(InjectMode::parse("none"), Some(InjectMode::Off));
        assert_eq!(InjectMode::parse("whatever"), None);
        assert_eq!(InjectMode::Auto.as_str(), "auto");
    }

    #[test]
    fn empty_text_is_a_noop() {
        let mut injector = Injector::new();
        let outcome = injector.inject("", InjectMode::Auto);
        assert!(outcome.ok);
        assert_eq!(outcome.method, "none");
        assert_eq!(outcome.chars, 0);
    }

    #[test]
    fn off_mode_reports_error_without_touching_the_desktop() {
        let mut injector = Injector::new();
        let outcome = injector.inject("你好", InjectMode::Off);
        assert!(!outcome.ok);
        assert!(outcome.error.unwrap().contains("注入已关闭"));
    }

    #[test]
    fn char_count_is_unicode_aware() {
        let mut injector = Injector::new();
        // 不真正注入：off 模式在计数后返回。
        let outcome = injector.inject("你好ab", InjectMode::Off);
        assert_eq!(outcome.chars, 4);
    }
}
