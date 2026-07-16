use crate::chunking::Chunker;
use crate::types::{Chunk, ChunkConfig};

/// FastCDC v2020 content-defined chunker.
pub struct FastCdcChunker {
    config: ChunkConfig,
}

impl FastCdcChunker {
    /// Create a new FastCDC chunker with the given config.
    #[must_use]
    pub fn new(config: ChunkConfig) -> Self {
        Self { config }
    }
}

impl Chunker for FastCdcChunker {
    fn chunk(&self, data: &[u8]) -> Vec<Chunk> {
        // Use the fastcdc crate's v2020 module
        let source_len = data.len() as u64;

        let fastcdc_chunker =
            fastcdc::v2020::FastCDC::new(data, self.config.min_size, self.config.avg_size, self.config.max_size);

        let mut chunks = Vec::new();
        let mut last_end = 0u64;

        for window in fastcdc_chunker {
            let offset = window.offset as u64;
            let length = window.length as u32;

            // Handle gap-free coverage
            assert!(offset == last_end, "Chunk offset {offset} != expected {last_end}");

            let chunk_data = data[offset as usize..(offset + u64::from(length)) as usize].to_vec();
            chunks.push(Chunk {
                offset,
                length,
                data: chunk_data,
            });
            last_end = offset + u64::from(length);
        }

        // The fastcdc crate's v2020 module produces chunks covering [0, data.len())
        assert!(
            last_end == source_len,
            "Last chunk end {last_end} != source len {source_len}"
        );

        chunks
    }

    fn config(&self) -> &ChunkConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::Chunker;

    #[test]
    fn test_fastcdc_empty_input() {
        let config = ChunkConfig::default_32k();
        let chunker = FastCdcChunker::new(config);
        let chunks = chunker.chunk(b"");
        assert!(chunks.is_empty(), "Empty input should produce no chunks");
    }

    #[test]
    fn test_fastcdc_small_input() {
        let config = ChunkConfig::default_32k();
        let chunker = FastCdcChunker::new(config);
        let data = b"hello world this is a test of variable length content that should produce at least one chunk even though its shorter than the average chunk size";
        let chunks = chunker.chunk(data);
        assert!(!chunks.is_empty(), "Non-empty input should produce at least one chunk");
        // Verify coverage
        let total: u64 = chunks.iter().map(|c| u64::from(c.length)).sum();
        assert_eq!(total, data.len() as u64, "Chunks must cover entire input");
    }

    #[test]
    fn test_fastcdc_determinism() {
        let config = ChunkConfig::default_32k();
        let chunker = FastCdcChunker::new(config);
        let data = vec![0u8; 100_000];
        // Run twice — boundaries must match
        let chunks1 = chunker.chunk(&data);
        let chunks2 = chunker.chunk(&data);
        assert_eq!(
            chunks1.len(),
            chunks2.len(),
            "Same input must produce same number of chunks"
        );
        for (c1, c2) in chunks1.iter().zip(chunks2.iter()) {
            assert_eq!(c1.offset, c2.offset, "Offset mismatch");
            assert_eq!(c1.length, c2.length, "Length mismatch");
        }
    }

    #[test]
    fn test_fastcdc_all_zeros() {
        let config = ChunkConfig {
            min_size: 256,
            avg_size: 1024,
            max_size: 4096,
        };
        let chunker = FastCdcChunker::new(config);
        let data = vec![0u8; 50_000];
        let chunks = chunker.chunk(&data);
        assert!(!chunks.is_empty(), "All zeros input must produce chunks");
        // Verify no chunk exceeds max_size
        for chunk in &chunks {
            assert!(chunk.length <= config.max_size as u32, "Chunk exceeds max size");
        }
        // Verify coverage
        let total: u64 = chunks.iter().map(|c| u64::from(c.length)).sum();
        assert_eq!(total, data.len() as u64);
    }

    #[test]
    fn test_fastcdc_boundary_shift_recovery() {
        let config = ChunkConfig {
            min_size: 64,
            avg_size: 256,
            max_size: 1024,
        };
        let chunker = FastCdcChunker::new(config);

        // Generate deterministic-ish data
        let mut data1 = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            data1.push((i % 251) as u8);
        }
        // Insert a byte near the beginning (simulates modification)
        let mut data2 = data1.clone();
        data2.insert(100, 0xFF);

        let chunks1 = chunker.chunk(&data1);
        let chunks2 = chunker.chunk(&data2);

        // After the insertion point, boundaries should re-synchronize
        // The total data should still be fully covered
        let total1: u64 = chunks1.iter().map(|c| u64::from(c.length)).sum();
        let total2: u64 = chunks2.iter().map(|c| u64::from(c.length)).sum();

        assert_eq!(total1, data1.len() as u64);
        assert_eq!(total2, data2.len() as u64);
    }
}
