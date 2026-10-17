//! UNRUN paid synthetic CURRENT metadata controls; no producer-authored activity claim.
use super::*;
use serde_json::Value;
#[path = "../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
#[path = "../../ui-egui/src/canvas_observed_box_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod observed;
use photocraft_doc::{Document, SampleType, item_identity::ItemRef};
use photocraft_raster::{Interrupt, backing::quota::QuotaKind};
use serde_json::json;
use std::sync::Arc;
fn owner(sample: SampleType, assigned: bool, observed_epoch: bool) -> AccountedDocument {
    let (base, _) = fixture::fixture(sample, assigned, 1);
    if observed_epoch { observed::with_raw(&base, [i32::MIN, 0, i32::MAX], &fixture::ctl()) } else { base }
}
fn session(sample: SampleType, assigned: bool) -> Session {
    let mut s = Session::new();
    s.add_owned_native_point_archive(fixture::archive(&owner(sample, assigned, true)), "Observed metadata", None, &fixture::ctl()).unwrap();
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
#[test]
#[ignore = "UNRUN: all3 paid observed roles combined scalar metadata/sharedkey/firstlabel/rawnoop-redo/nativecontinuation preservefilter/profile/native"]
fn properties_share_upstream_gesture_and_paid_custody() {
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        let mut s = session(sample, assigned);
        let old = s.active().unwrap().doc.clone();
        let id = old.layers[0].id;
        let past = s.active().unwrap().history.past_len();
        assert_eq!(s.execute("layer.setProps", json!({"layer":id.0,"visible":false,"coalesce":"same\0key"})).unwrap(), Value::Null);
        s.execute("layer.setProps", json!({"layer":id.0,"name":"Own renamed","opacity":0.375,"coalesce":"same\0key"})).unwrap();
        let changed = s.active().unwrap().doc.clone();
        assert!(!changed.layers[0].visible);
        assert_eq!(changed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Visibility"));
        assert_eq!(fixture::native(&changed), fixture::native(&old));
        assert_eq!(changed.source_color_context, old.source_color_context);
        assert_eq!(changed.icc_profile, old.icc_profile);
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks, old.xcf_state.as_ref().unwrap().filter_stacks);
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_masks, old.xcf_state.as_ref().unwrap().filter_masks);
        s.execute("edit.undo", Value::Null).unwrap();
        let h = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        s.execute("layer.setProps", json!({"layer":id.0,"visible":true,"opacity":1.0,"coalesce":"same\0key"})).unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert_eq!(s.active().unwrap().revision, rev);
        s.execute("edit.redo", Value::Null).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&changed));
        s.execute("layer.setProps", json!({"layer":"Own renamed","opacity":-0.0})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers[0].opacity.to_bits(), (-0.0f32).to_bits());
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(fixture::archive(s.active().unwrap().doc.accounted().unwrap()), "continued", None, &fixture::ctl()).unwrap();
        assert_eq!(reopened.active().unwrap().doc.layers[0].opacity.to_bits(), (-0.0f32).to_bits());
        let h = reopened.active().unwrap().history.clone();
        let rev = reopened.active().unwrap().revision;
        reopened.execute("layer.setProps", json!({"visible":"ignored","opacity":false,"name":null})).unwrap();
        assert!(reopened.active().unwrap().history.shares_accounted_history(&h));
        assert_eq!(reopened.active().unwrap().revision, rev);
    }
}
#[test]
#[ignore = "UNRUN: actualimplicit/assignednonobserved compound directcalls failclosed evenallnone/equal; malformed observedversion/epoch cannot fallthrough"]
fn all_actual_point_compound_entrances_require_observed_proof() {
    for assigned in [false, true] {
        for gesture in [false, true] {
            let mut s = internal(owner(SampleType::F32, assigned, false));
            let before = s.active().unwrap().doc.clone();
            let h = s.active().unwrap().history.clone();
            let id = before.layers[0].id;
            for visible in [None, Some(true)] {
                let edit = AccountedEdit::LayerMetadata { layer: id, name: None, visible, opacity: None };
                let r = if gesture {
                    s.edit_accounted_metadata_with_coalesce("direct", edit, Some("x"), &fixture::ctl())
                } else {
                    s.edit_accounted("direct", edit, &fixture::ctl())
                };
                assert!(r.is_err());
                assert!(s.active().unwrap().doc.shares_state(&before));
                assert!(s.active().unwrap().history.shares_accounted_history(&h));
            }
        }
        let base = owner(SampleType::F32, assigned, true);
        for fault in 0..3 {
            let malformed = current(&base, |d| {
                let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                match fault {
                    0 => n.operation.stored_version = "bad".into(),
                    1 => n.operation.schema_epoch = "bad".into(),
                    _ => n.composition.as_mut().unwrap().compatibility_epoch = "bad".into(),
                }
            });
            let mut s = internal(malformed);
            let before = s.active().unwrap().doc.clone();
            assert!(observed_box_layer_metadata_candidate(before.accounted().unwrap()));
            assert!(s.execute("layer.setProps", json!({"name":"bad cannot repair"})).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
    }
}
#[test]
#[ignore = "UNRUN: changed-only importedvisibility locks, name/opacity permission and hidden physicallynonzero savedmask refusal beforeequalnoop"]
fn locks_and_hidden_input_do_not_skip_qualification() {
    for assigned in [false, true] {
        let base = owner(SampleType::F32, assigned, true);
        let locked = current(&base, |d| {
            d.layers[0].locks.all = true;
            let id = d.layers[0].id;
            d.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().locks.visibility = true;
        });
        let mut s = internal(locked);
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute("layer.setProps", json!({"visible":false})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.execute("layer.setProps", json!({"visible":true,"name":"Allowed scalar","opacity":0.5})).unwrap();
        let bad = current(&base, |d| {
            d.layers[0].visible = false;
            let id = d.layers[0].id;
            let x = d.xcf_state.as_mut().unwrap();
            x.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
            x.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.5]).unwrap();
        });
        let mut s = internal(bad);
        let before = s.active().unwrap().doc.clone();
        let h = s.active().unwrap().history.clone();
        assert!(s.execute("layer.setProps", json!({"visible":false})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
    }
}
#[test]
#[ignore = "UNRUN: full prepared history/source/channel/view/journal fences and cumulativework/cancel/quota refuseatomically"]
fn prepared_fences_and_controls_are_atomic() {
    for assigned in [false, true] {
        for fault in 0..5 {
            let mut s = session(SampleType::F32, assigned);
            let before = s.active().unwrap().doc.clone();
            let p = json!({"name":"staged"});
            let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &fixture::ctl()).unwrap();
            match fault {
                0 => s.active_mut().unwrap().history.max_states += 1,
                1 => {
                    let old = s.active().unwrap().source_read_only;
                    s.active_mut().unwrap().source_read_only = !old;
                }
                2 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                3 => s.active_mut().unwrap().view_revision += 1,
                _ => s.journal.push(("external".into(), Value::Null)),
            }
            assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
        let mut s = session(SampleType::F32, assigned);
        let before = s.active().unwrap().doc.clone();
        let h = s.active().unwrap().history.clone();
        let j = s.journal.clone();
        let p = json!({"opacity":0.25});
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &deny).is_err());
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(
            crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) })
                .is_err()
        );
        let q = before.accounted().unwrap().quota().clone();
        let blocker = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.execute("layer.setProps", p).is_err());
        drop(blocker);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert_eq!(s.journal, j);
    }
}
