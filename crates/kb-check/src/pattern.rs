//! 内置测试数据模式（与设备端 `test_src.c` 对齐）。

use crate::error::CheckError;

/// 测试数据模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Auto,
    Any,
    Inc,
    Zero,
    Ascii,
    Empty,
}

impl Pattern {
    /// 全部合法模式。
    pub const ALL: [Self; 6] = [
        Self::Auto,
        Self::Any,
        Self::Inc,
        Self::Zero,
        Self::Ascii,
        Self::Empty,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Any => "any",
            Self::Inc => "inc",
            Self::Zero => "zero",
            Self::Ascii => "ascii",
            Self::Empty => "empty",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CheckError> {
        match value {
            "auto" => Ok(Self::Auto),
            "any" => Ok(Self::Any),
            "inc" => Ok(Self::Inc),
            "zero" => Ok(Self::Zero),
            "ascii" => Ok(Self::Ascii),
            "empty" => Ok(Self::Empty),
            other => Err(CheckError::InvalidPattern(other.to_string())),
        }
    }
}

/// 判断一段测试数据属于哪种内置模式。
pub fn classify_pattern(data: &[u8], seq: u16) -> &'static str {
    if data.is_empty() {
        return "empty";
    }
    if data.iter().all(|&byte| byte == 0) {
        return "zero";
    }
    if data
        .iter()
        .enumerate()
        .all(|(i, &byte)| byte == inc_byte(seq, i))
    {
        return "inc";
    }
    if data
        .iter()
        .all(|&byte| (0x20..0x7F).contains(&byte) || matches!(byte, 0x09 | 0x0A | 0x0D))
    {
        return "ascii";
    }
    "other"
}

pub(crate) fn inc_byte(seq: u16, index: usize) -> u8 {
    ((usize::from(seq) * 7 + index) & 0xFF) as u8
}

pub(crate) fn pattern_matches(expected: &str, data: &[u8], seq: u16) -> bool {
    match expected {
        "any" | "other" => true,
        "inc" => {
            !data.is_empty()
                && data
                    .iter()
                    .enumerate()
                    .all(|(i, &byte)| byte == inc_byte(seq, i))
        }
        "zero" => data.iter().all(|&byte| byte == 0),
        "ascii" => data
            .iter()
            .all(|&byte| (0x20..0x7F).contains(&byte) || matches!(byte, 0x09 | 0x0A | 0x0D)),
        "empty" => data.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_pattern, pattern_matches, Pattern};

    #[test]
    fn parses_known_patterns() {
        for pattern in Pattern::ALL {
            assert_eq!(
                Pattern::parse(pattern.as_str()).expect("已知模式应可解析"),
                pattern
            );
        }
        assert!(Pattern::parse("bogus").is_err());
    }

    #[test]
    fn classifies_and_verifies() {
        let inc: Vec<u8> = (0..8).map(|i| ((3 * 7 + i) & 0xFF) as u8).collect();
        assert_eq!(classify_pattern(&inc, 3), "inc");
        assert!(pattern_matches("inc", &inc, 3));
        assert!(!pattern_matches("zero", &inc, 3));

        assert_eq!(classify_pattern(&[], 0), "empty");
        assert_eq!(classify_pattern(&[0, 0], 0), "zero");
        assert_eq!(classify_pattern(b"hello", 0), "ascii");
        assert_eq!(classify_pattern(&[0xFF, 0x00], 0), "other");
    }
}
