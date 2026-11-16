//! 错误类型。

use std::fmt;

/// `kb-check` 的失败。
#[derive(Debug)]
pub enum CheckError {
    /// 未知的测试模式名。
    InvalidPattern(String),
    /// CSV 行格式错误。
    Csv {
        path: String,
        line: usize,
        message: String,
    },
    /// 文件读写失败。
    Io { path: String, message: String },
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPattern(name) => write!(
                f,
                "未知 pattern: {name:?}，可选 auto/any/inc/zero/ascii/empty"
            ),
            Self::Csv {
                path,
                line,
                message,
            } => write!(f, "{path}:{line}: 行格式错误: {message}"),
            Self::Io { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for CheckError {}

impl CheckError {
    pub(crate) fn io(path: impl AsRef<std::path::Path>, error: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            message: error.to_string(),
        }
    }

    pub(crate) fn csv(
        path: impl AsRef<std::path::Path>,
        line: usize,
        message: impl Into<String>,
    ) -> Self {
        Self::Csv {
            path: path.as_ref().display().to_string(),
            line,
            message: message.into(),
        }
    }
}
