//! 16 kHz / 单声道 / 16 bit PCM 的 WAV 写出。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use kb_protocol::{CHANNELS, SAMPLE_RATE};

/// 生成完整 WAV 字节（44 B 头 + PCM 数据）。
pub fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&u16::from(CHANNELS).to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * u32::from(CHANNELS) * 2).to_le_bytes());
    out.extend_from_slice(&(u16::from(CHANNELS) * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// 写出 WAV，返回写出的字节数。
pub fn write_wav(path: impl AsRef<Path>, pcm: &[i16]) -> std::io::Result<u64> {
    let bytes = wav_bytes(pcm);
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::wav_bytes;

    #[test]
    fn header_is_riff_pcm_16k_mono() {
        let pcm = vec![0i16; 320];
        let wav = wav_bytes(&pcm);
        assert_eq!(wav.len(), 44 + 640);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(wav[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(wav[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 32_000);
        assert_eq!(u16::from_le_bytes(wav[32..34].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 640);
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 640);
    }
}
