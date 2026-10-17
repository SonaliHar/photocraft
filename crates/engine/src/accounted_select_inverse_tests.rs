//! Deferred native inverse contracts. Ignored UNRUN, uncompiled; input/reference
//! buffers are external harness allocations. No Source or image-sampling grant.
use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::accounted_selection::CoverageDomain;
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits},
};
#[path = "accounted_programme_selection_native_test_support.rs"]
mod f;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 64 << 20, scratch: 16 << 20 })
}
#[test]
#[ignore = "UNRUN: native inverse raw-word contract; not compiled"]
fn native_words_keep_integer_lowbits_and_float_schedule() {
    let rows = [
        (SampleType::U8, vec![17], vec![238]),
        (SampleType::U16, 0x8001u16.to_ne_bytes().to_vec(), 0x7ffeu16.to_ne_bytes().to_vec()),
        (SampleType::U32, 0x80000001u32.to_ne_bytes().to_vec(), 0x7ffffffeu32.to_ne_bytes().to_vec()),
        (SampleType::F32, (-0.0f32).to_ne_bytes().to_vec(), 1.0f32.to_ne_bytes().to_vec()),
        (SampleType::F64, f64::from_bits(0x3fe0000000000001).to_ne_bytes().to_vec(), f64::from_bits(0x3fdffffffffffffe).to_ne_bytes().to_vec()),
    ];
    for (sample, input, expected) in rows {
        let mut actual = vec![0; input.len()];
        invert(&input, sample, &mut actual).unwrap();
        assert_eq!(actual, expected);
    }
    let mut out = [42; 8];
    assert_eq!(invert(&f64::NAN.to_ne_bytes(), SampleType::F64, &mut out), Err(BackingError::Unsupported));
    assert_eq!(out, [42; 8]);
    assert_eq!(invert(&[0; 3], SampleType::U32, &mut out), Err(BackingError::Length));
    assert_eq!(out, [42; 8]);
}
#[test]
#[ignore = "UNRUN: physical native bands and poison/control contract; not compiled"]
fn native_bands_cross_tiles_keep_depth_and_sparse_empty() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let q = quota();
        let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let bpp = format.bytes_per_pixel();
        let area = Rect::new(0, 0, 257, 2);
        let input = match sample {
            SampleType::U8 => vec![255],
            SampleType::U16 => u16::MAX.to_ne_bytes().to_vec(),
            SampleType::U32 => u32::MAX.to_ne_bytes().to_vec(),
            SampleType::F32 => 1f32.to_ne_bytes().to_vec(),
            SampleType::F64 => 1f64.to_ne_bytes().to_vec(),
        };
        let mut b = AccountedNativeSelectionBandBuilder::new(format, area, &q, &ctl()).unwrap();
        b.push_band(Rect::new(0, 0, 257, 1), &input.repeat(257), &ctl()).unwrap();
        b.push_band(Rect::new(0, 1, 257, 2), &vec![0; 257 * bpp], &ctl()).unwrap();
        let guard = b.finish(&ctl()).unwrap().unwrap();
        assert_eq!(guard.format(), format);
        assert_eq!(guard.bounds(), area);
        assert!(guard.quota().shares_ledger(&q));
        let mut empty = AccountedNativeSelectionBandBuilder::new(format, area, &q, &ctl()).unwrap();
        empty.push_band(area, &vec![0; 514 * bpp], &ctl()).unwrap();
        assert!(empty.finish(&ctl()).unwrap().is_none());
        let mut bad = AccountedNativeSelectionBandBuilder::new(format, area, &q, &ctl()).unwrap();
        assert!(bad.push_band(Rect::new(0, 1, 257, 2), &input.repeat(257), &ctl()).is_err());
        assert!(bad.push_band(Rect::new(0, 0, 257, 1), &input.repeat(257), &ctl()).is_err());
        assert!(bad.finish(&ctl()).is_err());
        let before = q.usage().unwrap();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(AccountedNativeSelectionBandBuilder::new(format, area, &q, &denied).is_err());
        assert_eq!(q.usage().unwrap(), before);
        let mut stopped = AccountedNativeSelectionBandBuilder::new(format, area, &q, &ctl()).unwrap();
        assert!(stopped.push_band(area, &input.repeat(514), &denied).is_err());
        assert!(stopped.finish(&ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: genuine programme selection inverse transaction/fences; not compiled"]
fn genuine_programmes_replace_guard_without_changing_programme_and_refuse_stale_target() {
    for case in f::CASES {
        for depth in 0..2 {
            let input = f::load(case, depth);
            let mut session = Session::new();
            f::adopt(&mut session, case, &input);
            let original = session.active().unwrap().doc.clone();
            let owner = original.accounted().unwrap();
            let format = PixelFormat::new(ColorMode::Grayscale, original.depth, false);
            let word = if depth == 0 { 0.25f32.to_ne_bytes().to_vec() } else { 0.25f64.to_ne_bytes().to_vec() };
            let n = original.size.width as usize * original.size.height as usize;
            let selection = AccountedSelection::try_from_native(
                format,
                original.bounds(),
                &vec![0; word.len()],
                &word.repeat(n),
                CoverageDomain::FiniteUnit,
                owner.quota(),
                &ctl(),
            )
            .unwrap();
            session
                .replace_accounted_selection(AccountedSelectionTarget::capture_with_control(&session, &ctl()).unwrap(), "fixture", Some(&selection), &ctl())
                .unwrap();
            let before = session.active().unwrap().doc.clone();
            let target = AccountedSelectionTarget::capture_with_control(&session, &ctl()).unwrap();
            run_captured(&mut session, &Value::Null, target, &ctl()).unwrap();
            let after = &session.active().unwrap().doc;
            assert_eq!(after.xcf_state, before.xcf_state);
            assert_eq!(after.source_color_context, before.source_color_context);
            assert_eq!(after.icc_profile, before.icc_profile);
            let expected = if depth == 0 { 0.75f32.to_ne_bytes().to_vec() } else { 0.75f64.to_ne_bytes().to_vec() };
            assert_eq!(after.selection.as_ref().unwrap().try_read_interleaved_with(after.bounds(), &Interrupt::NONE).unwrap(), expected.repeat(n));
            let midpoint_word = if depth == 0 { 0.5f32.to_ne_bytes().to_vec() } else { 0.5f64.to_ne_bytes().to_vec() };
            let midpoint = AccountedSelection::try_from_native(
                format,
                original.bounds(),
                &vec![0; midpoint_word.len()],
                &midpoint_word.repeat(n),
                CoverageDomain::FiniteUnit,
                owner.quota(),
                &ctl(),
            )
            .unwrap();
            session
                .replace_accounted_selection(AccountedSelectionTarget::capture_with_control(&session, &ctl()).unwrap(), "midpoint", Some(&midpoint), &ctl())
                .unwrap();
            let same = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let past = session.active().unwrap().history.past_len();
            let target = AccountedSelectionTarget::capture_with_control(&session, &ctl()).unwrap();
            run_captured(&mut session, &Value::Null, target, &ctl()).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&same));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.past_len(), past);
            let stale = AccountedSelectionTarget::capture_with_control(&session, &ctl()).unwrap();
            session.replace_accounted_selection(AccountedSelectionTarget::capture_with_control(&session, &ctl()).unwrap(), "Deselect", None, &ctl()).unwrap();
            let unchanged = session.active().unwrap().doc.clone();
            assert!(run_captured(&mut session, &Value::Null, stale, &ctl()).is_err());
            assert!(session.active().unwrap().doc.shares_state(&unchanged));
            assert!(validate_request(&session, &Value::Null).is_err());
        }
    }
}
