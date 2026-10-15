//! UNRUN paid Move regressions. Synthetic XCFs are real public decoder inputs;
//! native captures are external harness bytes, not claimed production receipts
//! or an independent GIMP rendering oracle. CURRENT-only lock/reference controls
//! have a genuine pre-copy fee and retain the original imported owner.
use super::*;
use crate::accounted_commands::PreparedAccountedCommand;
use photocraft_color::SampleType;
use photocraft_doc::{Document, LayerContent};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits},
};
use serde_json::json;
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[path = "../../../xcf/tests/common/mod.rs"]
mod xcf;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(depth: SampleType) -> Session {
    let (precision, word) = match depth {
        SampleType::U8 => (150, vec![127]),
        SampleType::U16 => (250, 32769u16.to_be_bytes().to_vec()),
        SampleType::U32 => (350, 0x80000001u32.to_be_bytes().to_vec()),
        SampleType::F32 => (650, (0.5 + 2f32.powi(-23)).to_be_bytes().to_vec()),
        SampleType::F64 => (750, (0.5 + 2f64.powi(-40)).to_be_bytes().to_vec()),
        _ => panic!(),
    };
    let mut raw = word.repeat(257 * 3 * 4);
    if depth == SampleType::F64 {
        raw[..8].copy_from_slice(&(-0.0f64).to_be_bytes());
    }
    if depth == SampleType::F32 {
        raw[..4].copy_from_slice(&(-0.0f32).to_be_bytes());
    }
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let original = Arc::new(xcf::image_custom(13, precision, 0, 257, 3, &raw, &Default::default()));
    let imported = photocraft_io::import_owned_staged_plain_bytes_in_quota("source.xcf", original, &Default::default(), &limits, q, &ctl()).unwrap();
    let mut s = Session::new();
    s.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &ctl()).unwrap();
    drop(imported);
    s
}
fn id(s: &Session) -> LayerId {
    s.active().unwrap().doc.layers[0].id
}
fn bounds(s: &Session) -> photocraft_geom::Rect {
    let LayerContent::BackedRaster(b) = &s.active().unwrap().doc.layers[0].content else { panic!() };
    b.operative_bounds()
}
fn native(s: &Session) -> Vec<u8> {
    let LayerContent::BackedRaster(b) = &s.active().unwrap().doc.layers[0].content else { panic!() };
    let r = b.operative_bounds();
    let n = b.format().bytes_per_pixel();
    let mut out = vec![0; r.width() as usize * r.height() as usize * n];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * n;
            b.checked_view().read_logical_pixel_native_into(x, y, &mut out[at..at + n], &ctl()).unwrap();
        }
    }
    out
}
fn archived(s: &Session) -> Session {
    let q = s.active().unwrap().doc.quota().unwrap();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&s.active().unwrap().doc, Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
    let archive =
        photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(&bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
            .unwrap();
    let mut out = Session::new();
    out.add_owned_native_archive(
        archive,
        crate::accounted_native_open::AccountedNativeOpenPolicy::KeepCurrentInterpretation,
        "Moved.pcraft",
        Some("actual/Moved.pcraft"),
        &ctl(),
    )
    .unwrap();
    out
}
#[test]
#[ignore = "UNRUN: five native depths, signed/tile-crossing coordinate views, exact raw pixels+alpha/profile/source, native continuation"]
fn registered_move_changes_only_current_coordinate_metadata_and_round_trips_native() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let mut s = fixture(depth);
        let before = s.active().unwrap().doc.clone();
        let words = native(&s);
        let original = bounds(&s);
        let source = s.active().unwrap().source.clone();
        let layer = id(&s);
        assert_eq!(s.execute(ID, json!({"layer":layer.0,"dx":-257,"dy":3})).unwrap(), Value::Null);
        let r = bounds(&s);
        assert_eq!(r, original.translate(-257, 3));
        assert_eq!(native(&s), words);
        let item = &s.active().unwrap().doc.xcf_state.as_ref().unwrap().items[0];
        assert_eq!(item.raster_bounds, Some(r));
        assert_eq!(s.active().unwrap().doc.icc_profile, before.icc_profile);
        assert_eq!(s.active().unwrap().doc.source_color_context, before.source_color_context);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Move"));
        let mut reopened = archived(&s);
        assert_eq!(bounds(&reopened), r);
        assert_eq!(native(&reopened), words);
        assert_eq!(reopened.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
        reopened.execute(ID, json!({"dx":257,"dy":-3})).unwrap();
        assert_eq!(bounds(&reopened), original);
        assert_eq!(native(&reopened), words);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        s.execute(ID, json!({"layer":{},"dx":0.1,"dy":false,"target":"mask"})).unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, rev);
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert_eq!(bounds(&s), r);
    }
}
#[test]
#[ignore = "UNRUN: mixed property/Move opaque keys continue one first-labelled anchor both ways; errors/noops preserve redo and key"]
fn coalescing_keys_are_not_prefixed_with_the_command_or_layer() {
    for properties_first in [false, true] {
        let mut s = fixture(SampleType::F64);
        let original = bounds(&s);
        let layer = id(&s);
        if properties_first {
            s.execute("layer.setProps", json!({"name":"Current","coalesce":"\0"})).unwrap();
        }
        s.execute(ID, json!({"layer":layer.0,"dx":3,"dy":-2,"coalesce":"\0"})).unwrap();
        s.execute("layer.setProps", json!({"opacity":0.5,"coalesce":"\0"})).unwrap();
        s.execute(ID, json!({"dx":-1,"dy":1,"coalesce":"\0"})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some(if properties_first { "Layer Properties" } else { "Move" }));
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert_eq!(bounds(&s), original);
        let history = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        s.execute(ID, json!({"layer":999999,"dx":0,"coalesce":"another"})).unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, rev);
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert_eq!(bounds(&s), original.translate(2, -1));
        s.execute(ID, json!({"dx":1,"coalesce":42})).unwrap();
        assert!(s.active().unwrap()._accounted_metadata_gesture.is_none());
        assert_eq!(s.active().unwrap().history.past_len(), 2);
    }
}
#[test]
#[ignore = "UNRUN: exact shared px/pt conversion, hidden guard before zero/target, hostile bounded unit diagnostic"]
fn delta_wire_and_outer_zero_guard_preserve_the_existing_schedule() {
    let mut s = fixture(SampleType::U8);
    let original = bounds(&s);
    let name = s.active().unwrap().doc.layers[0].name.clone();
    s.execute(ID, json!({"layer":name,"dx":1.6,"dy":-1.6,"units":false})).unwrap();
    assert_eq!(bounds(&s), original.translate(2, -2));
    let dpi = f64::from(s.active().unwrap().doc.resolution_dpi);
    let expected = (72.0 * dpi / 72.0).round() as i32;
    s.execute(ID, json!({"units":"pt","dx":72.0})).unwrap();
    assert_eq!(bounds(&s), original.translate(2 + expected, -2));
    let before = s.active().unwrap().doc.clone();
    for p in [
        json!({"dx":2147483648u64}),
        json!({"units":"pt","dx":null}),
        json!({"units":"q".repeat(1<<20),"dx":1}),
        json!({"dx":1,"target":"mask"}),
        json!({"dx":1,"target":"quickMask"}),
    ] {
        assert!(s.execute(ID, p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    s.execute("layer.setProps", json!({"visible":false})).unwrap();
    assert!(s.execute(ID, json!({"dx":0})).is_err());
    s.execute(ID, json!({"layer":999999,"dx":0})).unwrap();
    s.execute(ID, json!({"target":"quickMask","dx":0})).unwrap();
    assert!(s.execute(ID, json!({"layer":"absent","dx":0})).is_err());
}
fn current_with(change: impl FnOnce(&mut Document)) -> Session {
    let s = fixture(SampleType::F64);
    let source = s.active().unwrap().doc.accounted().unwrap().clone();
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    change(&mut doc);
    let owner = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q, &ctl()).unwrap();
    let mut out = Session::new();
    out.add_accounted_document(owner, None, &ctl()).unwrap();
    out
}
#[test]
#[ignore = "UNRUN: native position/all locks vs pixel-only, exact effect-reference shift and linked refusal without partial canonical updates"]
fn unsupported_current_fields_and_position_locks_never_apply_a_prefix() {
    for kind in 0..5 {
        let mut s = current_with(|doc| match kind {
            0 => doc.layers[0].locks.position = true,
            1 => doc.layers[0].locks.all = true,
            2 => doc.layers[0].effects.reference = Some((1.25, -0.0)),
            3 => doc.layers[0].link_group = Some(42),
            _ => doc.layers[0].locks.pixels = true,
        });
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let response = s.execute(ID, json!({"dx":1,"dy":1}));
        if matches!(kind, 2 | 4) {
            response.unwrap();
            if kind == 2 {
                assert_eq!(s.active().unwrap().doc.layers[0].effects.reference, Some((2.25, 1.0)));
            }
            let LayerContent::BackedRaster(old) = &before.layers[0].content else { panic!() };
            assert_eq!(bounds(&s), old.operative_bounds().translate(1, 1));
        } else {
            assert!(response.is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(s.journal, journal);
        }
    }
}
#[test]
#[ignore = "UNRUN: prepared Move paid source/path/history/prefs/channel/journal fences and cumulative cancel/quota rollback"]
fn prepared_and_direct_failures_keep_installed_state_and_physical_owners() {
    for changed in 0..7 {
        let mut s = fixture(SampleType::F64);
        let p = json!({"dx":2,"dy":-1});
        let prepared = PreparedAccountedCommand::prepare(&s, ID, &p, &ctl()).unwrap();
        match changed {
            0 => s.active_mut().unwrap().source.as_mut().unwrap().label = "changed source".into(),
            1 => s.active_mut().unwrap().path = Some("changed.pcraft".into()),
            2 => s.active_mut().unwrap().history.max_states += 1,
            3 => s.prefs.edit(|p| p.performance.history_states = p.performance.history_states.saturating_add(1)),
            4 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            5 => s.active_mut().unwrap().history.set_current_label("changed", &ctl()).unwrap(),
            _ => s.journal.push(("harness".into(), Value::Null)),
        };
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let rev = s.active().unwrap().revision;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().revision, rev);
    }
    let mut s = fixture(SampleType::F64);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let layer = id(&s);
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    current(before.accounted().unwrap(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    let cost = spent.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cost + cost / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(s.translate_accounted_imported_layer(layer, 2, -1, Some("private"), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert!(spent.load(Ordering::Relaxed) > cost);
    assert!(s.active().unwrap().doc.shares_state(&before));
    let q = before.quota().unwrap();
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute(ID, json!({"dx":1})).is_err());
    drop(hold);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}

#[test]
#[ignore = "UNRUN: prepared-model continuation preserves first anchor and rejects stale/tip/ledger/current-view mismatch without allocation"]
fn prepared_history_continuation_keeps_the_actual_first_anchor_and_noop_redo() {
    let mut s = fixture(SampleType::F64);
    s.execute(ID, json!({"dx":1,"coalesce":"history"})).unwrap();
    let state = s.active().unwrap();
    let owner = state.doc.accounted().unwrap();
    let mut history = state.history.clone();
    let expected = history.clone();
    let target = state.accounted_view.as_ref().unwrap().target();
    let staged = owner.try_with_imported_layer_translation(id(&s), 2, 0, &ctl()).unwrap();
    let old_len = history.past_len();
    history.try_continue_prepared_accounted(&expected, owner, staged.clone(), target, &ctl()).unwrap();
    assert_eq!(history.past_len(), old_len);
    assert_eq!(history.undo_label(), Some("Move"));
    let current = history.clone();
    assert!(history.try_continue_prepared_accounted(&expected, owner, staged.clone(), target, &ctl()).is_err());
    assert!(history.shares_accounted_history(&current));
    let mut limited = expected.clone();
    limited.max_states = 0;
    let limited_expected = limited.clone();
    // Exact actual-owner no-op bypasses changed-only history preferences.
    limited.try_continue_prepared_accounted(&limited_expected, owner, owner.clone(), target, &ctl()).unwrap();
    assert!(limited.shares_accounted_history(&limited_expected));
    let foreign = fixture(SampleType::F64).active().unwrap().doc.accounted().unwrap().clone();
    let mut h = expected.clone();
    assert!(h.try_continue_prepared_accounted(&expected, owner, foreign, target, &ctl()).is_err());
    assert!(h.shares_accounted_history(&expected));
    assert!(s.undo_with_control(&ctl()).unwrap());
    let state = s.active().unwrap();
    let owner = state.doc.accounted().unwrap();
    let target = state.accounted_view.as_ref().unwrap().target();
    let mut h = state.history.clone();
    let expected = h.clone();
    let staged = owner.try_with_imported_layer_translation(id(&s), 2, 0, &ctl()).unwrap();
    assert!(h.try_continue_prepared_accounted(&expected, owner, staged, target, &ctl()).is_err());
    assert!(h.shares_accounted_history(&expected));
    assert!(h.can_redo());
}
