use photocraft_io::xcf_policy;
use photocraft_xcf::{BudgetKind, ErrorKind, ResourceLimits, ResourcePolicy};
#[path = "common/xcf.rs"]
mod common;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn measured_import_uses_the_shared_plain_and_wrapper_transaction() {
    use std::io::Write;
    let input = common::image(12, 150, 0, 1, 1, &[12, 34, 56, 255]);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&input).unwrap();
    let wrapped = encoder.finish().unwrap();
    for bytes in [&input, &wrapped] {
        let (measured, stats) = photocraft_io::xcf_import::import_with_stats("measured.xcf", bytes, &photocraft_raster::Interrupt::NONE).unwrap();
        let ordinary = photocraft_io::xcf_import::import("measured.xcf", bytes, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(measured.document.layers[0].surface(), ordinary.document.layers[0].surface());
        assert_eq!(measured.document.xcf_state.as_ref().unwrap().provenance, ordinary.document.xcf_state.as_ref().unwrap().provenance);
        assert!(stats.peak_charged_bytes >= bytes.len() + (256 * 256 * 4));
        assert!(!stats.decode.is_zero());
        assert!(!stats.mapping.is_zero());
    }
}

#[test]
fn public_import_rejects_raised_policy_before_reading_invalid_input() {
    let limits = photocraft_xcf::Limits { resident_bytes: usize::MAX, ..Default::default() };
    let error = photocraft_io::xcf_import::import_with_limits("invalid.xcf", b"invalid", &photocraft_raster::Interrupt::NONE, limits).unwrap_err();
    assert!(matches!(error, photocraft_io::IoError::Xcf(ref e) if e.kind == ErrorKind::Limit), "{error}");
}

#[test]
fn public_import_uses_one_policy_for_plain_and_wrapped_inputs() {
    use std::io::Write;
    let input = common::image(12, 150, 0, 1, 1, &[12, 34, 56, 255]);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&input).unwrap();
    let wrapped = encoder.finish().unwrap();
    let native = photocraft_io::xcf_import::import("plain.xcf", &input, &photocraft_raster::Interrupt::NONE).unwrap();
    let compressed = photocraft_io::xcf_import::import("wrapped.xcf.gz", &wrapped, &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(native.document.layers.len(), compressed.document.layers.len());
    assert_eq!(native.document.layers[0].surface(), compressed.document.layers[0].surface());
    assert_eq!(compressed.document.xcf_state.as_ref().unwrap().provenance.container, "gzip");
    for source in [&input, &wrapped] {
        let zero = photocraft_xcf::Limits { input_bytes: 0, ..Default::default() };
        assert!(
            matches!(photocraft_io::xcf_import::import_with_limits("zero.xcf", source, &photocraft_raster::Interrupt::NONE, zero), Err(photocraft_io::IoError::Xcf(ref e)) if e.kind == ErrorKind::Limit)
        );
    }
}

#[test]
fn native_and_wasm_defaults_and_component_reductions_are_preserved() {
    for policy in [ResourcePolicy::default(), ResourcePolicy::WasmStandard] {
        let base = ResourceLimits::for_policy(policy).unwrap().lowered(&[("decoderBytes", 19), ("sampleWorkUnits", 29), ("deadlineMs", 39)]).unwrap();
        let mapped = xcf_policy::for_resources(&base).unwrap();
        assert_eq!(mapped.resources.policy(), policy);
        assert_eq!(mapped.parser.input_bytes as u64, base.get(BudgetKind::Encoded).min(base.get(BudgetKind::Logical)));
        assert_eq!(mapped.parser.resident_bytes as u64, base.get(BudgetKind::Resident));
        for kind in [BudgetKind::Decoder, BudgetKind::Work, BudgetKind::Deadline] {
            assert_eq!(mapped.resources.get(kind), base.get(kind));
        }
    }
    assert!(ResourceLimits::for_policy(ResourcePolicy::NativeLarge).is_err());
}
#[test]
fn every_legacy_ceiling_lowers_the_matching_shared_budget_and_zero_is_real() {
    let base = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let mut parser = xcf_policy::for_resources(&base).unwrap().parser;
    parser.input_bytes = 0;
    parser.resident_bytes = 1;
    parser.side = 2;
    parser.pixels = 3;
    parser.drawables = 4;
    parser.properties = 5;
    parser.string_bytes = 6;
    parser.profile_bytes = 7;
    parser.property_bytes = 8;
    parser.metadata_bytes = 9;
    parser.visits = 10;
    parser.group_depth = 11;
    let mapped = xcf_policy::from_parser_limits(parser, &base).unwrap();
    for (kind, expected) in [
        (BudgetKind::Encoded, 0),
        (BudgetKind::Logical, 0),
        (BudgetKind::Resident, 1),
        (BudgetKind::Side, 2),
        (BudgetKind::Pixels, 3),
        (BudgetKind::Drawables, 4),
        (BudgetKind::Properties, 5),
        (BudgetKind::String, 6),
        (BudgetKind::Profile, 7),
        (BudgetKind::Property, 8),
        (BudgetKind::Metadata, 9),
        (BudgetKind::Visits, 10),
        (BudgetKind::GroupDepth, 11),
    ] {
        assert_eq!(mapped.resources.get(kind), expected, "{}", kind.name());
    }
    assert_eq!(mapped.parser.input_bytes, 0);
}
#[test]
fn attempted_raises_and_address_extremes_fail_before_io() {
    let base = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    for field in 0..12 {
        let mut parser = xcf_policy::for_resources(&base).unwrap().parser;
        match field {
            0 => parser.input_bytes = usize::MAX,
            1 => parser.resident_bytes = usize::MAX,
            2 => parser.side = u32::MAX,
            3 => parser.pixels = u64::MAX,
            4 => parser.drawables = usize::MAX,
            5 => parser.properties = usize::MAX,
            6 => parser.string_bytes = usize::MAX,
            7 => parser.profile_bytes = usize::MAX,
            8 => parser.property_bytes = usize::MAX,
            9 => parser.metadata_bytes = usize::MAX,
            10 => parser.visits = usize::MAX,
            _ => parser.group_depth = usize::MAX,
        }
        assert_eq!(xcf_policy::from_parser_limits(parser, &base).unwrap_err().kind, ErrorKind::Limit);
    }
    let lower = base.lowered(&[("logicalBytes", 10)]).unwrap();
    let mapped = xcf_policy::for_resources(&lower).unwrap();
    assert_eq!(mapped.parser.input_bytes, 10);
    assert_eq!(mapped.resources.get(BudgetKind::Encoded), 10);
    let mut raised = mapped.parser;
    raised.input_bytes = 11;
    assert!(xcf_policy::from_parser_limits(raised, &lower).is_err());
}
