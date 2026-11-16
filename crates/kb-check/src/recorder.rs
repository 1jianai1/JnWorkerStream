//! CSV 录制与读取（`seq,sha256,arrival_ms,payload_len,data_len,dev_ms,flags`）。

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::CheckError;
use crate::types::PayloadInfo;

/// 冻结的 CSV 列顺序。
pub const CSV_FIELDS: [&str; 7] = [
    "seq",
    "sha256",
    "arrival_ms",
    "payload_len",
    "data_len",
    "dev_ms",
    "flags",
];

/// 一条 CSV 记录。
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub seq: u16,
    pub sha256: String,
    pub arrival_ms: f64,
    pub payload_len: usize,
    pub data_len: usize,
    pub dev_ms: u32,
    pub flags: u8,
}

/// 录制结束后的汇总。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSummary {
    pub path: String,
    pub rows: u64,
    pub bytes: u64,
}

/// 把每帧写成一行的 CSV 录制器。
#[derive(Debug)]
pub struct Recorder {
    path: PathBuf,
    writer: BufWriter<File>,
    rows: u64,
    bytes: u64,
    since_flush: u32,
    flush_every: u32,
}

impl Recorder {
    /// 创建文件并写入表头；父目录不存在时自动创建。
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CheckError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|error| CheckError::io(&path, error))?;
            }
        }
        let file = File::create(&path).map_err(|error| CheckError::io(&path, error))?;
        let mut writer = BufWriter::new(file);
        let header = format!("{}\n", CSV_FIELDS.join(","));
        writer
            .write_all(header.as_bytes())
            .map_err(|error| CheckError::io(&path, error))?;
        Ok(Self {
            path,
            writer,
            rows: 0,
            bytes: header.len() as u64,
            since_flush: 0,
            flush_every: 64,
        })
    }

    /// 写入一行；`arrival_ms` 保留 3 位小数。
    pub fn write(&mut self, info: &PayloadInfo, arrival_ms: f64) -> Result<(), CheckError> {
        let row = format!(
            "{},{},{:.3},{},{},{},{}\n",
            info.seq,
            info.sha256,
            arrival_ms,
            info.payload_len,
            info.data_len,
            info.dev_ms,
            info.flags
        );
        self.writer
            .write_all(row.as_bytes())
            .map_err(|error| CheckError::io(&self.path, error))?;
        self.rows += 1;
        self.bytes += row.len() as u64;
        self.since_flush += 1;
        if self.since_flush >= self.flush_every {
            self.writer
                .flush()
                .map_err(|error| CheckError::io(&self.path, error))?;
            self.since_flush = 0;
        }
        Ok(())
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// flush 并返回汇总（幂等：可多次调用，但之后还可继续写）。
    pub fn flush(&mut self) -> Result<RecordSummary, CheckError> {
        self.writer
            .flush()
            .map_err(|error| CheckError::io(&self.path, error))?;
        self.since_flush = 0;
        Ok(RecordSummary {
            path: self.path.display().to_string(),
            rows: self.rows,
            bytes: self.bytes,
        })
    }
}

/// 读取 `Recorder` 生成的 CSV（兼容只有前三列的精简 CSV）。
pub fn load_records(path: impl AsRef<Path>) -> Result<Vec<Record>, CheckError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|error| CheckError::io(path, error))?;
    let mut reader = BufReader::new(file);
    let mut head = String::new();
    reader
        .read_line(&mut head)
        .map_err(|error| CheckError::io(path, error))?;
    let header = head.trim_start_matches('\u{feff}').trim();
    if header.is_empty() || !header.split(',').any(|field| field.trim() == "seq") {
        return Err(CheckError::csv(
            path,
            1,
            "缺少 seq 列，不是本程序录制的 CSV",
        ));
    }

    let mut records = Vec::new();
    let mut line = String::new();
    let mut line_no = 1usize;
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| CheckError::io(path, error))?;
        if read == 0 {
            break;
        }
        line_no += 1;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        records.push(parse_row(path, line_no, text)?);
    }
    Ok(records)
}

fn parse_row(path: &Path, line_no: usize, text: &str) -> Result<Record, CheckError> {
    let fields: Vec<&str> = text.split(',').map(unquote).collect();
    if fields.len() < 3 {
        return Err(CheckError::csv(path, line_no, "字段不足 3 列"));
    }
    let parse = |value: &str, name: &str| -> Result<u64, CheckError> {
        value
            .trim()
            .parse::<u64>()
            .map_err(|error| CheckError::csv(path, line_no, format!("{name} 非法: {error}")))
    };
    let seq = parse(fields[0], "seq")?;
    let seq = u16::try_from(seq)
        .map_err(|_| CheckError::csv(path, line_no, format!("seq 超出 u16: {seq}")))?;
    let sha256 = fields[1].trim().to_lowercase();
    if sha256.is_empty() {
        return Err(CheckError::csv(path, line_no, "sha256 为空"));
    }
    let arrival_ms = fields[2]
        .trim()
        .parse::<f64>()
        .map_err(|error| CheckError::csv(path, line_no, format!("arrival_ms 非法: {error}")))?;
    let field_u64 = |index: usize, name: &str| -> Result<u64, CheckError> {
        fields
            .get(index)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map_or(Ok(0), |value| {
                value.parse::<u64>().map_err(|error| {
                    CheckError::csv(path, line_no, format!("{name} 非法: {error}"))
                })
            })
    };
    let payload_len = field_u64(3, "payload_len")? as usize;
    let data_len = field_u64(4, "data_len")? as usize;
    let dev_ms = u32::try_from(field_u64(5, "dev_ms")?)
        .map_err(|_| CheckError::csv(path, line_no, "dev_ms 超出 u32"))?;
    let flags = u8::try_from(field_u64(6, "flags")?)
        .map_err(|_| CheckError::csv(path, line_no, "flags 超出 u8"))?;

    Ok(Record {
        seq,
        sha256,
        arrival_ms,
        payload_len,
        data_len,
        dev_ms,
        flags,
    })
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"')
}
