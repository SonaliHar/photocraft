//! Genuine +1 original packets -> derived CURRENT opacity; no partial-opacity producer source claim.
#![allow(dead_code, unused_imports)]
#[path = "accounted_exposure_loaded_test_support.rs"]
mod loaded;
#[path = "accounted_filtered_flat_invert_test_support.rs"]
mod originals;
pub use loaded::mixed_current as loaded_mixed_current;
pub use originals::{CASES, Case, archive, bytes, ctl, current, import, observed, payload, same, storage, workspace};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::sync::Arc;
pub fn ordinary_point() -> photocraft_io::OwnedXcfImport {
    loaded::import(&loaded::CASES[0])
}
/// External malformed encoded buffer: verifies SOURCE remains +1, not a GIMP partial witness.
pub fn partial_source(case: &Case) -> Vec<u8> {
    let mut encoded = bytes(case);
    let capture: serde_json::Value = serde_json::from_slice(&std::fs::read(workspace().join(case.capture)).unwrap()).unwrap();
    let property = capture["raw"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["effects"].as_array().unwrap())
        .flat_map(|e| e["properties"].as_array().unwrap())
        .find(|p| p["tag"] == 33)
        .unwrap();
    let at = property["offset"].as_u64().unwrap() as usize;
    assert_eq!(&encoded[at..at + 4], &1f32.to_bits().to_be_bytes());
    encoded[at..at + 4].copy_from_slice(&0.5f32.to_bits().to_be_bytes());
    encoded
}
pub fn rejects_source(case: &Case) {
    let encoded = partial_source(case);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
    assert!(
        photocraft_io::import_owned_xcf_bytes_in_quota(
            "nonunit-source.xcf",
            Arc::new(encoded),
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
            q,
            &ctl()
        )
        .is_err()
    );
}
