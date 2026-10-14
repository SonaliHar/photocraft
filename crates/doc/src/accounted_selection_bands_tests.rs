//! UNRUN band-owner lifecycle/capacity drafts. No source/import qualification.
use super::*;
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 8 << 20, scratch: 4 << 20 })
}
#[test]
fn ordered_irregular_bands_preserve_native_words_and_zero_padded_tail() {
    let quota = quota();
    let initial = quota.usage().unwrap();
    let canvas = Rect::new(0, 0, 257, 259);
    let mut builder = AccountedSelectionBandBuilder::new(canvas, &quota, &control()).unwrap();
    for (first, last) in [(0, 73), (73, 256), (256, 259)] {
        let raw: Vec<u8> = (first..last).flat_map(|y| (0..257).map(move |x| ((x + 3 * y) % 255 + 1) as u8)).collect();
        builder.push_band(Rect::new(0, first, 257, last), &raw, &control()).unwrap();
    }
    let selection = builder.finish(&control()).unwrap().unwrap();
    assert_eq!(selection.format(), PixelFormat::GRAY8);
    assert_eq!(selection.bounds(), canvas);
    let output = selection.surface().try_read_interleaved(canvas).unwrap();
    let expected: Vec<u8> = (0..259).flat_map(|y| (0..257).map(move |x| ((x + 3 * y) % 255 + 1) as u8)).collect();
    assert_eq!(output, expected);
    for (x, y) in [(257, 0), (0, 259), (511, 511)] {
        let mut raw = [1];
        selection.surface().try_read_pixel_bytes(x, y, &mut raw).unwrap();
        assert_eq!(raw, [0]);
    }
    assert!(quota.usage().unwrap().resident > initial.resident);
    drop(selection);
    assert_eq!(quota.usage().unwrap(), initial);
}
#[test]
fn incomplete_duplicate_outoforder_and_wrong_length_never_finish_an_owner() {
    let q = quota();
    let before = q.usage().unwrap();
    let incomplete = AccountedSelectionBandBuilder::new(Rect::new(0, 0, 2, 2), &q, &control()).unwrap();
    assert!(incomplete.finish(&control()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    for invalid in [Rect::new(0, 0, 2, 1), Rect::new(0, 2, 2, 3), Rect::new(1, 1, 2, 2)] {
        let mut b = AccountedSelectionBandBuilder::new(Rect::new(0, 0, 2, 2), &q, &control()).unwrap();
        b.push_band(Rect::new(0, 0, 2, 1), &[1, 2], &control()).unwrap();
        assert!(b.push_band(invalid, &[1, 2], &control()).is_err());
        assert!(b.finish(&control()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let mut b = AccountedSelectionBandBuilder::new(Rect::new(0, 0, 2, 1), &q, &control()).unwrap();
    assert!(b.push_band(Rect::new(0, 0, 2, 1), &[1], &control()).is_err());
    assert!(b.push_band(Rect::new(0, 0, 2, 1), &[1, 2], &control()).is_err());
    assert!(b.finish(&control()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn empty_is_none_and_sparse_output_owns_only_real_native_tiles() {
    let q = quota();
    let before = q.usage().unwrap();
    let canvas = Rect::new(0, 0, 257, 2);
    let mut b = AccountedSelectionBandBuilder::new(canvas, &q, &control()).unwrap();
    b.push_band(canvas, &[0; 514], &control()).unwrap();
    assert!(b.finish(&control()).unwrap().is_none());
    assert_eq!(q.usage().unwrap(), before);
    let mut raw = [0; 514];
    raw[513] = 255;
    let mut b = AccountedSelectionBandBuilder::new(canvas, &q, &control()).unwrap();
    b.push_band(canvas, &raw, &control()).unwrap();
    let s = b.finish(&control()).unwrap().unwrap();
    assert_eq!(s.surface().tile_count(), 1);
    assert_eq!(s.surface().default_bytes(), [0]);
    let mut pixel = [0];
    s.surface().try_read_pixel_bytes(256, 1, &mut pixel).unwrap();
    assert_eq!(pixel, [255]);
    assert_eq!(s.owner._payload.bytes(), 65537);
    drop(s);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn prospective_quota_denial_and_late_cancel_release_every_private_allocation() {
    let tiny = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 1 });
    assert!(AccountedSelectionBandBuilder::new(Rect::new(0, 0, 1, 1), &tiny, &control()).is_err());
    assert_eq!(tiny.usage().unwrap().resident, 0);
    let q = quota();
    let before = q.usage().unwrap();
    let mut b = AccountedSelectionBandBuilder::new(Rect::new(0, 0, 1, 2), &q, &control()).unwrap();
    let stop = AtomicBool::new(false);
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let segments = AtomicUsize::new(0);
    let work = |n| {
        if n == 1026 && segments.fetch_add(1, Ordering::Relaxed) == 1 {
            stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    let ctl = ReadControl { interrupt: &interrupt, work: &work };
    assert!(b.push_band(Rect::new(0, 0, 1, 2), &[1, 2], &ctl).is_err());
    assert!(b.finish(&control()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn exact_storage_comparator_checks_defaults_topology_tail_bytes_and_polling() {
    let q = quota();
    let mut b = AccountedSelectionBandBuilder::new(Rect::new(0, 0, 1, 1), &q, &control()).unwrap();
    b.push_band(Rect::new(0, 0, 1, 1), &[127], &control()).unwrap();
    let s = b.finish(&control()).unwrap().unwrap();
    let mut equal = s.surface().clone();
    assert!(s.matches_native_surface(&equal, &control()).unwrap());
    // Equal logical coverage cannot hide retained off-canvas words.
    equal.try_write_interleaved(Rect::new(1, 0, 2, 1), &[1]).unwrap();
    assert!(!s.matches_native_surface(&equal, &control()).unwrap());
    let other = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[1]).unwrap();
    assert!(!s.matches_native_surface(&other, &control()).unwrap());
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(s.matches_native_surface(s.surface(), &deny).is_err());
}
