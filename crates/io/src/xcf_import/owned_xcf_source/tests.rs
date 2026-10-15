//! UNRUN selected owned-family custody and default compatibility drafts.
use super::*;
use crate::xcf_fixture;
use photocraft_geom::TileCoord;
use photocraft_raster::{
    backing::quota::{QuotaKind, QuotaLimits},
    checked_view::NativeTileBytes,
};
use std::sync::atomic::{AtomicU64, Ordering};
fn limits() -> ResourceLimits {
    ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("residentBytes", 32 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)]).unwrap()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn raw(precision: u32) -> Arc<Vec<u8>> {
    let pixels = match precision {
        350 => [0x01000001u32, 0x76543210, 0xabcdef01, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect::<Vec<_>>(),
        750 => [-0.0f64, 0.25, 0.75, 1.0].into_iter().flat_map(f64::to_be_bytes).collect(),
        _ => vec![31, 61, 91, 255],
    };
    Arc::new(xcf_fixture::image(13, precision, 0, 1, 1, &pixels))
}
#[test]
fn original_plain_records_select_plain_despite_filename_and_preserve_all_native_padding() {
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let initial = q.usage().unwrap();
    for precision in [150, 350, 750] {
        let bytes = raw(precision);
        let old =
            owned_plain_source::import_owned_staged_plain_bytes_in_quota("old", bytes.clone(), &Default::default(), &limits(), q.clone(), &control()).unwrap();
        let selected = import_owned_xcf_bytes_in_quota("gimp-threshold-alpha.xcf", bytes, &Default::default(), &limits(), q.clone(), &control()).unwrap();
        assert!(matches!(&selected, OwnedXcfImport::Plain(_)));
        let LayerContent::BackedRaster(a) = &old.document().document().layers[0].content else { panic!("oldlayer") };
        let LayerContent::BackedRaster(b) = &selected.document().document().layers[0].content else { panic!("selectedlayer") };
        assert_eq!(a.default_bytes(), b.default_bytes());
        let a_pin = a.checked_view().pin_tile(TileCoord::containing(0, 0), &control()).unwrap();
        let b_pin = b.checked_view().pin_tile(TileCoord::containing(0, 0), &control()).unwrap();
        let NativeTileBytes::FullTile(a) = a_pin.encoded() else { panic!("oldtile") };
        let NativeTileBytes::FullTile(b) = b_pin.encoded() else { panic!("selectedtile") };
        assert_eq!(a, b);
        drop(a_pin);
        drop(b_pin);
        assert_eq!(old.document().document().source_color_context, selected.document().document().source_color_context);
        assert_eq!(old.document().document().working_profile_interpretation, selected.document().document().working_profile_interpretation);
        drop(old);
        drop(selected);
        assert_eq!(q.usage().unwrap(), initial);
    }
    drop(outside);
}
#[test]
fn failed_selection_is_atomic_and_parent_work_is_not_refunded_or_reset() {
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 256).unwrap();
    let before = q.usage().unwrap();
    let bad = Arc::new(b"gimp xcf v022\0".to_vec());
    assert!(import_owned_xcf_bytes_in_quota("looks-plain.xcf", bad, &Default::default(), &limits(), q.clone(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(
        import_owned_xcf_bytes_in_quota("x", raw(150), &Default::default(), &limits(), q.clone(), &ReadControl { interrupt: &Interrupt::NONE, work: &work })
            .is_err()
    );
    assert!(spent.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), before);
    drop(outside);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: requires independently saved pinned Threshold Alpha sources"]
fn actual_threshold_family_matches_typed_promotion_and_plain_route_stays_closed() {
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        let bytes = Arc::new(std::fs::read(root.join(&name)).unwrap());
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let q = quota();
        let baseline = q.usage().unwrap();
        let selected = import_owned_xcf_bytes_in_quota(&name, bytes.clone(), &Default::default(), &limits(), q.clone(), &control()).unwrap();
        assert!(matches!(&selected, OwnedXcfImport::ThresholdAlpha(_)));
        let typed = owned_threshold_source::import_owned_staged_threshold_alpha_bytes_in_quota(
            &name,
            bytes.clone(),
            &Default::default(),
            &limits(),
            q.clone(),
            &control(),
        )
        .unwrap();
        selected.document().validate_readonly_threshold_alpha_storage(&control()).unwrap();
        let a = selected.document().document();
        let b = typed.document().document();
        let ma = &a.xcf_state.as_ref().unwrap().filter_masks[0];
        let mb = &b.xcf_state.as_ref().unwrap().filter_masks[0];
        assert_eq!(ma.surface, mb.surface);
        assert_eq!(ma.bounds, mb.bounds);
        assert_eq!(a.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, b.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
        assert!(owned_plain_source::import_owned_staged_plain_bytes_in_quota(&name, bytes, &Default::default(), &limits(), q.clone(), &control()).is_err());
        drop(typed);
        drop(selected);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
