//! sidecar 帧编解码（ADR-007），与 `sidecars/ble/protocol.py` 逐字节对齐。

use std::io::{Read, Write};

use serde_json::{Map, Value};

use crate::error::SidecarError;

/// 帧 magic。
pub const MAGIC: [u8; 2] = *b"JN";
/// 协议版本。
pub const PROTO_VERSION: u8 = 1;
/// 单帧上限（header + body）。
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
/// 固定前缀长度：magic(2) + version(1) + flags(1) + header_len(4) + body_len(4)。
pub const PREFIX_LEN: usize = 12;
/// 帧头 flags：body gzip（V1 恒 0）。
pub const FLAG_BODY_GZIP: u8 = 0x01;

/// 一条 sidecar 帧。
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarFrame {
    pub header: Map<String, Value>,
    pub body: Vec<u8>,
}

impl SidecarFrame {
    pub fn kind(&self) -> Option<&str> {
        self.header.get("kind").and_then(Value::as_str)
    }

    pub fn id(&self) -> Option<&str> {
        self.header.get("id").and_then(Value::as_str)
    }

    pub fn event(&self) -> Option<&str> {
        self.header.get("event").and_then(Value::as_str)
    }

    pub fn method(&self) -> Option<&str> {
        self.header.get("method").and_then(Value::as_str)
    }

    pub fn ok(&self) -> bool {
        self.header
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 错误码（`error.code`）。
    pub fn error_code(&self) -> Option<&str> {
        self.header
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
    }

    /// 错误消息（`error.message`）。
    pub fn error_message(&self) -> Option<&str> {
        self.header
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.header.get(key).and_then(Value::as_u64)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.header.get(key).and_then(Value::as_str)
    }
}

/// 编码一帧（含 magic 与长度）。
pub fn encode(header: &Map<String, Value>, body: &[u8]) -> Result<Vec<u8>, SidecarError> {
    if !header.contains_key("kind") || !header.contains_key("ts_ms") {
        return Err(SidecarError::protocol("header 缺 kind 或 ts_ms"));
    }
    let header_bytes = serde_json::to_vec(&Value::Object(header.clone()))
        .map_err(|error| SidecarError::protocol(format!("header 序列化失败: {error}")))?;
    let total = PREFIX_LEN + header_bytes.len() + body.len();
    if total > MAX_FRAME {
        return Err(SidecarError::protocol(format!(
            "帧 {total} B 超过 {MAX_FRAME} B 上限"
        )));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC);
    out.push(PROTO_VERSION);
    out.push(0);
    out.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(body);
    Ok(out)
}

/// 写入一帧并 flush。
pub fn write_frame(
    writer: &mut impl Write,
    header: &Map<String, Value>,
    body: &[u8],
) -> Result<usize, SidecarError> {
    let data = encode(header, body)?;
    writer
        .write_all(&data)
        .and_then(|()| writer.flush())
        .map_err(|error| SidecarError::write_failed(format!("写 sidecar 失败: {error}")))?;
    Ok(data.len())
}

fn read_exact(reader: &mut impl Read, size: usize) -> Result<Vec<u8>, SidecarError> {
    let mut buffer = vec![0u8; size];
    reader
        .read_exact(&mut buffer)
        .map_err(|error| SidecarError::protocol(format!("半帧/读失败: {error}")))?;
    Ok(buffer)
}

/// 读取一帧；EOF 返回 `None`。
pub fn read_frame(reader: &mut impl Read) -> Result<Option<SidecarFrame>, SidecarError> {
    let mut prefix = [0u8; PREFIX_LEN];
    let mut filled = 0usize;
    while filled < PREFIX_LEN {
        let read = reader
            .read(&mut prefix[filled..])
            .map_err(|error| SidecarError::protocol(format!("读 sidecar 失败: {error}")))?;
        if read == 0 {
            if filled == 0 {
                return Ok(None);
            }
            return Err(SidecarError::protocol(format!(
                "半帧结束：期望 {PREFIX_LEN} B，实际 {filled} B"
            )));
        }
        filled += read;
    }

    if prefix[0..2] != MAGIC {
        return Err(SidecarError::protocol("magic 非法"));
    }
    if prefix[2] != PROTO_VERSION {
        return Err(SidecarError::version_mismatch(prefix[2], PROTO_VERSION));
    }
    let flags = prefix[3];
    if flags & !FLAG_BODY_GZIP != 0 {
        return Err(SidecarError::protocol(format!("未知 flags: 0x{flags:02X}")));
    }
    let header_len = u32::from_le_bytes(prefix[4..8].try_into().expect("4 B")) as usize;
    let body_len = u32::from_le_bytes(prefix[8..12].try_into().expect("4 B")) as usize;
    if PREFIX_LEN + header_len + body_len > MAX_FRAME {
        return Err(SidecarError::protocol("帧超过 8 MiB 上限"));
    }

    let header_bytes = read_exact(reader, header_len)?;
    let body = read_exact(reader, body_len)?;
    let value: Value = serde_json::from_slice(&header_bytes)
        .map_err(|error| SidecarError::protocol(format!("header JSON 非法: {error}")))?;
    let Value::Object(header) = value else {
        return Err(SidecarError::protocol("header 必须是 JSON object"));
    };
    if !header.contains_key("kind") {
        return Err(SidecarError::protocol("header 缺 kind"));
    }
    Ok(Some(SidecarFrame { header, body }))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    fn header(kind: &str) -> Map<String, Value> {
        let Value::Object(map) = json!({
            "kind": kind,
            "id": "req-1",
            "ts_ms": 1_763_200_000_000u64,
            "ok": true,
            "note": "已配对"
        }) else {
            unreachable!()
        };
        map
    }

    fn encode_header(kind: &str) -> Vec<u8> {
        encode(&header(kind), b"").expect("编码")
    }

    #[test]
    fn roundtrip_with_body_and_unicode() {
        let body: Vec<u8> = (0..64u8).collect();
        let raw = encode(&header("response"), &body).expect("编码");
        let mut cursor = Cursor::new(raw);
        let frame = read_frame(&mut cursor).expect("读取").expect("有帧");
        assert_eq!(frame.kind(), Some("response"));
        assert_eq!(frame.id(), Some("req-1"));
        assert_eq!(frame.body, body);
        assert_eq!(
            frame.header.get("note").and_then(Value::as_str),
            Some("已配对")
        );
        assert!(read_frame(&mut cursor).expect("EOF").is_none());
    }

    #[test]
    fn empty_body() {
        let raw = encode_header("event");
        let frame = read_frame(&mut Cursor::new(raw))
            .expect("读取")
            .expect("帧");
        assert!(frame.body.is_empty());
    }

    #[test]
    fn half_prefix_is_error() {
        let raw = encode_header("event");
        let error = read_frame(&mut Cursor::new(&raw[..7])).expect_err("半帧应报错");
        assert_eq!(error.code(), "E_SIDECAR_PROTOCOL");
    }

    #[test]
    fn bad_magic_and_version() {
        let mut raw = encode_header("event");
        raw[0] = 0;
        assert!(read_frame(&mut Cursor::new(raw)).is_err());

        let mut raw = encode_header("event");
        raw[2] = 2;
        let error = read_frame(&mut Cursor::new(raw)).expect_err("版本应报错");
        assert_eq!(error.code(), "E_SIDECAR_PROTOCOL");
        assert!(error.to_string().contains("版本"));
    }

    #[test]
    fn oversized_frame_rejected() {
        let mut raw = encode_header("event");
        raw[4..8].copy_from_slice(&(MAX_FRAME as u32).to_le_bytes());
        let error = read_frame(&mut Cursor::new(raw)).expect_err("超限应报错");
        assert!(error.to_string().contains("8 MiB"));
    }

    #[test]
    fn header_must_be_object_with_kind() {
        let header_bytes = b"[]";
        let mut raw = Vec::new();
        raw.extend_from_slice(&MAGIC);
        raw.push(PROTO_VERSION);
        raw.push(0);
        raw.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(header_bytes);
        assert!(read_frame(&mut Cursor::new(raw)).is_err());

        let no_kind = b"{\"ts_ms\":1}";
        let mut raw = Vec::new();
        raw.extend_from_slice(&MAGIC);
        raw.push(PROTO_VERSION);
        raw.push(0);
        raw.extend_from_slice(&(no_kind.len() as u32).to_le_bytes());
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(no_kind);
        assert!(read_frame(&mut Cursor::new(raw)).is_err());
    }

    #[test]
    fn encode_requires_kind_and_ts() {
        let mut map = Map::new();
        map.insert("kind".to_string(), json!("event"));
        assert!(encode(&map, b"").is_err());
        map.insert("ts_ms".to_string(), json!(1));
        assert!(encode(&map, b"").is_ok());
    }

    #[test]
    fn error_fields_are_parsed() {
        let Value::Object(mut map) = json!({
            "kind": "response",
            "id": "r2",
            "ts_ms": 1,
            "ok": false,
            "error": {"code": "E_BLE_CONNECT_TIMEOUT", "message": "超时"}
        }) else {
            unreachable!()
        };
        let frame = SidecarFrame {
            header: std::mem::take(&mut map),
            body: Vec::new(),
        };
        assert!(!frame.ok());
        assert_eq!(frame.error_code(), Some("E_BLE_CONNECT_TIMEOUT"));
        assert_eq!(frame.error_message(), Some("超时"));
    }
}
