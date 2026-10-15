//! UNRUN paid synthetic CURRENT Move, not a GIMP-authored translation oracle.
use super::*;
#[path = "../../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "../../../ui-egui/src/canvas_observed_box_fixture.rs"]
#[allow(dead_code)]
mod observed;
use photocraft_doc::{Document, LayerContent, SampleType, item_identity::ItemRef};
use photocraft_raster::Interrupt;
use serde_json::json;
use std::sync::Arc;
fn owner(sample: SampleType, assigned: bool, epoch: bool) -> AccountedDocument {
    let (base, _) = fixture::fixture(sample, assigned, 1);
    if epoch { observed::with_raw(&base, [i32::MIN, 0, i32::MAX], &fixture::ctl()) } else { base }
}
fn session(sample: SampleType, assigned: bool) -> Session {
    let mut s = Session::new();
    s.add_owned_native_point_archive(fixture::archive(&owner(sample, assigned, true)), "Observed Move", None, &fixture::ctl()).unwrap();
    s
}
fn current(base: &AccountedDocument, change: impl FnOnce(&mut Document)) -> AccountedDocument {
    let fee = base.quota().reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut d = base.document().clone();
    change(&mut d);
    assert!(photocraft_io::retained_native_model_census(&d, Default::default(), &Interrupt::NONE).unwrap().retained_bytes() as u64 <= fee.bytes());
    if base.is_readonly_assigned_point_storage() {
        AccountedDocument::new_readonly_assigned_point_storage(d, Arc::new((base.clone(), fee)), base.quota().clone(), &fixture::ctl()).unwrap()
    } else {
        AccountedDocument::new_readonly_point_storage(d, Arc::new((base.clone(), fee)), base.quota().clone(), &fixture::ctl()).unwrap()
    }
}
fn internal(owner: AccountedDocument) -> Session {
    let mut s = Session::new();
    let state = crate::DocState::new_accounted(owner, None, 16, &fixture::ctl()).unwrap();
    s.publish_document_state(state, &fixture::ctl()).unwrap();
    s
}
fn bounds(d: &Document) -> photocraft_geom::Rect {
    let LayerContent::BackedRaster(l) = &d.layers[0].content else { panic!("native") };
    l.operative_bounds()
}
#[test]
#[ignore = "UNRUN: all3 actualroles fixedroot/native inverse mapping across256, fixedcanvasmask/stack/raw/ICC/reference andstrict nativecontinuation"]
fn translated_source_keeps_canvas_mask_and_original_native() {
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        let base = owner(sample, assigned, true);
        let base = current(&base, |d| d.layers[0].effects.reference = Some((1.25, -0.0)));
        let mut s = internal(base);
        let old = s.active().unwrap().doc.clone();
        let original_bounds = bounds(&old);
        let id = old.layers[0].id;
        let past = s.active().unwrap().history.past_len();
        for (dx, dy) in [(7, 0), (-270, -5), (256, 2)] {
            let before = s.active().unwrap().doc.clone();
            let b = bounds(&before);
            let reference = before.layers[0].effects.reference.unwrap();
            let rev = s.active().unwrap().revision;
            assert_eq!(s.execute("layer.translate", json!({"layer":id.0,"dx":dx,"dy":dy})).unwrap(), Value::Null);
            let changed = s.active().unwrap().doc.clone();
            assert_eq!(bounds(&changed), photocraft_geom::Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy));
            assert_eq!(changed.layers[0].effects.reference, Some((reference.0 + f64::from(dx), reference.1 + f64::from(dy))));
            assert_eq!(changed.layers[0].effects.reference.unwrap().1.to_bits(), (reference.1 + f64::from(dy)).to_bits());
            assert_eq!(fixture::native(&changed), fixture::native(&old));
            assert_eq!(changed.source_color_context, old.source_color_context);
            assert_eq!(changed.icc_profile, old.icc_profile);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks, old.xcf_state.as_ref().unwrap().filter_stacks);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_masks, old.xcf_state.as_ref().unwrap().filter_masks);
            assert_eq!(changed.size, old.size);
            assert_eq!(s.active().unwrap().revision, rev + 1);
            let row = changed.xcf_state.as_ref().unwrap().items.iter().find(|r| r.item == ItemRef::Layer(id)).unwrap();
            assert_eq!(row.raster_bounds, Some(bounds(&changed)));
            assert_eq!(observed::raw(&changed), [i32::MIN, 0, i32::MAX]);
            let prepared =
                photocraft_compose::backed_box_context::PreparedBackedBoxContext::new(changed.accounted().unwrap(), Default::default(), &fixture::ctl())
                    .unwrap();
            let b = bounds(&changed);
            assert_eq!(prepared.bounds(), photocraft_geom::Rect::new(b.x0 - 4, b.y0 - 4, b.x1 + 4, b.y1 + 4));
            drop(prepared);
        }
        assert_ne!(bounds(&s.active().unwrap().doc), original_bounds);
        assert_eq!(s.active().unwrap().history.past_len(), past + 3);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Move"));
        let changed = s.active().unwrap().doc.clone();
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(fixture::archive(changed.accounted().unwrap()), "continued", None, &fixture::ctl()).unwrap();
        fixture::exact(&changed, &reopened.active().unwrap().doc);
    }
}
#[test]
#[ignore = "UNRUN: exact upstream rounds/wrongtypes/units/name-active hidden guard, zero-before-targetlocks andmask/link scope"]
fn wire_and_zero_behavior_stay_upstream() {
    let mut s = session(SampleType::F32, false);
    let old = bounds(&s.active().unwrap().doc);
    s.execute("layer.translate", json!({"dx":2.5,"dy":"ignored","units":false})).unwrap();
    let b = bounds(&s.active().unwrap().doc);
    assert_eq!((b.x0, b.y0), (old.x0 + 3, old.y0));
    let h = s.active().unwrap().history.clone();
    s.execute("layer.translate", json!({"dx":0.1,"dy":0.1,"layer":false,"target":"mask"})).unwrap();
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    s.execute("layer.setProps", json!({"visible":false})).unwrap();
    assert!(s.execute("layer.translate", json!({"dx":0,"layer":"CURRENT native drawable"})).is_err());
    let h = s.active().unwrap().history.clone();
    s.execute("layer.translate", json!({"dx":0,"layer":u64::MAX})).unwrap();
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    let id = s.active().unwrap().doc.layers[0].id;
    s.translate_accounted_imported_layer(id, 0, 0, None, &fixture::ctl()).unwrap();
    let mut s = session(SampleType::F32, false);
    for p in [json!({"dx":1,"target":"mask"}), json!({"dx":1,"target":"quickMask"}), json!({"dx":1,"units":"hostile"}), json!({"dx":null,"units":"pt"})] {
        assert!(s.execute("layer.translate", p).is_err());
    }
    // Point units reuse the existing actual DPI formula, including its rounding.
    let p = json!({"dx":1,"dy":-1,"units":"pt"});
    let expected = crate::layer_multi_cmds::translation_delta(&s, &p).unwrap();
    let before = bounds(&s.active().unwrap().doc);
    s.execute("layer.translate", p).unwrap();
    let after = bounds(&s.active().unwrap().doc);
    assert_eq!((after.x0, after.y0), (before.x0 + expected.0, before.y0 + expected.1));
}
#[test]
#[ignore = "UNRUN: sameopaque mixedmetadataMove key retains firstlabel/anchor, rawzero no-op keepsredo andnative continuation"]
fn metadata_and_move_share_one_opaque_gesture() {
    for move_first in [false, true] {
        let mut s = session(SampleType::F32, true);
        let original = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        let (first, second) = if move_first {
            (("layer.translate", json!({"dx":7,"dy":3,"coalesce":"same"})), ("layer.setProps", json!({"name":"renamed","coalesce":"same"})))
        } else {
            (("layer.setProps", json!({"name":"renamed","coalesce":"same"})), ("layer.translate", json!({"dx":7,"dy":3,"coalesce":"same"})))
        };
        s.execute(first.0, first.1).unwrap();
        s.execute(second.0, second.1).unwrap();
        let changed = s.active().unwrap().doc.clone();
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some(if move_first { "Move" } else { "Layer Properties" }));
        s.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(bounds(&s.active().unwrap().doc), bounds(&original));
        let h = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        s.execute("layer.translate", json!({"dx":0,"dy":0,"coalesce":"same"})).unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert_eq!(s.active().unwrap().revision, rev);
        s.execute("edit.redo", Value::Null).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&changed));
    }
}
#[test]
#[ignore = "UNRUN: nonobserved actualPoint directnonzero failsclosed; position/all/link/endpoint andprepared stale/work/cancel/quota areatomic"]
fn class_locks_and_failed_preparation_do_not_publish() {
    for assigned in [false, true] {
        let mut s = internal(owner(SampleType::F32, assigned, false));
        let old = s.active().unwrap().doc.clone();
        let id = old.layers[0].id;
        assert!(s.translate_accounted_imported_layer(id, 1, 0, None, &fixture::ctl()).is_err());
        assert!(s.translate_accounted_imported_selection(1, 0, None, &fixture::ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&old));
        s.translate_accounted_imported_layer(id, 0, 0, None, &fixture::ctl()).unwrap();
        let base = owner(SampleType::F32, assigned, true);
        for lock in 0..3 {
            let controlled = current(&base, |d| match lock {
                0 => d.layers[0].locks.position = true,
                1 => d.layers[0].locks.all = true,
                _ => d.layers[0].link_group = Some(7),
            });
            let mut s = internal(controlled);
            let old = s.active().unwrap().doc.clone();
            s.execute("layer.translate", json!({"dx":0})).unwrap();
            assert!(s.execute("layer.translate", json!({"dx":1})).is_err());
            assert!(s.active().unwrap().doc.shares_state(&old));
        }
        for fault in 0..5 {
            let mut s = session(SampleType::F32, assigned);
            let old = s.active().unwrap().doc.clone();
            let p = json!({"dx":1});
            let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.translate", &p, &fixture::ctl()).unwrap();
            match fault {
                0 => s.active_mut().unwrap().history.max_states += 1,
                1 => {
                    let r = s.active().unwrap().source_read_only;
                    s.active_mut().unwrap().source_read_only = !r;
                }
                2 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                3 => s.active_mut().unwrap().view_revision += 1,
                _ => s.journal.push(("external".into(), Value::Null)),
            }
            assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
            assert!(s.active().unwrap().doc.shares_state(&old));
        }
        let mut s = session(SampleType::F32, assigned);
        let old = s.active().unwrap().doc.clone();
        let h = s.active().unwrap().history.clone();
        let p = json!({"dx":i32::MAX});
        assert!(s.execute("layer.translate", p).is_err());
        let p = json!({"dx":1});
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(Request::parse(&s, &p, &deny).is_err());
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(Request::parse(&s, &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        let q = old.accounted().unwrap().quota().clone();
        let blocker = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.execute("layer.translate", p).is_err());
        drop(blocker);
        assert!(s.active().unwrap().doc.shares_state(&old));
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
    }
}
