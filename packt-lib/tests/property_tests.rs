use proptest::prelude::*;

proptest! {
    /// Delta roundtrip: decode(encode(base, modified)) should equal modified
    /// whenever delta encoding was beneficial.
    #[test]
    fn test_delta_roundtrip(base: Vec<u8>, modified: Vec<u8>) {
        // Constrain sizes to avoid pathological cases
        let base = if base.len() < 256 {
            let mut v = base;
            v.resize(256, 0);
            v
        } else {
            base
        };
        let modified = if modified.len() < 256 {
            let mut v = modified;
            v.resize(256, 0);
            v
        } else {
            modified
        };

        // Ensure size ratio is within bounds
        let (larger, smaller) = if base.len() >= modified.len() {
            (base.len(), modified.len())
        } else {
            (modified.len(), base.len())
        };
        if larger > smaller * 4 || smaller == 0 {
            return Ok(());
        }

        let encoder = packt_lib::store::delta::DeltaEncoder::new(3);
        if let Some(delta) = encoder.try_encode(&base, &modified).unwrap() {
            let decoded = encoder.decode(&base, &delta, modified.len()).unwrap();
            prop_assert_eq!(decoded, modified, "Delta roundtrip failed");
        }
    }

    /// CDC determinism: same data must produce identical chunk boundaries.
    #[test]
    fn test_chunking_determinism(data: Vec<u8>) {
        use packt_lib::chunking::fastcdc::FastCdcChunker;
        use packt_lib::chunking::Chunker;
        use packt_lib::types::ChunkConfig;

        let config = ChunkConfig::default_32k();
        let chunker = FastCdcChunker::new(config);
        let chunks1 = chunker.chunk(&data);
        let chunks2 = chunker.chunk(&data);

        prop_assert_eq!(chunks1.len(), chunks2.len());
        for (a, b) in chunks1.iter().zip(chunks2.iter()) {
            prop_assert_eq!(a.offset, b.offset);
            prop_assert_eq!(a.length, b.length);
            prop_assert!(a.data == b.data);
        }
    }

    /// Pack roundtrip: write_pack → read_pack preserves entries.
    #[test]
    fn test_pack_roundtrip(entries: Vec<(Vec<u8>, bool)>) {
        use packt_lib::store::pack::{EntryType, PackEntry, write_pack, read_pack};
        use packt_lib::types::Hash;

        // Limit to 20 entries to keep test fast
        let entries: Vec<_> = entries.into_iter().take(20).collect();
        if entries.is_empty() {
            return Ok(());
        }

        let pack_entries: Vec<PackEntry> = entries
            .into_iter()
            .filter(|(data, _)| !data.is_empty())
            .map(|(data, is_delta)| {
                let orig_len = data.len() as u32;
                let hash = Hash::from_blake3(blake3::hash(&data));
                PackEntry {
                    hash,
                    data,
                    orig_length: orig_len,
                    entry_type: if is_delta {
                        // For delta we need a valid base hash; use self as base
                        EntryType::Delta { base_hash: hash }
                    } else {
                        EntryType::Full
                    },
                    signature: None,
                }
            })
            .collect();

        let pack = write_pack(&pack_entries).unwrap();
        let (read_entries, _checksum, _superblock) = read_pack(&pack).unwrap();

        // write_pack reorders entries: Full first, then Delta.
        // Compare by hash set membership instead of position.
        let original_hashes: std::collections::HashSet<_> =
            pack_entries.iter().map(|e| e.hash).collect();
        let read_hashes: std::collections::HashSet<_> =
            read_entries.iter().map(|e| e.hash).collect();
        prop_assert_eq!(original_hashes, read_hashes);
    }
}
