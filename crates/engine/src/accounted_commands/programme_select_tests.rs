//! Deferred registered view-only focus, not document or pixel selection edits.
use super::*;
#[path = "programme_select_test_support.rs"]
mod f;
use serde_json::json;
#[test]
#[ignore = "UNRUN/uncompiled four genuine ordered/masked F32/F64 sources, registered secondroot replace-focus, same model/programmes/native/profile/source, upstream revision/clean/history/journal/Null including equal target; no new content authority."]
fn mixed_focus_changes_only_the_existing_paid_view_bookkeeping() {
    for mask in [false, true] {
        for depth in 0..2 {
            let mut s = f::session(mask, depth);
            let (first, second) = f::roots(&s.active().unwrap().doc);
            s.execute("layer.select", json!({"layer":first.0})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
            let raw = before.layers.iter().map(|l| (l.id, f::programmes::raw(&before, l.id))).collect::<Vec<_>>();
            for _ in 0..2 {
                let st = s.active().unwrap();
                let revision = st.revision;
                let saved = st.saved_revision;
                let clean = revision == saved;
                let view_revision = st.view_revision;
                let past = st.history.past_len();
                let redo = st.history.can_redo();
                let source = st.source.clone();
                let protected = st.source_read_only;
                let journal = s.journal.len();
                assert_eq!(s.execute("layer.select", json!({"layer":second.0,"mode":"replace"})).unwrap(), Value::Null);
                let st = s.active().unwrap();
                assert!(before.shares_state(&st.doc));
                assert_eq!(st.active_layer, Some(second));
                assert_eq!(st.layer_anchor, Some(second));
                assert!(st.selected_layers.is_empty());
                let view = st.accounted_view.as_ref().unwrap();
                assert_eq!(view.target().active, Some(second));
                assert_eq!(view.selected(), &[second]);
                assert_eq!(view.anchor(), Some(second));
                assert_eq!(st.revision, revision + 1);
                assert_eq!(st.saved_revision, if clean { revision + 1 } else { saved });
                assert_eq!(st.view_revision, view_revision);
                assert_eq!(st.history.past_len(), past);
                assert_eq!(st.history.can_redo(), redo);
                assert_eq!(st.source, source);
                assert_eq!(st.source_read_only, protected);
                assert_eq!(s.journal.len(), journal + 1);
                witness.preserved(&st.doc, second, &f::ctl());
                for (id, bytes) in &raw {
                    assert_eq!(f::programmes::raw(&st.doc, *id), *bytes);
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled mixed focus captured source/path/readOnly/view/channel/history/prefs fences without model revision, foreign/wrongwire and inactive invalidCurrent refusal, early Work/cancel and unchanged model/history/journal; no late cancellation claim."]
fn mixed_focus_refuses_stale_context_or_unsupported_selection_atomically() {
    for mask in [false, true] {
        for variant in 0..7 {
            let mut s = f::session(mask, 0);
            let (_, second) = f::roots(&s.active().unwrap().doc);
            let before = s.active().unwrap().doc.clone();
            let p = json!({"layer":second.0});
            let prepared = super::super::PreparedAccountedCommand::prepare(&s, "layer.select", &p, &f::ctl()).unwrap();
            let journal = s.journal.clone();
            match variant {
                0 => s.active_mut().unwrap().source.as_mut().unwrap().label.push_str(" changed"),
                1 => s.active_mut().unwrap().path = Some("foreign.pcraft".into()),
                2 => {
                    let ro = s.active().unwrap().source_read_only;
                    s.active_mut().unwrap().source_read_only = !ro
                }
                3 => s.active_mut().unwrap().view_revision += 1,
                4 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                5 => s.active_mut().unwrap().history.set_current_label("external metadata", &f::ctl()).unwrap(),
                _ => s.prefs.edit(|p| p.performance.history_states = 1),
            }
            assert!(prepared.execute(&mut s, &f::ctl()).is_err());
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(s.journal, journal);
        }
        let mut s = f::session(mask, 1);
        let before = s.active().unwrap().doc.clone();
        let (_, second) = f::roots(&before);
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        for p in [
            json!({"layer":0}),
            json!({"layer":u64::MAX}),
            json!({"index":1}),
            json!({"layer":"Own root 1"}),
            json!({"layer":second.0,"mode":"add"}),
            json!({"layer":second.0,"mode":"toggle"}),
            json!({"layer":second.0,"mode":"range"}),
            json!({"layer":second.0,"mode":"remove"}),
            json!({"layer":second.0,"mode":"replace","documentId":"1"}),
        ] {
            assert!(s.execute("layer.select", p).is_err());
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert!(history.shares_accounted_history(&s.active().unwrap().history));
            assert_eq!(s.journal, journal);
        }
        let p = json!({"layer":second.0});
        let deny = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(super::super::PreparedAccountedCommand::prepare(&s, "layer.select", &p, &c).is_err());
        let yes = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&yes);
        assert!(super::super::PreparedAccountedCommand::prepare(&s, "layer.select", &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
    }
    // Paid intentionally malformed CURRENT fixture: disabled node arguments are
    // still part of complete programme validation, even for an equal focus.
    let mut s = f::session(false, 0);
    let old = s.active().unwrap().doc.accounted().unwrap().clone();
    let q = old.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = old.document().clone();
    let node = d
        .xcf_state
        .as_mut()
        .unwrap()
        .filter_stacks
        .iter_mut()
        .flat_map(|st| &mut st.nodes)
        .find(|n| {
            n.arguments
                .iter()
                .any(|a| matches!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))
        })
        .unwrap();
    node.enabled = false;
    node.arguments
        .iter_mut()
        .find(|a| matches!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))))
        .unwrap()
        .value = photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits()));
    let bad = AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, std::sync::Arc::new((old, index, resident)), q, &f::ctl()).unwrap();
    let st = s.active_mut().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &f::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "invalid external CURRENT", view.target(), &f::ctl()).unwrap();
    st.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    st.accounted_view = Some(view);
    st.history = history;
    let before = s.active().unwrap().doc.clone();
    let layer = s.active().unwrap().active_layer.unwrap();
    assert!(s.execute("layer.select", json!({"layer":layer.0})).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
}
