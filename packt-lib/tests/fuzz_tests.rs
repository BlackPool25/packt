use proptest::prelude::*;

proptest! {
    /// Pack reader must never panic on arbitrary (random) input.
    /// It should return Err gracefully for invalid data.
    #[test]
    fn test_pack_reader_never_panics(data: Vec<u8>) {
        let _ = packt_lib::store::pack::read_pack(&data);
    }

    /// Delta codec must never panic on arbitrary input.
    #[test]
    fn test_delta_codec_never_panics(base: Vec<u8>, delta: Vec<u8>) {
        let encoder = packt_lib::store::delta::DeltaEncoder::new(3);
        // try_encode should never panic
        let _ = encoder.try_encode(&base, &delta);
        // decode should never panic (may return Err or garbage)
        let _ = encoder.decode(&base, &delta, base.len());
    }
}
