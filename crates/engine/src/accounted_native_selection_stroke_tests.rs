//! Deferred actual Legacy installed-selection fences. Generated XCF/native
//! input and comparison buffers are external harness, not document receipts.
use super::*;
#[path = "accounted_native_stroke_test_support.rs"]
mod narrow;
#[path = "accounted_wide_plain_xcf_test_support.rs"]
mod wide;
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::sync::Arc;
// Same real import ledger permits constructor scratch; old fixture scratch0
// owners are intentionally not asserted to support new selection allocation.
pub(super) fn session(sample: SampleType) -> Session {
    let bytes = if matches!(sample, SampleType::U32 | SampleType::F64) {
        wide::encoded(false, sample, false)
    } else {
        let (precision, colour, alpha) = match sample {
            SampleType::U8 => (150, vec![63], vec![191]),
            SampleType::U16 => (250, 0x4001u16.to_be_bytes().to_vec(), 0xc001u16.to_be_bytes().to_vec()),
            SampleType::F32 => (650, 0.25000003f32.to_be_bytes().to_vec(), 0.75000006f32.to_be_bytes().to_vec()),
            _ => panic!("five native depths"),
        };
        let mut pixel = Vec::new();
        for _ in 0..3 {
            pixel.extend_from_slice(&colour);
        }
        pixel.extend_from_slice(&alpha);
        xcf::image_custom(13, precision, 0, 257, 259, &pixel.repeat(257 * 259), &Default::default())
    };
    let q = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 16 << 20 });
    let input = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "selected-stroke.xcf",
        Arc::new(bytes),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap(),
        q,
        &narrow::ctl(),
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&input, "selected-stroke.xcf", Some("authorized/selected-stroke.xcf"), &narrow::ctl()).unwrap();
    s.tools.brush.pressure_size = false;
    s
}
fn install(s: &mut Session, value: f64) {
    let owner = s.active().unwrap().doc.accounted().unwrap();
    let area = owner.document().bounds();
    let f = PixelFormat::new(ColorMode::Grayscale, owner.document().depth, false);
    let mut word = vec![0; f.bytes_per_pixel()];
    photocraft_color::try_write_sample_f64(&mut word, f.sample, 0, value).unwrap();
    let bytes = word.repeat(area.width() as usize * area.height() as usize);
    let selected =
        AccountedSelection::try_from_native(f, area, &vec![0; word.len()], &bytes, CoverageDomain::FiniteUnit, owner.quota(), &narrow::ctl()).unwrap();
    let target = crate::accounted_selection_session::AccountedSelectionTarget::capture(s).unwrap();
    s.replace_accounted_selection(target, "native coverage", Some(&selected), &narrow::ctl()).unwrap();
}
#[test]
#[ignore = "UNRUN/uncompiled: Legacy five-depth actual installed selection zero/noop/redo and native mask retention; captured and prepared selection replacement fences, original Work/cancel refund. No GIMP/UI acceptance or programme selection grant."]
fn installed_selection_noop_and_snapshot_fences_are_complete() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let mut s = session(sample);
        install(&mut s, 0.375);
        let before = s.active().unwrap().doc.clone();
        let bytes = narrow::payload(&before);
        let source = s.active().unwrap().source.clone();
        let down = capture_accounted_native_stroke_target(&s).unwrap().unwrap();
        let params = narrow::patch();
        let prepared = Request::parse(&s, "paint.stroke", &params, &narrow::ctl()).unwrap().prepare(&s, &narrow::ctl()).unwrap();
        install(&mut s, 0.625);
        let replaced = s.active().unwrap().doc.clone();
        let mut stale = narrow::patch();
        assert!(down.inject_captured_params(&s, &mut stale).is_err());
        assert!(prepared.execute(&mut s, &narrow::ctl()).is_err());
        assert!(replaced.shares_state(&s.active().unwrap().doc));
        assert_eq!(narrow::payload(&s.active().unwrap().doc), bytes);
        assert_eq!(s.active().unwrap().source, source);
        install(&mut s, 0.);
        let zero = s.active().unwrap().doc.clone();
        let selection = zero.selection.as_ref().unwrap().try_read_interleaved(zero.bounds()).unwrap();
        let rev = s.active().unwrap().revision;
        let past = s.active().unwrap().history.past_len();
        s.execute("paint.stroke", narrow::patch()).unwrap();
        assert!(zero.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().revision, rev);
        assert_eq!(s.active().unwrap().history.past_len(), past);
        assert_eq!(narrow::payload(&s.active().unwrap().doc), bytes);
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().try_read_interleaved(zero.bounds()).unwrap(), selection);
        // After undo, a selected0/out-of-bounds no-op must preserve the redo branch.
        assert!(s.undo_with_control(&narrow::ctl()).unwrap());
        let current = s.active().unwrap().doc.clone();
        let mut outside = narrow::patch();
        outside["points"] = serde_json::json!([[-1000., -1000., 1.]]);
        s.execute("paint.stroke", outside).unwrap();
        assert!(current.shares_state(&s.active().unwrap().doc));
        assert!(s.active().unwrap().history.can_redo());
        let owner = s.active().unwrap().doc.accounted().unwrap();
        let usage = owner.quota().usage().unwrap();
        let params = narrow::patch();
        let work = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
        assert!(Request::parse(&s, "paint.stroke", &params, &c).is_err());
        assert_eq!(owner.quota().usage().unwrap(), usage);
        let yes = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&yes);
        let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(Request::parse(&s, "paint.stroke", &params, &c).is_err());
        assert_eq!(owner.quota().usage().unwrap(), usage);
    }
}
