//! UNRUN registered plain-owned Clear lifecycle and exact-kernel comparisons.
use super::*;
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(depth: SampleType, width: u32, height: u32, locked: bool) -> Session {
    let (precision, word) = match depth {
        SampleType::U8 => (150, vec![127]),
        SampleType::U16 => (250, 32768u16.to_be_bytes().to_vec()),
        SampleType::U32 => (350, 0x80000001u32.to_be_bytes().to_vec()),
        SampleType::F32 => (650, 0.5f32.to_be_bytes().to_vec()),
        SampleType::F64 => (750, (0.5 + 2f64.powi(-40)).to_be_bytes().to_vec()),
        _ => panic!(),
    };
    let mut raw = word.repeat(width as usize * height as usize * 4);
    if depth == SampleType::F64 {
        raw[..8].copy_from_slice(&(-0.0f64).to_be_bytes());
    }
    if depth == SampleType::F32 {
        raw[..4].copy_from_slice(&(-0.0f32).to_be_bytes());
    }
    let q = BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 16 << 20 }, 256 << 20, QuotaUsage::default())
        .unwrap()
        .0;
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = xcf::FixtureOptions::default();
    if locked {
        options.layer_properties.push((28, 1u32.to_be_bytes().to_vec()));
    }
    let imported = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "source.xcf",
        Arc::new(xcf::image_custom(13, precision, 0, width, height, &raw, &options)),
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &ctl()).unwrap();
    s
}
fn native(s: &Session) -> Vec<u8> {
    let st = s.active().unwrap();
    let LayerContent::BackedRaster(layer) = &st.doc.layers[0].content else { panic!() };
    let area = layer.operative_bounds();
    let bpp = layer.format().bytes_per_pixel();
    let mut raw = vec![0; area.width() as usize * area.height() as usize * bpp];
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let i = ((y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize) * bpp;
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[i..i + bpp], &ctl()).unwrap();
        }
    }
    raw
}
fn select(s: &mut Session, format: photocraft_color::PixelFormat, bytes: &[u8]) {
    let st = s.active().unwrap();
    let q = st.doc.quota().unwrap();
    let selection =
        AccountedSelection::try_from_native(format, st.doc.bounds(), &vec![0; format.bytes_per_pixel()], bytes, CoverageDomain::FiniteUnit, q, &ctl()).unwrap();
    let target = crate::accounted_selection_session::AccountedSelectionTarget::capture(s).unwrap();
    s.replace_accounted_selection(target, "fixture selection", Some(&selection), &ctl()).unwrap();
}
#[test]
fn registered_clear_is_one_undo_preserves_complete_source_state_and_noop_redo() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let mut s = fixture(depth, 257, 2, false);
        let before = s.active().unwrap().doc.clone();
        let source = s.active().unwrap().source.clone();
        let raw = native(&s);
        let history = s.active().unwrap().history.past_len();
        s.execute(ID, json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(native(&s).iter().all(|v| *v == 0));
        assert_eq!(s.active().unwrap().doc.xcf_state, before.xcf_state);
        assert_eq!(s.active().unwrap().doc.source_color_context, before.source_color_context);
        assert_eq!(s.active().unwrap().source, source);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert_eq!(native(&s), raw);
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert!(s.redo_with_control(&ctl()).unwrap());
        s.execute("layer.setProps", json!({"opacity":0.25})).unwrap();
        assert!(s.undo_with_control(&ctl()).unwrap());
        let cleared = s.active().unwrap().doc.clone();
        s.execute(ID, json!({})).unwrap();
        assert!(cleared.shares_state(&s.active().unwrap().doc));
        assert!(s.redo_with_control(&ctl()).unwrap());
    }
}
#[test]
fn ordinary_fractional_selection_and_native_wide_alpha_rules_match_clear_policy() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = fixture(depth, 3, 1, false);
        select(&mut s, photocraft_color::PixelFormat::GRAY8, &[0, 128, 255]);
        let st = s.active().unwrap();
        let raw = native(&s);
        let format = st.doc.pixel_format();
        let mut expected = Surface::try_from_interleaved(format, st.doc.bounds(), &raw).unwrap();
        crate::pixels::try_clear_surface(&mut expected, st.doc.selection.as_ref().unwrap().content_bounds(), st.doc.selection.as_ref(), &Interrupt::NONE)
            .unwrap();
        let expected = expected.try_read_interleaved(st.doc.bounds()).unwrap();
        s.execute(ID, json!({})).unwrap();
        assert_eq!(native(&s), expected);
    }
    for depth in [SampleType::U32, SampleType::F64] {
        let mut s = fixture(depth, 3, 1, false);
        select(&mut s, photocraft_color::PixelFormat::GRAY8, &[0, 128, 255]);
        let before = s.active().unwrap().doc.clone();
        let raw_fractional = native(&s);
        let format = before.pixel_format();
        let bpp = format.bytes_per_pixel();
        let mut expected = raw_fractional.clone();
        let alpha = photocraft_color::try_read_sample_f64(&raw_fractional[bpp..2*bpp], depth, 3).unwrap();
        photocraft_color::try_write_sample_f64(&mut expected[bpp..2*bpp], depth, 3, alpha * (1.0 - 128.0/255.0)).unwrap();
        expected[2*bpp..].fill(0);
        s.execute(ID, json!({})).unwrap();
        assert_eq!(native(&s), expected);
        assert_eq!(&native(&s)[bpp..bpp+3*(bpp/4)], &raw_fractional[bpp..bpp+3*(bpp/4)]);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert_eq!(native(&s), raw_fractional);
        assert!(before.shares_state(&s.active().unwrap().doc));
        select(&mut s, photocraft_color::PixelFormat::GRAY8, &[0, 0, 255]);
        let raw = native(&s);
        s.execute(ID, json!({})).unwrap();
        let output = native(&s);
        let bpp = s.active().unwrap().doc.pixel_format().bytes_per_pixel();
        assert_eq!(&output[..2 * bpp], &raw[..2 * bpp]);
        assert!(output[2 * bpp..].iter().all(|v| *v == 0));
    }
}
#[test]
fn named_target_locks_view_and_request_refusals_leave_model_and_journal() {
    let mut locked = fixture(SampleType::F32, 2, 1, true);
    let before = locked.active().unwrap().doc.clone();
    assert!(locked.execute(ID, json!({})).is_err());
    assert!(before.shares_state(&locked.active().unwrap().doc));
    let mut s = fixture(SampleType::F32, 2, 1, false);
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    for p in [
        json!({"target":"mask"}),
        json!({"target":{"channel":0}}),
        json!({"layer":0}),
        json!({"documentId":"1","expectedRevision":"999"}),
        json!({"mode":"normal"}),
    ] {
        assert!(s.execute(ID, p).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.journal, journal);
    }
    let p = json!({});
    let prepared = Prepared::prepare(&s, &p, &ctl()).unwrap();
    s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
    let name = s.active().unwrap().doc.layers[0].name.clone();
    s.execute(ID, json!({"layer":name})).unwrap();
}
#[test]
fn cancellation_work_and_live_quota_denial_are_private() {
    let mut s = fixture(SampleType::F64, 2, 1, false);
    let _warm = native(&s);
    let owner = s.active().unwrap().doc.clone();
    let q = owner.quota().unwrap().clone();
    let baseline = q.usage().unwrap();
    let p = json!({});
    let deny = |_| Err(BackingError::Work);
    assert!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    let stop = || true;
    assert!(matches!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    let calls = AtomicU64::new(0);
    let late = |_| {
        if q.usage().unwrap().resident > baseline.resident && calls.fetch_add(1, Ordering::Relaxed) > 2 { Err(BackingError::Cancelled) } else { Ok(()) }
    };
    assert!(Prepared::prepare(&s, &p, &ReadControl { interrupt: &Interrupt::NONE, work: &late }).is_err());
    assert!(calls.load(Ordering::Relaxed) > 2);
    assert_eq!(q.usage().unwrap(), baseline);
    let block = q.reserve(QuotaKind::Resident, (256 << 20) - baseline.resident).unwrap();
    assert!(Prepared::prepare(&s, &p, &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(owner.shares_state(&s.active().unwrap().doc));
    s.execute(ID, json!({})).unwrap();
}
#[test]
fn native_roi_cap_refuses_large_canvas_before_clear_or_journal_publication() {
    let mut s = fixture(SampleType::U8, 2049, 2049, false);
    let owner = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    assert!(s.execute(ID, json!({})).is_err());
    assert!(owner.shares_state(&s.active().unwrap().doc));
    assert_eq!(s.journal, journal);
}

#[path = "accounted_fractional_clear_tests.rs"]
mod fractional_tests;
