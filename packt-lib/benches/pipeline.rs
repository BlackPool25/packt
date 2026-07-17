use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use packt_lib::chunking::Chunker;
use packt_lib::chunking::fastcdc::FastCdcChunker;
use packt_lib::hash::ContentHasher;
use packt_lib::hash::blake3_hasher::Blake3Hasher;
use packt_lib::store::pack::{EntryType, PackEntry, read_pack, write_pack};
use packt_lib::types::{ChunkConfig, Hash};

/// Benchmark FastCDC chunking throughput on 64 MB of zeroed data at default
/// 32 KB average chunk size.
fn chunking_throughput(c: &mut Criterion) {
    let data = vec![0u8; 64_000_000];
    let config = ChunkConfig::default_32k();
    let chunker = FastCdcChunker::new(config);

    let mut group = c.benchmark_group("chunking");
    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("fastcdc_32k", |b| {
        b.iter(|| {
            let chunks = chunker.chunk(black_box(&data));
            black_box(chunks.len());
        });
    });
    group.finish();
}

/// Benchmark BLAKE3 hashing throughput on three data sizes: 1 KB, 32 KB, and
/// 1 MB.  Each size gets its own sub-benchmark with the appropriate
/// `Throughput` so Criterion reports MiB/s.
fn hashing_throughput(c: &mut Criterion) {
    let hasher = Blake3Hasher::new();
    let sizes: &[usize] = &[1024, 32_768, 1_048_576];

    let mut group = c.benchmark_group("hashing");
    for &size in sizes {
        let data = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_function(format!("blake3_{size}"), |b| {
            b.iter(|| {
                let hash = hasher.hash(black_box(&data));
                black_box(hash);
            });
        });
    }
    group.finish();
}

/// Benchmark pack round-trip: serialise 1000 synthetic chunks into a pack,
/// then deserialise and verify the index.
fn pack_roundtrip(c: &mut Criterion) {
    let chunks: Vec<PackEntry> = (0..1000)
        .map(|i| {
            let data = format!("benchmark pack chunk {i} with some extra data for realistic sizing");
            let len = data.len() as u32;
            let data_bytes = data.into_bytes();
            let hash = Hash::from_blake3(blake3::hash(&data_bytes));
            PackEntry {
                hash,
                data: data_bytes,
                orig_length: len,
                entry_type: EntryType::Full,
                signature: None,
            }
        })
        .collect();

    let mut group = c.benchmark_group("pack");
    group.throughput(Throughput::Elements(chunks.len() as u64));
    group.bench_function("roundtrip_1000", |b| {
        b.iter(|| {
            let pack = write_pack(black_box(&chunks)).unwrap();
            let (entries, _, _) = read_pack(black_box(&pack)).unwrap();
            black_box(entries.len());
        });
    });
    group.finish();
}

criterion_group!(benches, chunking_throughput, hashing_throughput, pack_roundtrip);
criterion_main!(benches);
