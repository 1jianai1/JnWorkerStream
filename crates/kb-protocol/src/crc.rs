//! CRC-16/CCITT-FALSE（`poly=0x1021`、`init=0xFFFF`、不反射、`xorout=0`）。
//!
//! 逐位实现，与设备端 C 代码及 `py_example/frame.py::crc16` 完全一致。

/// 计算 CRC-16/CCITT-FALSE。
///
/// 自检向量：`crc16(b"123456789") == 0x29B1`、`crc16(b"") == 0xFFFF`。
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::crc16;

    #[test]
    fn reference_vectors() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
        assert_eq!(crc16(b""), 0xFFFF);
    }

    #[test]
    fn full_byte_range_is_stable() {
        let data: Vec<u8> = (0..=255u8).collect();
        assert_eq!(crc16(&data), crc16(&data));
        assert_ne!(crc16(&data), 0);
    }
}
