//! UNRUN registered owned Fill, resident-kernel parity and atomicity drafts.
use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::{
    LayerContent,
    accounted_selection::{AccountedSelection, CoverageDomain},
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackingError,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(mode: ColorMode, sample: SampleType, alpha_lock: bool) -> Session {
    let (precision, color, alpha) = match sample {
        SampleType::U8 => (150, vec![127], vec![191]),
        SampleType::U16 => (250, 0x8001u16.to_be_bytes().to_vec(), 0xc001u16.to_be_bytes().to_vec()),
        SampleType::U32 => (350, 0x80000001u32.to_be_bytes().to_vec(), 0xc0000001u32.to_be_bytes().to_vec()),
        SampleType::F32 => (650, 0.5f32.to_be_bytes().to_vec(), 0.75f32.to_be_bytes().to_vec()),
        SampleType::F64 => (750, (0.5 + 2f64.powi(-44)).to_be_bytes().to_vec(), 0.75f64.to_be_bytes().to_vec()),
    };
    let mut pixel = color.repeat(if mode == ColorMode::Grayscale { 1 } else { 3 });
    pixel.extend(alpha);
    let mut options = xcf::FixtureOptions { model: if mode == ColorMode::Grayscale { 1 } else { 0 }, ..Default::default() };
    if alpha_lock {
        options.layer_properties.push((10, 1u32.to_be_bytes().to_vec()));
    }
    let q = BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 16 << 20 }, 256 << 20, QuotaUsage::default())
        .unwrap()
        .0;
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let source = Arc::new(xcf::image_custom(13, precision, 0, 257, 2, &pixel.repeat(257 * 2), &options));
    let imported = photocraft_io::import_owned_staged_plain_bytes_in_quota("source.xcf", source, &Default::default(), &limits, q, &ctl()).unwrap();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &ctl()).unwrap();
    session
}
fn native(session: &Session) -> Vec<u8> {
    let state = session.active().unwrap();
    let LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
    let area = layer.operative_bounds();
    let bpp = layer.format().bytes_per_pixel();
    let mut raw = vec![0; area.width() as usize * area.height() as usize * bpp];
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let begin = ((y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize) * bpp;
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[begin..begin + bpp], &ctl()).unwrap();
        }
    }
    raw
}
fn selection(session: &mut Session) {
    let state = session.active().unwrap();
    let mut bytes = vec![0; 257 * 2];
    bytes[0] = 255;
    bytes[255] = 128;
    bytes[256] = 255;
    bytes[257 + 256] = 255;
    let owner = AccountedSelection::try_from_native(
        PixelFormat::GRAY8,
        state.doc.bounds(),
        &[0],
        &bytes,
        CoverageDomain::FiniteUnit,
        state.doc.quota().unwrap(),
        &ctl(),
    )
    .unwrap();
    let target = crate::accounted_selection_session::AccountedSelectionTarget::capture(session).unwrap();
    session.replace_accounted_selection(target, "test selection", Some(&owner), &ctl()).unwrap();
}
fn ordinary(session: &Session) -> Session {
    let state = session.active().unwrap();
    let raw = native(session);
    let mut doc = state.doc.as_ref().clone();
    doc.layers[0].content = LayerContent::Raster(Surface::try_from_interleaved(doc.pixel_format(), doc.bounds(), &raw).unwrap());
    // Numeric oracle only. This deliberately supplies no imported/source proof.
    doc.xcf_state = None;
    doc.source_color_context = None;
    let mut expected = Session::new();
    expected.add_document(doc, None).unwrap();
    expected.tools.foreground = session.tools.foreground;
    expected.tools.background = session.tools.background;
    expected
}

#[test]
fn registered_solid_fill_matches_actual_resident_command_across_native_cells() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for params in [
                json!({"contents":"white"}),
                json!({"contents":"gray","mode":"multiply","opacity":37}),
                json!({"contents":"color","color":[0.2,0.8,0.4,0.6],"mode":"screen","preserveTransparency":true}),
                json!({"contents":"foreground","mode":"dissolve","opacity":51}),
            ] {
                let mut session = fixture(mode, sample, false);
                selection(&mut session);
                session.tools.foreground = [0.1, 0.3, 0.8, 0.7];
                let before = session.active().unwrap().doc.clone();
                let source = session.active().unwrap().source.clone();
                let mut expected = ordinary(&session);
                let result = session.execute("edit.fill", params.clone()).unwrap();
                expected.execute("edit.fill", params.clone()).unwrap();
                let state = expected.active().unwrap();
                let expected = state.doc.layers[0].surface().unwrap().try_read_interleaved(state.doc.bounds()).unwrap();
                assert_eq!(native(&session), expected, "{mode:?}/{sample:?}/{params}");
                assert!(result["contents"].is_string());
                assert_eq!(session.active().unwrap().doc.xcf_state, before.xcf_state);
                assert_eq!(session.active().unwrap().source, source);
                let LayerContent::BackedRaster(layer) = &session.active().unwrap().doc.layers[0].content else { panic!() };
                assert_eq!(layer.default_bytes(), &vec![0; layer.format().bytes_per_pixel()]);
            }
        }
    }
}
#[test]
fn registered_reply_history_noop_redo_and_alpha_lock_are_preserved() {
    let mut session = fixture(ColorMode::Rgb, SampleType::F32, true);
    let old = native(&session);
    let before = session.active().unwrap().doc.clone();
    let past = session.active().unwrap().history.past_len();
    let result = session.execute("edit.fill", json!({"contents":"white"})).unwrap();
    assert_eq!(result, json!({"contents":"white"}));
    assert_eq!(session.active().unwrap().history.past_len(), past + 1);
    let changed = native(&session);
    for (a, b) in old.as_chunks::<16>().0.iter().zip(changed.as_chunks::<16>().0.iter()) {
        assert_eq!(&a[12..], &b[12..]);
    }
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(native(&session), old);
    assert!(before.shares_state(&session.active().unwrap().doc));
    // True zero-opacity no-op returns the canonical response and keeps redo.
    assert_eq!(session.execute("edit.fill", json!({"contents":"black","opacity":0})).unwrap(), json!({"contents":"black"}));
    assert!(session.active().unwrap().history.can_redo());
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(native(&session), changed);
    assert!(session.journal.last().is_some());
}
#[test]
fn upstream_defaults_ignored_fields_and_color_fallback_match_resident() {
    for params in [
        Value::Null,
        json!({"contents":null,"mode":null,"opacity":null,"preserveTransparency":null}),
        json!({"color":"invalid","ignored":{"nested":[1,2,3]}}),
        json!({"color":[0.2,"bad",0.4,null,999]}),
        json!({"contents":"background","mode":"N_o-r(m)a l","opacity":200}),
    ] {
        let mut session = fixture(ColorMode::Rgb, SampleType::U16, false);
        session.tools.foreground = [0.2, 0.4, 0.6, 0.8];
        session.tools.background = [0.7, 0.3, 0.1, 0.9];
        let mut expected = ordinary(&session);
        assert_eq!(session.execute("edit.fill", params.clone()).unwrap(), expected.execute("edit.fill", params).unwrap());
        let state = expected.active().unwrap();
        assert_eq!(native(&session), state.doc.layers[0].surface().unwrap().try_read_interleaved(state.doc.bounds()).unwrap());
    }
}
#[test]
fn stale_view_source_identity_and_tools_refuse_without_history_or_journal_change() {
    for mutation in 0..3 {
        let mut session = fixture(ColorMode::Rgb, SampleType::F32, false);
        let params = json!({"contents":"foreground"});
        let before = session.active().unwrap().doc.clone();
        let journal = session.journal.clone();
        let prepared = Prepared::prepare(&session, &params, &ctl()).unwrap();
        match mutation {
            0 => session.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            1 => session.active_mut().unwrap().path = Some("other.pcraft".into()),
            _ => session.tools.foreground = [0.8, 0.1, 0.2, 1.0],
        }
        assert!(prepared.execute(&mut session, &ctl()).is_err());
        assert!(before.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.journal, journal);
    }
}
#[test]
fn unsupported_recipes_wide_formats_and_hostile_values_are_private() {
    let mut session = fixture(ColorMode::Rgb, SampleType::F32, false);
    let before = session.active().unwrap().doc.clone();
    let journal = session.journal.clone();
    for params in [
        json!({"contents":"pattern"}),
        json!({"contents":"history"}),
        json!({"contents":"contentAware"}),
        json!({"target":"mask"}),
        json!({"target":{"channel":0}}),
        json!({"contents":{"huge":vec![0;4096]}}),
        json!({"mode":[vec![0;4096]]}),
        json!({"opacity":{"nested":true}}),
        json!({"preserveTransparency":[false]}),
    ] {
        assert!(session.execute("edit.fill", params).is_err());
        assert!(before.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.journal, journal);
    }
    for sample in [SampleType::U32, SampleType::F64] {
        let mut wide = fixture(ColorMode::Rgb, sample, false);
        let before = wide.active().unwrap().doc.clone();
        assert!(wide.execute("edit.fill", json!({"contents":"white"})).is_err());
        assert!(before.shares_state(&wide.active().unwrap().doc));
    }
}
#[test]
fn actual_work_cancel_and_live_quota_denials_keep_owner_and_charges() {
    let session = fixture(ColorMode::Rgb, SampleType::F32, false);
    let _warm = native(&session);
    let owner = session.active().unwrap().doc.clone();
    let q = owner.quota().unwrap();
    let before = q.usage().unwrap();
    let params = json!({"contents":"white"});
    let deny = |_| Err(BackingError::Work);
    assert!(Prepared::prepare(&session, &params, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(Prepared::prepare(&session, &params, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    let polls = AtomicU64::new(0);
    let late = |_| {
        polls.fetch_add(1, Ordering::Relaxed);
        if q.usage().unwrap().index > before.index { Err(BackingError::Work) } else { Ok(()) }
    };
    assert!(Prepared::prepare(&session, &params, &ReadControl { interrupt: &Interrupt::NONE, work: &late }).is_err());
    assert!(polls.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), before);
    let fee = q.reserve(QuotaKind::Resident, (256 << 20) - before.resident).unwrap();
    assert!(Prepared::prepare(&session, &params, &ctl()).is_err());
    drop(fee);
    assert_eq!(q.usage().unwrap(), before);
    assert!(owner.shares_state(&session.active().unwrap().doc));
}
