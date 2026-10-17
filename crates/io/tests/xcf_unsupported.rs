#[path = "common/xcf.rs"]
mod common;
use photocraft_raster::Interrupt;
fn one(properties: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
    let options = common::FixtureOptions { layer_properties: properties, ..Default::default() };
    common::image_custom(12, 150, 1, 1, 1, &[10, 20, 30, 255], &options)
}
#[test]
fn hidden_blend_effect_link_text_float_and_unknown_are_named_errors() {
    for (tag, payload, expected) in [
        (7, 30u32.to_be_bytes().to_vec(), "blend mode"),
        (5, 0u64.to_be_bytes().to_vec(), "floating selection"),
        (26, 0u32.to_be_bytes().to_vec(), "GIMP text"),
        (47, vec![0; 8], "vector"),
        (48, vec![0; 8], "linked"),
        (35, 2u32.to_be_bytes().to_vec(), "composite mode"),
        (36, 3u32.to_be_bytes().to_vec(), "composite space"),
        (999, vec![], "unknown layer property"),
    ] {
        let bytes = one(vec![(8, 0u32.to_be_bytes().to_vec()), (tag, payload)]);
        let error = photocraft_io::import("untrusted.xcf", &bytes).unwrap_err().to_string();
        assert!(error.contains("Layer"), "{error}");
        assert!(error.contains(expected), "{error}");
    }
    // Unverified text never acquires a silently flattened stored-raster fallback.
    assert!(photocraft_io::import("text.xcf", &one(vec![(26, 0u32.to_be_bytes().to_vec())])).is_err());
}
#[test]
fn common_mapping_budget_and_cancel_fail_before_commit() {
    let bytes = one(vec![]);
    let limits = photocraft_xcf::Limits { resident_bytes: 100_000, ..Default::default() };
    let error = photocraft_io::xcf_import::import_with_limits("tight.xcf", &bytes, &Interrupt::NONE, limits).unwrap_err().to_string();
    assert!(error.contains("budget"));
    let stop = || true;
    let ctl = Interrupt::cancel_only(&stop);
    assert!(matches!(photocraft_io::xcf_import::import("cancel.xcf", &bytes, &ctl), Err(photocraft_io::IoError::Cancelled)));
}
#[test]
fn invalid_icc_nonfinite_opacity_resolution_and_palette_reject() {
    for props in [
        vec![(33, f32::NAN.to_be_bytes().to_vec())],
        vec![(33, 2.0f32.to_be_bytes().to_vec())],
        vec![(15, [i32::MAX, 0].iter().flat_map(|n| n.to_be_bytes()).collect())],
    ] {
        assert!(photocraft_io::import("bad.xcf", &one(props)).is_err());
    }
    let mut parasite = Vec::new();
    common::u32be(&mut parasite, 12);
    parasite.extend(b"icc-profile\0");
    common::u32be(&mut parasite, 0);
    common::u32be(&mut parasite, 3);
    parasite.extend(b"bad");
    let options = common::FixtureOptions { image_properties: vec![(21, parasite)], ..Default::default() };
    let bytes = common::image_custom(12, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    assert!(photocraft_io::import("profile.xcf", &bytes).unwrap_err().to_string().contains("profile"));
}

#[test]
fn malformed_group_membership_and_mask_dimensions_reject_atomically() {
    assert!(photocraft_io::import("orphan.xcf", &one(vec![(30, [0u32, 0].into_iter().flat_map(u32::to_be_bytes).collect())])).is_err());
    let mut bytes = one(vec![]);
    let layer = u64::from_be_bytes(bytes[47..55].try_into().unwrap()) as usize;
    let mask_slot = layer + 16 + 6 + 8 + 8;
    let mask = bytes.len();
    common::patch(&mut bytes, mask_slot, mask, 12);
    common::u32be(&mut bytes, 2);
    common::u32be(&mut bytes, 1);
    common::u32be(&mut bytes, 5);
    bytes.extend(b"Mask\0");
    common::prop(&mut bytes, 0, &[]);
    let hierarchy_slot = bytes.len();
    common::pointer(&mut bytes, 0, 12);
    let hierarchy = bytes.len();
    common::patch(&mut bytes, hierarchy_slot, hierarchy, 12);
    common::u32be(&mut bytes, 2);
    common::u32be(&mut bytes, 1);
    common::u32be(&mut bytes, 1);
    let level_slot = bytes.len();
    common::pointer(&mut bytes, 0, 12);
    common::pointer(&mut bytes, 0, 12);
    let level = bytes.len();
    common::patch(&mut bytes, level_slot, level, 12);
    common::u32be(&mut bytes, 2);
    common::u32be(&mut bytes, 1);
    let tile_slot = bytes.len();
    common::pointer(&mut bytes, 0, 12);
    common::pointer(&mut bytes, 0, 12);
    let tile = bytes.len();
    common::patch(&mut bytes, tile_slot, tile, 12);
    // Source fixture uses RLE; two repeated mask pixels.
    bytes.extend([1, 255]);
    let error = photocraft_io::import("mask.xcf", &bytes).unwrap_err().to_string();
    assert!(error.contains("unequal layer-mask geometry"), "{error}");
}
#[test]
fn cancellation_after_decode_during_mapping_returns_no_document() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let options = common::FixtureOptions { layer_type: Some(0), ..Default::default() };
    let bytes = common::image_custom(12, 150, 0, 512, 512, &vec![7; 512 * 512 * 3], &options);
    let mapping = AtomicBool::new(false);
    let checks = AtomicUsize::new(0);
    let progress = |v: f32| {
        if v >= 0.7 {
            mapping.store(true, Ordering::Relaxed);
        }
    };
    let stop = || mapping.load(Ordering::Relaxed) && checks.fetch_add(1, Ordering::Relaxed) >= 10;
    assert!(matches!(photocraft_io::xcf_import::import("mid-map.xcf", &bytes, &Interrupt::new(&stop, &progress)), Err(photocraft_io::IoError::Cancelled)));
    assert!(mapping.load(Ordering::Relaxed));
    assert_eq!(checks.load(Ordering::Relaxed), 11);
}
