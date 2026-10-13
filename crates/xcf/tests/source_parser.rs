mod common;
use photocraft_xcf::*;
struct Generated {
    prefix: Vec<u8>,
    length: u64,
    reads: usize,
    largest: usize,
}
impl Source for Generated {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_exact_at(&mut self, at: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        budget.poll("generated read")?;
        assert!(out.len() <= 65536, "source-sized read is forbidden");
        assert!(at.checked_add(out.len() as u64).unwrap() <= self.length);
        self.reads += 1;
        self.largest = self.largest.max(out.len());
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.prefix.get((at + i as u64) as usize).copied().unwrap_or(0);
        }
        Ok(())
    }
}
#[test]
fn immutable_scratch_source_parses_without_encoded_resident_vec() {
    let prefix = common::image(22, 150, 0, 1, 1, &[7, 8, 9, 255]);
    let mut baseline = prefix.clone();
    baseline.resize(2 << 20, 0);
    let expected = decode(&baseline, &DecodeOptions::default()).unwrap();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", 128 << 10)]).unwrap();
    let budget = Budget::new(limits, None);
    let encoded = budget.reserve(BudgetKind::Encoded, baseline.len() as u64, "owned encoded").unwrap();
    let scratch = budget.reserve(BudgetKind::Scratch, baseline.len() as u64, "owned scratch").unwrap();
    let mut source = Generated { prefix, length: baseline.len() as u64, reads: 0, largest: 0 };
    let actual = decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::CallerReservedScratch).unwrap();
    assert_eq!(actual.decoded.file.layers[0].plane.as_ref().unwrap().bytes, expected.layers[0].plane.as_ref().unwrap().bytes);
    assert_eq!(actual.decoded.file.raw_properties[0].payload, expected.raw_properties[0].payload);
    assert!(source.reads > 0);
    assert!(source.largest <= 65536);
    assert!(budget.peak(BudgetKind::Resident) < 128 << 10);
    drop(actual);
    drop(encoded);
    drop(scratch);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

fn source_decode(bytes: &[u8], overrides: &[(&str, u64)]) -> Result<XcfFile, XcfError> {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(overrides).unwrap();
    let budget = Budget::new(limits, None);
    let mut source = ByteSource::new(bytes);
    let result = decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory);
    match result {
        Ok(result) => Ok(result.decoded.file),
        Err(e) => {
            assert_eq!(budget.used(BudgetKind::Resident), 0);
            assert_eq!(budget.used(BudgetKind::Encoded), 0);
            Err(e)
        }
    }
}
#[test]
fn byte_source_matches_slice_native_planes_all_precisions_and_compressions() {
    let cases: Vec<(u32, Vec<u8>)> = vec![
        (150, vec![7, 8, 9, 255]),
        (250, [123u16, 124, 125, 65535].into_iter().flat_map(u16::to_be_bytes).collect()),
        (350, [1u32, 2, 3, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect()),
        (550, [0x3400u16, 0x3800, 0x3a00, 0x3c00].into_iter().flat_map(u16::to_be_bytes).collect()),
        (650, [0.25f32, 0.5, 0.75, 1.0].into_iter().flat_map(|v| v.to_bits().to_be_bytes()).collect()),
        (750, [f64::from_bits(0x3fd0000000000001), -0.0, 0.75, 1.0].into_iter().flat_map(|v| v.to_bits().to_be_bytes()).collect()),
    ];
    for (precision, pixels) in cases {
        for compression in 0..=2 {
            let bytes = common::image(22, precision, compression, 1, 1, &pixels);
            let expected = decode(&bytes, &DecodeOptions::default()).unwrap();
            let actual = source_decode(&bytes, &[]).unwrap();
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "precision{precision} compression{compression}");
        }
    }
}
#[test]
fn missing_or_wrong_owned_source_reservations_fail_before_reads() {
    let prefix = common::image(22, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for kind in [PlainSourceAccounting::CallerReservedMemory, PlainSourceAccounting::CallerReservedScratch] {
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
        let mut source = Generated { length: prefix.len() as u64, prefix: prefix.clone(), reads: 0, largest: 0 };
        assert_eq!(decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, kind).unwrap_err().kind, ErrorKind::Limit);
        let encoded = budget.reserve(BudgetKind::Encoded, source.len(), "encoded").unwrap();
        assert!(decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, kind).is_err());
        assert_eq!(source.reads, 0);
        drop(encoded);
    }
}
#[test]
fn scratch_and_memory_accounting_are_distinct_and_zero_limits_are_real() {
    let prefix = common::image(22, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let mut source = Generated { length: 2 << 20, prefix, reads: 0, largest: 0 };
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", 128 << 10)]).unwrap(), None);
    assert_eq!(
        decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory).unwrap_err().kind,
        ErrorKind::Limit
    );
    assert_eq!(source.reads, 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    for key in ["encodedBytes", "logicalBytes", "residentBytes", "sampleWorkUnits"] {
        let bytes = common::image(22, 150, 0, 1, 1, &[1, 2, 3, 255]);
        assert_eq!(source_decode(&bytes, &[(key, 0)]).unwrap_err().kind, ErrorKind::Limit);
    }
}
struct Failing {
    bytes: Vec<u8>,
    fail_at: u64,
    reads: usize,
}
impl Source for Failing {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_exact_at(&mut self, at: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        self.reads += 1;
        if at >= self.fail_at {
            return Err(XcfError { kind: ErrorKind::Truncated, offset: at, item: String::new(), message: "injected late read failure".into() });
        }
        // Source trait promises exact reads: simulate an adapter making tiny underlying reads.
        for (i, chunk) in out.chunks_mut(3).enumerate() {
            let local = at + i as u64 * 3;
            ByteSource::new(&self.bytes).read_exact_at(local, chunk, budget)?;
        }
        Ok(())
    }
}
#[test]
fn late_tile_read_error_propagates_and_refunds_parsed_and_source_storage() {
    let bytes = common::image(22, 750, 0, 1, 1, &[0.5f64, 0.0, 0.75, 1.0].into_iter().flat_map(|v| v.to_bits().to_be_bytes()).collect::<Vec<_>>());
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let mut source = Failing { fail_at: bytes.len() as u64 - 32, bytes, reads: 0 };
    let error = decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Truncated);
    assert_eq!(error.offset, source.fail_at);
    assert!(source.reads > 10);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}
#[test]
fn bounded_large_string_buffer_and_retained_metadata_parity() {
    let options = common::FixtureOptions { name: Some("x".repeat(65536)), ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let expected = decode(&bytes, &DecodeOptions::default()).unwrap();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let budget = Budget::new(limits, None);
    let mut source = Generated { length: bytes.len() as u64, prefix: bytes, reads: 0, largest: 0 };
    let result = decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory).unwrap();
    assert_eq!(result.decoded.file.layers[0].name, expected.layers[0].name);
    assert!(source.largest <= 4096);
    let peak = budget.peak(BudgetKind::Resident);
    drop(result);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", peak - 1)]).unwrap(), None);
    assert_eq!(
        decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory).unwrap_err().kind,
        ErrorKind::Limit
    );
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}
#[test]
fn u64_pointer_overflow_and_wrapped_source_are_not_materialized() {
    let mut bytes = common::image(22, 150, 0, 1, 1, &[1, 2, 3, 255]);
    // First layer pointer follows header+compression/end property at offset47.
    let pointer = 14 + 16 + 9 + 8;
    bytes[pointer..pointer + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    let error = source_decode(&bytes, &[]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Structure);
    let error = source_decode(&[31, 139, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Header);
}
#[test]
fn shared_cancellation_during_record_reads_discards_whole_result() {
    use std::cell::Cell;
    let count = Cell::new(0usize);
    let cancelled = || {
        count.set(count.get() + 1);
        count.get() > 25
    };
    let bytes = common::image(22, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), Some(&cancelled));
    let mut source = ByteSource::new(&bytes);
    let error = decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}

#[test]
fn owned_text_parasite_and_opaque_payload_match_slice_and_survive_source_drop() {
    let config = b"(text \"Independent \\316\\251\") (font-size 17.125) (hinting yes)\0";
    let mut parasite = Vec::new();
    common::u32be(&mut parasite, 16);
    parasite.extend(b"gimp-text-layer\0");
    common::u32be(&mut parasite, 1);
    common::u32be(&mut parasite, config.len() as u32);
    parasite.extend(config);
    let options = common::FixtureOptions { image_properties: vec![(999, vec![31; 8201])], layer_properties: vec![(21, parasite)], ..Default::default() };
    let bytes = common::image_custom(22, 150, 1, 1, 1, &[7, 8, 9, 255], &options);
    let expected = decode(&bytes, &DecodeOptions::default()).unwrap();
    let actual = source_decode(&bytes, &[]).unwrap();
    drop(bytes);
    assert_eq!(format!("{:?}", actual.layers[0].text.as_ref().unwrap()), format!("{:?}", expected.layers[0].text.as_ref().unwrap()));
    assert_eq!(actual.raw_properties[1].payload, expected.raw_properties[1].payload);
    assert_eq!(actual.layers[0].raw_properties[0].payload, expected.layers[0].raw_properties[0].payload);
}
#[test]
fn encoded_logical_exact_and_one_under_limits_have_no_materialization_fallback() {
    let bytes = common::image(22, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for key in ["encodedBytes", "logicalBytes"] {
        assert!(source_decode(&bytes, &[(key, bytes.len() as u64)]).is_ok());
        assert_eq!(source_decode(&bytes, &[(key, bytes.len() as u64 - 1)]).unwrap_err().kind, ErrorKind::Limit);
    }
}

#[test]
fn failure_inside_owned_string_window_releases_live_temporary_capacity() {
    let options = common::FixtureOptions { name: Some("n".repeat(20000)), ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let mut source = Failing { bytes, fail_at: 8192, reads: 0 };
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let error = decode_plain_source_with_budget(&mut source, &DecodeOptions::default(), &budget, PlainSourceAccounting::BorrowedMemory).unwrap_err();
    assert_eq!(error.message, "injected late read failure");
    assert!(budget.peak(BudgetKind::Resident) > 40000);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}
#[test]
fn options_only_cancel_is_polled_inside_large_source_windows() {
    use std::cell::Cell;
    let count = Cell::new(0usize);
    let cancelled = || {
        count.set(count.get() + 1);
        count.get() > 35
    };
    let options = common::FixtureOptions { name: Some("n".repeat(20000)), ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let mut source = ByteSource::new(&bytes);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let decode_options = DecodeOptions { cancelled: Some(&cancelled), ..Default::default() };
    assert_eq!(
        decode_plain_source_with_budget(&mut source, &decode_options, &budget, PlainSourceAccounting::BorrowedMemory).unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}
#[test]
fn multi_tile_raw_rle_zlib_sources_match_partial_edge_tiles() {
    let pixels: Vec<u8> = (0..128 * 65).flat_map(|i| [(i % 256) as u8, ((i / 7) % 256) as u8, 91, 255]).collect();
    for compression in 0..=2 {
        let bytes = common::image(22, 150, compression, 128, 65, &pixels);
        let actual = source_decode(&bytes, &[]).unwrap();
        assert_eq!(actual.layers[0].plane.as_ref().unwrap().bytes, pixels);
    }
}
