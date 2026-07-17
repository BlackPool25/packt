use crate::types::Chunk;

// Chunk hint system for semantic boundary alignment.
// Hints guide the chunker to prefer semantic positions (e.g., tensor
// boundaries in ML checkpoints). The force_split function guarantees
// alignment as a post-processing step.

/// Provides preferred chunk boundary positions for a data buffer.
pub trait ChunkHinter: Send + Sync {
    /// Return byte offsets within `data` that are preferred chunk boundaries.
    fn hints(&self, data: &[u8]) -> Vec<usize>;
}

/// A no-op hinter that produces no hints.
pub struct NoopHinter;

impl ChunkHinter for NoopHinter {
    fn hints(&self, _data: &[u8]) -> Vec<usize> {
        Vec::new()
    }
}

/// Parse a safetensors header and return tensor data offsets as hints.
pub struct SafetensorsHinter;

impl ChunkHinter for SafetensorsHinter {
    fn hints(&self, data: &[u8]) -> Vec<usize> {
        if data.len() < 8 {
            return Vec::new();
        }
        let header_size = u64::from_le_bytes(match data[..8].try_into() {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        });
        let raw_end = match 8usize.checked_add(header_size as usize) {
            Some(end) if end <= data.len() => end,
            _ => return Vec::new(),
        };
        // Data section starts at 8-byte aligned boundary after header
        let data_start = (raw_end + 7) & !7;
        if data_start >= data.len() {
            return Vec::new();
        }
        let json_bytes = &data[8..raw_end];
        let json_end = json_bytes.iter().position(|&b| b == 0).unwrap_or(json_bytes.len());
        let Ok(json_str) = std::str::from_utf8(&json_bytes[..json_end]) else {
            return Vec::new();
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) else {
            return Vec::new();
        };
        let mut result: Vec<usize> = Vec::new();
        if let serde_json::Value::Object(map) = parsed {
            for info in map.values() {
                if let Some(offsets) = info.get("data_offsets").and_then(|v| v.as_array()) {
                    if let Some(start) = offsets.first().and_then(serde_json::Value::as_u64) {
                        let offset = data_start + start as usize;
                        if offset >= data_start && offset < data.len() {
                            result.push(offset);
                        }
                    }
                }
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }
}

/// Force-split chunks at boundary positions.
///
/// Any chunk spanning a boundary is split. `boundaries` must be sorted.
pub fn force_split(chunks: &mut Vec<Chunk>, boundaries: &[usize]) {
    if boundaries.is_empty() {
        return;
    }
    let mut i = 0;
    while i < chunks.len() {
        let chunk = &mut chunks[i];
        let chunk_start = chunk.offset as usize;
        let chunk_end = chunk_start + chunk.length as usize;
        let split_idx = boundaries.iter().position(|&b| b > chunk_start && b < chunk_end);
        if let Some(bi) = split_idx {
            let split_at = boundaries[bi];
            let left_len = (split_at - chunk_start) as u32;
            if left_len > 0 && left_len < chunk.length {
                let right_len = chunk.length - left_len;
                let right_data = chunk.data[left_len as usize..].to_vec();
                chunk.length = left_len;
                chunk.data.truncate(left_len as usize);
                chunks.insert(
                    i + 1,
                    Chunk {
                        offset: split_at as u64,
                        length: right_len,
                        data: right_data,
                    },
                );
                i += 1;
                continue;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Chunk;

    #[test]
    fn test_empty_boundaries() {
        let mut chunks = vec![Chunk {
            offset: 0,
            length: 100,
            data: vec![0u8; 100],
        }];
        force_split(&mut chunks, &[]);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_split_at_midpoint() {
        let data = vec![0u8; 200];
        let mut chunks = vec![Chunk {
            offset: 0,
            length: 200,
            data,
        }];
        force_split(&mut chunks, &[100]);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 100);
        assert_eq!(chunks[1].offset, 100);
        assert_eq!(chunks[1].length, 100);
    }

    #[test]
    fn test_split_multiple() {
        let data = vec![0u8; 500];
        let mut chunks = vec![Chunk {
            offset: 100,
            length: 400,
            data,
        }];
        force_split(&mut chunks, &[200, 300]);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].offset, 100);
        assert_eq!(chunks[0].length, 100);
        assert_eq!(chunks[1].offset, 200);
        assert_eq!(chunks[1].length, 100);
        assert_eq!(chunks[2].offset, 300);
        assert_eq!(chunks[2].length, 200);
    }

    #[test]
    fn test_safetensors_hinter_valid() {
        let json = r#"{"t1":{"dtype":"F32","shape":[4],"data_offsets":[0,16]},"t2":{"dtype":"F32","shape":[4],"data_offsets":[16,32]}}"#;
        let header_size = json.len() as u64;
        let mut data = Vec::new();
        data.extend_from_slice(&header_size.to_le_bytes());
        data.extend_from_slice(json.as_bytes());
        // Pad to 8-byte alignment (standard safetensors spec)
        while data.len() % 8 != 0 {
            data.push(0);
        }
        let data_start = data.len();
        data.extend_from_slice(&[0u8; 32]);
        let hinter = SafetensorsHinter;
        let hints = hinter.hints(&data);
        assert!(!hints.is_empty(), "Should find tensor boundaries");
        // data_offsets are relative to data_start
        assert!(hints.contains(&data_start), "Should hint at first tensor offset");
        assert!(
            hints.contains(&(data_start + 16)),
            "Should hint at second tensor offset"
        );
    }
}
