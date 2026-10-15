//! UNRUN existing-catalog owned import/atomic failure regression drafts.
use super::*;
use crate::xcf_fixture;
use photocraft_geom::TileCoord;
use photocraft_raster::checked_view::NativeTileBytes;
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use std::sync::atomic::{AtomicU64, Ordering};
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 32 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn raw(precision: u32) -> Arc<Vec<u8>> {
    let bytes = match precision {
        350 => [0x01000001u32, 0x76543210, 0xabcdef01, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect::<Vec<_>>(),
        750 => [-0.0f64, 0.25, 0.75, 1.0].into_iter().flat_map(f64::to_be_bytes).collect(),
        _ => vec![31, 61, 91, 255],
    };
    Arc::new(xcf_fixture::image(12, precision, 0, 1, 1, &bytes))
}
#[test]
fn two_real_imports_share_the_catalog_and_preserve_raw_words_with_no_reset() {
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 4096).unwrap();
    let before = q.usage().unwrap();
    for precision in [350, 750] {
        let bytes = raw(precision);
        let fresh = import_owned_staged_plain_bytes("same", bytes.clone(), &Default::default(), &limits(), &Interrupt::NONE).unwrap();
        let owned = import_owned_staged_plain_bytes_in_quota("same", bytes.clone(), &Default::default(), &limits(), q.clone(), &control()).unwrap();
        assert!(owned.document().quota().shares_ledger(&q));
        assert!(!fresh.document().quota().shares_ledger(&q));
        let LayerContent::BackedRaster(actual) = &owned.document().document().layers[0].content else { panic!("not backed") };
        let LayerContent::BackedRaster(expected) = &fresh.document().document().layers[0].content else { panic!("not backed") };
        let a = actual.checked_view().pin_tile(TileCoord::containing(0, 0), &control()).unwrap();
        let b = expected.checked_view().pin_tile(TileCoord::containing(0, 0), &control()).unwrap();
        let NativeTileBytes::FullTile(a_bytes) = a.encoded() else { panic!("actual tile") };
        let NativeTileBytes::FullTile(b_bytes) = b.encoded() else { panic!("expected tile") };
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(actual.default_bytes(), expected.default_bytes());
        assert_eq!(owned.document().document().source_color_context, fresh.document().document().source_color_context);
        drop(a);
        drop(b);
        drop(owned);
        assert_eq!(q.usage().unwrap(), before);
    }
    let first = import_owned_staged_plain_bytes_in_quota("one", raw(150), &Default::default(), &limits(), q.clone(), &control()).unwrap();
    let first_usage = q.usage().unwrap();
    let second = import_owned_staged_plain_bytes_in_quota("two", raw(150), &Default::default(), &limits(), q.clone(), &control()).unwrap();
    assert!(q.usage().unwrap().resident > first_usage.resident);
    drop(second);
    assert_eq!(q.usage().unwrap(), first_usage);
    drop(first);
    assert_eq!(q.usage().unwrap(), before);
    drop(outside);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn preparation_denial_and_explicit_scratch_policy_precede_decode_work() {
    let small = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 });
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(import_owned_staged_plain_bytes_in_quota("denied", raw(150), &Default::default(), &limits(), small.clone(), &c).is_err());
    assert_eq!(spent.load(Ordering::Relaxed), 1);
    assert_eq!(small.usage().unwrap(), QuotaUsage::default());
    let q = quota();
    let standard = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    assert!(import_owned_staged_plain_bytes_in_quota("scratch", raw(150), &Default::default(), &standard, q.clone(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn actual_parent_work_denial_is_nonrefundable_while_all_owned_storage_refunds() {
    let q = quota();
    let outside = q.reserve(QuotaKind::Index, 512).unwrap();
    let before = q.usage().unwrap();
    let spent = AtomicU64::new(0);
    let work = |n| {
        if n > 256 {
            return Err(BackingError::Work);
        }
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(import_owned_staged_plain_bytes_in_quota("work", raw(350), &Default::default(), &limits(), q.clone(), &c).is_err());
    assert!(spent.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), before);
    drop(outside);
}
#[test]
fn malformed_and_cancelled_source_leave_existing_catalog_owners_unchanged() {
    let q = quota();
    let held = q.reserve(QuotaKind::Resident, 512).unwrap();
    let before = q.usage().unwrap();
    assert!(import_owned_staged_plain_bytes_in_quota("bad", Arc::new(b"malformed".to_vec()), &Default::default(), &limits(), q.clone(), &control()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(import_owned_staged_plain_bytes_in_quota("cancel", raw(150), &Default::default(), &limits(), q.clone(), &c), Err(IoError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    drop(held);
}
