//! Deferred actual paid Legacy-owner selected Clear lifecycle. No Source extension.
use super::*;
use photocraft_raster::backing::quota::QuotaKind;
#[test]
#[ignore = "UNRUN/uncompiled: native fractional selection registered Clear/undo/redo and same-meter additional-work/quota/cancel atomicity"]
fn native_selection_fractional_clear_history_and_controls() {
    for depth in [SampleType::U32, SampleType::F64] {
        let mut s = fixture(depth, 3, 1, false);
        let sf = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, depth, false);
        let weight = if depth == SampleType::U32 { f64::from(0x80000001u32) / f64::from(u32::MAX) } else { 0.5 + 2f64.powi(-42) };
        let mut mask = vec![0; 3 * sf.bytes_per_pixel()];
        photocraft_color::try_write_sample_f64(&mut mask, depth, 1, weight).unwrap();
        photocraft_color::try_write_sample_f64(&mut mask, depth, 2, 1.0).unwrap();
        select(&mut s, sf, &mask);
        let raw = native(&s);
        let owner = s.active().unwrap().doc.clone();
        let q = owner.quota().unwrap().clone();
        let baseline = q.usage().unwrap();
        let bounds = owner.selection.as_ref().unwrap().try_content_bounds_with_interrupt(&Interrupt::NONE).unwrap();
        let selected_work = (u64::from(bounds.width()) * u64::from(bounds.height()) * 1024).min(4096);
        let p = json!({});
        let deny = |n| if n == selected_work { Err(BackingError::Work) } else { Ok(()) };
        assert!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        assert!(owner.shares_state(&s.active().unwrap().doc));
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &cancelled, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        let block = q.reserve(QuotaKind::Resident, (256 << 20) - baseline.resident).unwrap();
        assert!(Prepared::prepare(&s, &p, &ctl()).is_err());
        drop(block);
        assert_eq!(q.usage().unwrap(), baseline);
        let bpp = owner.pixel_format().bytes_per_pixel();
        let mut expected = raw.clone();
        let alpha = photocraft_color::try_read_sample_f64(&raw[bpp..2 * bpp], depth, 3).unwrap();
        photocraft_color::try_write_sample_f64(&mut expected[bpp..2 * bpp], depth, 3, alpha * (1.0 - weight)).unwrap();
        expected[2 * bpp..].fill(0);
        let history = s.active().unwrap().history.past_len();
        s.execute(ID, json!({})).unwrap();
        assert_eq!(native(&s), expected);
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert_eq!(s.active().unwrap().doc.xcf_state, owner.xcf_state);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert_eq!(native(&s), raw);
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert_eq!(native(&s), expected);
    }
}
