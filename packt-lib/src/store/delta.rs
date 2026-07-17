use crate::error::{PacktError, Result};

/// Delta compression using zstd dictionary mode (--patch-from equivalent).
pub struct DeltaEncoder {
    compression_level: i32,
}

impl DeltaEncoder {
    #[must_use]
    pub fn new(compression_level: i32) -> Self {
        Self { compression_level }
    }

    #[must_use]
    pub fn should_attempt(base_chunk: &[u8], new_chunk: &[u8]) -> bool {
        let base_len = base_chunk.len();
        let new_len = new_chunk.len();
        if base_len < 256 || new_len < 256 {
            return false;
        }
        let (larger, smaller) = if base_len >= new_len {
            (base_len as f64, new_len as f64)
        } else {
            (new_len as f64, base_len as f64)
        };
        if smaller == 0.0 {
            return false;
        }
        larger / smaller <= 4.0
    }

    pub fn try_encode(&self, base_chunk: &[u8], new_chunk: &[u8]) -> Result<Option<Vec<u8>>> {
        if !Self::should_attempt(base_chunk, new_chunk) {
            return Ok(None);
        }
        let mut comp = zstd::bulk::Compressor::with_dictionary(self.compression_level, base_chunk)
            .map_err(|e| PacktError::Serialization(format!("zstd dict compress: {e}")))?;
        let delta_frame = comp
            .compress(new_chunk)
            .map_err(|e| PacktError::Serialization(format!("zstd dict compress: {e}")))?;
        let standalone = zstd::bulk::compress(new_chunk, self.compression_level)
            .map_err(|e| PacktError::Serialization(format!("zstd compress: {e}")))?;
        if delta_frame.len() < standalone.len() * 90 / 100 {
            Ok(Some(delta_frame))
        } else {
            Ok(None)
        }
    }

    pub fn decode(&self, base_chunk: &[u8], delta_data: &[u8], output_size: usize) -> Result<Vec<u8>> {
        let mut decomp = zstd::bulk::Decompressor::with_dictionary(base_chunk)
            .map_err(|e| PacktError::Serialization(format!("zstd dict decompress: {e}")))?;
        let data = decomp
            .decompress(delta_data, output_size)
            .map_err(|e| PacktError::Serialization(format!("zstd dict decompress: {e}")))?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp;

    fn test_data(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 251) as u8).collect()
    }

    fn modify_data(base: &[u8], change_ratio: f64) -> Vec<u8> {
        let mut data = base.to_vec();
        let num_changes = (base.len() as f64 * change_ratio) as usize;
        for i in 0..cmp::min(num_changes, base.len()) {
            let pos = (i * 97) % base.len();
            data[pos] = data[pos].wrapping_add(37);
        }
        data
    }

    #[test]
    fn test_delta_identical() {
        let encoder = DeltaEncoder::new(3);
        let data = test_data(4096);
        let delta = encoder.try_encode(&data, &data).unwrap();
        assert!(delta.is_some());
        let decoded = encoder.decode(&data, &delta.unwrap(), data.len()).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_delta_roundtrip_small_change() {
        let encoder = DeltaEncoder::new(3);
        let base = test_data(4096);
        let modified = modify_data(&base, 0.001);
        let delta = encoder.try_encode(&base, &modified).unwrap();
        assert!(delta.is_some());
        let decoded = encoder.decode(&base, &delta.unwrap(), modified.len()).unwrap();
        assert_eq!(decoded, modified);
    }

    #[test]
    fn test_delta_not_beneficial() {
        let encoder = DeltaEncoder::new(3);
        let base = test_data(4096);
        let different: Vec<u8> = (0..4096).map(|i| ((i * 137 + 73) % 251) as u8).collect();
        let result = encoder.try_encode(&base, &different).unwrap();
        if let Some(delta) = result {
            let decoded = encoder.decode(&base, &delta, different.len()).unwrap();
            assert_eq!(decoded, different);
        }
    }

    #[test]
    fn test_delta_small_chunks() {
        assert!(!DeltaEncoder::should_attempt(&[1u8; 100], &[2u8; 100]));
    }

    #[test]
    fn test_delta_realistic_size() {
        let encoder = DeltaEncoder::new(3);
        let base = test_data(32_768);
        let modified = modify_data(&base, 0.005);
        let delta = encoder.try_encode(&base, &modified).unwrap();
        assert!(delta.is_some());
        let decoded = encoder.decode(&base, &delta.unwrap(), modified.len()).unwrap();
        assert_eq!(decoded, modified);
    }
}
