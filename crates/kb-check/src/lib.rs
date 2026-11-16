//! 接收校验、CSV 录制与跨链路比较（P2a）。
//!
//! 与参考实现 `py_example/check.py` 对齐，语义冻结于 `docs/interfaces.md` §3.4、§3.5：
//!
//! - [`Checker`]：序号缺口/重复/乱序、长度、内置测试模式、抖动、整体 sha256；
//! - [`Recorder`] / [`load_records`]：CSV 7 列（顺序冻结）；
//! - [`compare_files`]：比较帧数、seq 序列/集合、sha256 序列、payload 长度分布，
//!   **不比较** `dev_ms`、`flags`。
//!
//! 本 crate 不依赖第三方库：SHA-256 内置实现，避免拉取额外依赖。

pub mod checker;
pub mod compare;
pub mod error;
pub mod pattern;
pub mod recorder;
pub mod sha256;
pub mod types;

pub use checker::{Checker, CheckerConfig};
pub use compare::{compare_files, compare_records, CompareReport, Mismatch};
pub use error::CheckError;
pub use pattern::{classify_pattern, Pattern};
pub use recorder::{load_records, Record, RecordSummary, Recorder};
pub use sha256::Sha256;
pub use types::{CheckStats, PayloadInfo};
