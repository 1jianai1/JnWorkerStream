//! IMA-ADPCM 解码器（与设备端 `src/audio/adpcm.c`、`audio_rx.py` 对称）。

/// 步长表（89 项）。
pub const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// 索引调整表（16 项）。
pub const INDEX_TABLE: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// 有状态解码器：`predictor` / `index` 跨块连续。
#[derive(Debug, Clone, Default)]
pub struct AdpcmDecoder {
    predictor: i32,
    index: usize,
}

impl AdpcmDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 重置到段首状态（新段/封段后必须调用）。
    pub fn reset(&mut self) {
        self.predictor = 0;
        self.index = 0;
    }

    pub fn predictor(&self) -> i32 {
        self.predictor
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// 解码一块（160 B → 320 采样），低 nibble 先。
    pub fn decode_block(&mut self, data: &[u8]) -> Vec<i16> {
        let mut out = Vec::with_capacity(data.len() * 2);
        self.decode_block_into(data, &mut out);
        out
    }

    /// 解码一块并追加到 `out`。
    pub fn decode_block_into(&mut self, data: &[u8], out: &mut Vec<i16>) {
        out.reserve(data.len() * 2);
        for &byte in data {
            out.push(self.nibble(byte & 0x0F));
            out.push(self.nibble(byte >> 4));
        }
    }

    fn nibble(&mut self, code: u8) -> i16 {
        let step = STEP_TABLE[self.index];
        let mut delta = step >> 3;
        if code & 0x4 != 0 {
            delta += step;
        }
        if code & 0x2 != 0 {
            delta += step >> 1;
        }
        if code & 0x1 != 0 {
            delta += step >> 2;
        }

        let predicted = if code & 0x8 != 0 {
            self.predictor - delta
        } else {
            self.predictor + delta
        };
        self.predictor = predicted.clamp(-32_768, 32_767);
        let next_index = self.index as i32 + i32::from(INDEX_TABLE[usize::from(code & 0x0F)]);
        self.index = next_index.clamp(0, 88) as usize;
        self.predictor as i16
    }
}

#[cfg(test)]
mod tests {
    use super::{AdpcmDecoder, INDEX_TABLE, STEP_TABLE};

    #[test]
    fn tables_have_frozen_lengths() {
        assert_eq!(STEP_TABLE.len(), 89);
        assert_eq!(INDEX_TABLE.len(), 16);
    }

    #[test]
    fn matches_reference_samples() {
        // 参考值由 py_example.audio_rx.AdpcmDecoder 对 bytes(range(160)) 解码得到。
        let data: Vec<u8> = (0..160u16).map(|value| value as u8).collect();
        let mut decoder = AdpcmDecoder::new();
        let pcm = decoder.decode_block(&data);
        assert_eq!(pcm.len(), 320);
        assert_eq!(&pcm[..12], &[0, 0, 1, 1, 4, 4, 8, 8, 15, 16, 27, 28]);
        assert_eq!(&pcm[316..], &[-32768, -32768, -32768, -32768]);
        assert_eq!((decoder.predictor(), decoder.index()), (-32_768, 87));
    }

    #[test]
    fn reset_restores_initial_state() {
        let mut decoder = AdpcmDecoder::new();
        decoder.decode_block(&[0xFF; 160]);
        assert_ne!((decoder.predictor(), decoder.index()), (0, 0));
        decoder.reset();
        assert_eq!((decoder.predictor(), decoder.index()), (0, 0));
        assert_eq!(decoder.decode_block(&[0x00]), vec![0, 0]);
    }
}
