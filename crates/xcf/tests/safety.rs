mod common;
use photocraft_xcf::*;
#[test]
fn malformed_input_budget_and_cancel_are_errors() {
    let bytes = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], &DecodeOptions::default()).is_err(), "truncation {end}");
    }
    let mut options = DecodeOptions::default();
    options.limits.resident_bytes = bytes.len();
    assert_eq!(decode(&bytes, &options).unwrap_err().kind, ErrorKind::Limit);
    let cancelled = || true;
    options = DecodeOptions::default();
    options.cancelled = Some(&cancelled);
    assert_eq!(decode(&bytes, &options).unwrap_err().kind, ErrorKind::Cancelled);
}
#[test]
fn unsupported_precision_and_nonfinite_fail() {
    assert!(decode(&common::image(12, 999, 0, 1, 1, &[0; 4]), &DecodeOptions::default()).is_err());
    for p in [4, 5, 6] {
        assert!(decode(&common::image(p, 150, 0, 1, 1, &[0; 4]), &DecodeOptions::default()).is_err());
    }
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let bytes = common::image(12, 650, 0, 1, 1, &[invalid, 0.0, 0.0, 1.0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>());
        assert!(decode(&bytes, &DecodeOptions::default()).is_err());
    }
    for invalid in [0x7e00u16, 0x7c00, 0xfc00] {
        let bytes = common::image(12, 550, 2, 1, 1, &[invalid, 0, 0, 0x3c00].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>());
        assert!(decode(&bytes, &DecodeOptions::default()).is_err());
    }
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(256))]
    #[test]
    fn random_bytes_and_mutated_valid_files_fail_safely(data in proptest::collection::vec(proptest::prelude::any::<u8>(),0..4096), at in 0usize..256, value in proptest::prelude::any::<u8>()) {
        let mut options=DecodeOptions::default();
        options.limits.resident_bytes=8<<20;options.limits.input_bytes=8192;
        options.limits.pixels=65536;options.limits.drawables=64;options.limits.visits=10000;
        let _=decode(&data,&options);
        let mut valid=common::image(12,150,2,1,1,&[1,2,3,255]);
        if let Some(byte)=valid.get_mut(at){*byte=value;}
        let _=decode(&valid,&options);
    }
}
#[test]
fn pointer_cycles_and_absurd_dimensions_reject() {
    let mut bytes = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    // Header 14 + canvas12 + precision4 + compression9 + END8.
    bytes[47..55].copy_from_slice(&47u64.to_be_bytes());
    assert!(decode(&bytes, &DecodeOptions::default()).is_err());
    let mut bytes = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    bytes[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut options = DecodeOptions::default();
    options.limits.side = u32::MAX;
    assert_eq!(decode(&bytes, &options).unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn floating_alpha_outside_unit_interval_is_explicit_error() {
    let bytes = common::image(12, 650, 2, 1, 1, &[0.0f32, 0.5, 1.0, 2.0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>());
    let error = decode(&bytes, &Default::default()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert!(error.message.contains("coverage"));
    assert!(!error.item.is_empty());
}

fn pointer(bytes: &[u8], at: usize) -> usize {
    u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
}
fn locations(bytes: &[u8]) -> (usize, usize, usize) {
    let layer = pointer(bytes, 47);
    let name_len = u32::from_be_bytes(bytes[layer + 12..layer + 16].try_into().unwrap()) as usize;
    let hierarchy_slot = layer + 16 + name_len + 8;
    let hierarchy = pointer(bytes, hierarchy_slot);
    let level = pointer(bytes, hierarchy + 12);
    (hierarchy_slot, hierarchy, pointer(bytes, level + 8))
}
#[test]
fn compressed_payloads_reject_short_overlong_and_expansion_bombs() {
    use std::io::Write;
    let rle = common::image(12, 150, 1, 1, 1, &[1, 2, 3, 255]);
    let (_, _, tile) = locations(&rle);
    for payload in [vec![126, 1], vec![127, 0, 0], vec![255], vec![128, 0, 2, 1]] {
        let mut bad = rle[..tile].to_vec();
        bad.extend(payload);
        assert!(matches!(decode(&bad, &Default::default()).unwrap_err().kind, ErrorKind::Compression | ErrorKind::Truncated));
    }
    let zlib = common::image(12, 150, 2, 1, 1, &[1, 2, 3, 255]);
    let (_, _, tile) = locations(&zlib);
    for size in [3, 5, 1 << 20] {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&vec![0; size]).unwrap();
        let mut bad = zlib[..tile].to_vec();
        bad.extend(encoder.finish().unwrap());
        assert_eq!(decode(&bad, &Default::default()).unwrap_err().kind, ErrorKind::Compression);
    }
    for end in tile..zlib.len() {
        assert!(decode(&zlib[..end], &Default::default()).is_err(), "zlib truncated at {end}");
    }
}
#[test]
fn explicit_work_metadata_depth_and_allocation_budget_caps() {
    let valid = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for limits in [
        Limits { visits: 0, ..Default::default() },
        Limits { drawables: 0, ..Default::default() },
        Limits { properties: 0, ..Default::default() },
        Limits { pixels: 0, ..Default::default() },
        Limits { string_bytes: 1, ..Default::default() },
        Limits { property_bytes: 0, ..Default::default() },
        Limits { metadata_bytes: 0, ..Default::default() },
        Limits { resident_bytes: valid.len() + 1, ..Default::default() },
    ] {
        assert_eq!(decode(&valid, &DecodeOptions { limits, ..Default::default() }).unwrap_err().kind, ErrorKind::Limit);
    }
    let options = common::FixtureOptions { layer_properties: vec![(30, vec![0; 12])], ..Default::default() };
    let deep = common::image_custom(12, 150, 0, 1, 1, &[0; 4], &options);
    assert_eq!(
        decode(&deep, &DecodeOptions { limits: Limits { group_depth: 1, ..Default::default() }, ..Default::default() }).unwrap_err().kind,
        ErrorKind::Limit
    );
    let options = common::FixtureOptions { layer_properties: vec![(29, vec![0])], ..Default::default() };
    assert_eq!(decode(&common::image_custom(12, 150, 0, 1, 1, &[0; 4], &options), &Default::default()).unwrap_err().kind, ErrorKind::Structure);
    let mut mask_cycle = valid;
    let (slot, _, _) = locations(&mask_cycle);
    let layer = pointer(&mask_cycle, 47);
    common::patch(&mut mask_cycle, slot + 8, layer, 12);
    assert_eq!(decode(&mask_cycle, &Default::default()).unwrap_err().kind, ErrorKind::Structure);
    let work = common::image(12, 150, 1, 64, 64, &vec![255; 64 * 64 * 4]);
    let error = decode(&work, &DecodeOptions { limits: Limits { visits: 50, ..Default::default() }, ..Default::default() }).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert!(error.message.contains("record/work visit limit"));
}
#[test]
fn cancellation_during_rle_tile_decode_is_polled() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let bytes = common::image(12, 150, 1, 64, 64, &vec![255; 64 * 64 * 4]);
    let checks = AtomicUsize::new(0);
    let drawable_stage = AtomicBool::new(false);
    let progress = |v: f32| {
        if v >= 0.05 {
            drawable_stage.store(true, Ordering::Relaxed);
        }
    };
    let stop = || drawable_stage.load(Ordering::Relaxed) && checks.fetch_add(1, Ordering::Relaxed) >= 50;
    assert_eq!(
        decode(&bytes, &DecodeOptions { cancelled: Some(&stop), progress: Some(&progress), ..Default::default() }).unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(checks.load(Ordering::Relaxed), 51);
}
