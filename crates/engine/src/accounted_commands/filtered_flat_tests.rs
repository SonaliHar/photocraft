//! Deferred actual mixed-class transactions. External captures are harness data.
use super::*;
#[path = "../../../ui-egui/src/filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn session(index: usize) -> Session {
    let source = Arc::new(fixture::import(index));
    let owner = fixture::larger(source);
    let mut s = Session::new();
    s.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "mixed.pcraft", None, &fixture::ctl()).unwrap();
    s
}
fn node<'a>(s: &'a Session, op: &str) -> (&'a photocraft_doc::xcf_filters::FilterStack, &'a photocraft_doc::xcf_filters::FilterNode) {
    let stack = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.nodes[0].operation.name == op).unwrap();
    (stack, &stack.nodes[0])
}
fn changed(s: &Session, op: &str) -> Value {
    let (_, n) = node(s, op);
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let argument = if op == "gimp:semi-flatten" {
        let FilterValue::Color { components, encoding, .. } = &n.arguments[0].value else { panic!("color") };
        let ExactNumber::Float32Bits(alpha) = components[3] else { panic!("alpha") };
        let tag = if *encoding == photocraft_doc::SampleEncoding::Linear { "nonlinear_profile" } else { "linear" };
        serde_json::json!({"name":"color","value":{"Color":{"components":[{"Float32Bits":0.125f32.to_bits()},{"Float32Bits":0.25f32.to_bits()},{"Float32Bits":0.375f32.to_bits()},{"Float32Bits":alpha}],"encoding":tag,"profile":null}}})
    } else {
        let name = if op == "gegl:hue-chroma" { "hue" } else { "vibrance" };
        let a = n.arguments.iter().find(|a| a.name == name).unwrap();
        let FilterValue::Number(ExactNumber::Float32Bits(old)) = a.value else { panic!("f32") };
        let value = if old == 0.0f32.to_bits() { 17.0f32 } else { 0.0f32 };
        serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":value.to_bits()}}})
    };
    serde_json::json!({"filterId":n.id.0.to_string(),"arguments":[argument],"enabled":!n.enabled})
}
fn payload(a: &photocraft_doc::Document, b: &photocraft_doc::Document) {
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.xcf_state.as_ref().unwrap().filter_masks, b.xcf_state.as_ref().unwrap().filter_masks);
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(fixture::raw(a), fixture::raw(b));
        let (photocraft_doc::LayerContent::BackedRaster(a), photocraft_doc::LayerContent::BackedRaster(b)) = (&a.content, &b.content) else { panic!("backed") };
        assert_eq!(a.descriptor(), b.descriptor());
        assert_eq!(a.default_bytes(), b.default_bytes());
    }
}
#[test]
#[ignore = "UNRUN:all three nonfirst filtered targets exact atomic typed arguments+enable, neighbor/source/native/profile/mask preservation, raw no-op/history/undo/native continuation; larger CURRENT is synthetic, no GIMP numeric oracle"]
fn mixed_target_edits_are_atomic_and_continue_native() {
    for index in 0..2 {
        let mut s = session(index);
        for op in ["gimp:semi-flatten", "gegl:hue-chroma", "gegl:vibrance"] {
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let (st, n) = node(&s, op);
            let target = n.id;
            let old_revision = st.revision;
            let old_enabled = n.enabled;
            let p = changed(&s, op);
            let reply = s.execute("xcf.filter.set", p.clone()).unwrap();
            assert_eq!(reply, serde_json::json!({"filterId":target.0.to_string(),"updated":true}));
            let after = s.active().unwrap().doc.clone();
            assert_eq!(s.active().unwrap().revision, revision + 1);
            payload(&before, &after);
            for old in &before.xcf_state.as_ref().unwrap().filter_stacks {
                let next = after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == old.owner).unwrap();
                if old.nodes[0].id == target {
                    assert_eq!(next.revision, old_revision + 1);
                    assert_eq!(next.nodes[0].enabled, !old_enabled);
                } else {
                    assert_eq!(next, old);
                }
            }
            assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Edit GIMP Stock Filter"));
            let no = s.active().unwrap().doc.clone();
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            assert!(s.active().unwrap().doc.shares_state(&no));
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            let equal = serde_json::json!({"filterId":target.0.to_string(),"enabled":old_enabled});
            assert_eq!(s.execute("xcf.filter.enable", equal.clone()).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            assert_eq!(s.execute("xcf.filter.set", equal).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            fixture::same(&after, &s.active().unwrap().doc);
            let mut reopened = Session::new();
            reopened.add_owned_native_filtered_flat_archive(fixture::archive(after.accounted().unwrap()), "continued.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&after, &reopened.active().unwrap().doc);
        }
    }
}
#[test]
#[ignore = "UNRUN:all plain/filtered root compound metadata, opaque cross-target key firstlabel/raw opacity/noopredo; original witnessed Rename Layer trim/content/capture/native authority, no new individual L1 mutation grant"]
fn mixed_metadata_and_witnessed_rename_preserve_upstream_semantics() {
    let mut s = session(0);
    let before = s.active().unwrap().doc.clone();
    let a = before.layers[0].id;
    let b = before.layers[2].id;
    s.execute("layer.setProps", serde_json::json!({"layer":a.0,"visible":false,"coalesce":"same\u{0}key"})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":b.0,"name":"mixed target","opacity":-0.0,"coalesce":"same\u{0}key"})).unwrap();
    assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
    let after = s.active().unwrap().doc.clone();
    assert_eq!(after.layer(b).unwrap().opacity.to_bits(), (-0.0f32).to_bits());
    payload(&before, &after);
    s.execute("edit.undo", serde_json::json!({})).unwrap();
    fixture::same(&before, &s.active().unwrap().doc);
    s.execute("layer.setProps", serde_json::json!({"layer":a.0,"visible":true,"coalesce":"same\u{0}key"})).unwrap();
    assert!(s.active().unwrap().history.can_redo());
    s.execute("edit.redo", serde_json::json!({})).unwrap();
    fixture::same(&after, &s.active().unwrap().doc);
    for layer in before.layers.iter().map(|l| l.id).collect::<Vec<_>>() {
        let rev = s.active().unwrap().revision;
        let doc = s.active().unwrap().doc.id;
        let p = serde_json::json!({"layerId":layer.0.to_string(),"name":"  Renamed root  ","documentId":doc.0.to_string(),"expectedRevision":rev.to_string()});
        s.execute("layer.renameLayer", p.clone()).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(layer).unwrap().name, "Renamed root");
        assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Rename Layer"));
        assert!(s.execute("layer.renameLayer", p).is_err());
    }
    let baseline = s.active().unwrap().doc.clone();
    for (id, p) in [
        ("layer.translate", serde_json::json!({"dx":0})),
        ("xcf.filter.remove", serde_json::json!({"filterId":node(&s,"gegl:hue-chroma").1.id.0.to_string()})),
        ("layer.select", serde_json::json!({"index":0})),
    ] {
        assert!(s.execute(id, p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&baseline));
    }
}
#[test]
#[ignore = "UNRUN:paid history-mismatch equal activation, stale prepared fences, all-root malformed hidden proof and lock/MAX/Work/cancel rollback using same q genuine source/native owners; external callback counters only plumbing"]
fn mixed_equal_and_changed_requests_keep_full_proofs_and_fences() {
    let mut s = session(0);
    let (_, n) = node(&s, "gegl:hue-chroma");
    let p = serde_json::json!({"filterId":n.id.0.to_string(),"enabled":n.enabled});
    let before = s.active().unwrap().doc.clone();
    let prepared = Request::parse(&s, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let mut s = session(0);
    let (_, n) = node(&s, "gegl:hue-chroma");
    let p = serde_json::json!({"filterId":n.id.0.to_string(),"enabled":n.enabled});
    let titled = s.active().unwrap().doc.accounted().unwrap().try_with_document_name("paid mismatch", &fixture::ctl()).unwrap();
    let st = s.active().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&titled, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &fixture::ctl())
        .unwrap();
    s.active_mut().unwrap().accounted_view = Some(view);
    s.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(titled).unwrap();
    assert!(Request::parse(&s, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).is_err());
    let mut s = session(0);
    let p = changed(&s, "gimp:semi-flatten");
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let calls = AtomicU64::new(0);
    let deny = |_: u64| {
        calls.fetch_add(1, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
    assert!(Request::parse(&s, "xcf.filter.set", &p, &c).is_err());
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    let stop = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(Request::parse(&s, "xcf.filter.set", &p, &c).is_err());
    // Actual paid native CURRENT lock variant: equal activation still checks locks.
    let locked_session = session(0);
    let original = locked_session.active().unwrap().doc.clone();
    let (stack, n) = node(&locked_session, "gegl:hue-chroma");
    let ItemRef::Layer(layer) = stack.owner else { panic!("root") };
    let equal = serde_json::json!({"filterId":n.id.0.to_string(),"enabled":n.enabled});
    let locked = fixture::current(original.accounted().unwrap(), |d| d.layer_mut(layer).unwrap().locks.pixels = true);
    let mut locked_session = Session::new();
    locked_session.add_owned_native_filtered_flat_archive(fixture::archive(&locked), "locked.pcraft", None, &fixture::ctl()).unwrap();
    let before_locked = locked_session.active().unwrap().doc.clone();
    assert!(locked_session.execute("xcf.filter.enable", equal).is_err());
    assert!(locked_session.active().unwrap().doc.shares_state(&before_locked));
    // Coherent paid history/view around an intentionally invalid inactive OTHER root.
    let mut invalid_session = session(0);
    let baseline = invalid_session.active().unwrap().doc.clone();
    let bad = fixture::current(baseline.accounted().unwrap(), |d| {
        let layer = d.layers[3].id;
        d.layers[3].visible = false;
        let x = d.xcf_state.as_mut().unwrap();
        x.items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().visible = false;
        let stack = x.filter_stacks.iter_mut().find(|s| s.owner == ItemRef::Layer(layer)).unwrap();
        stack.nodes[0].enabled = false;
        stack.nodes[0].arguments[0].value =
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits()));
    });
    let st = invalid_session.active().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &fixture::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "external invalid current", view.target(), &fixture::ctl()).unwrap();
    let st = invalid_session.active_mut().unwrap();
    st.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    st.accounted_view = Some(view);
    st.history = history;
    let (_, n) = node(&invalid_session, "gegl:hue-chroma");
    let equal = serde_json::json!({"filterId":n.id.0.to_string(),"enabled":n.enabled});
    assert!(invalid_session.active().unwrap().history.current().unwrap().shares_state(&invalid_session.active().unwrap().doc));
    assert!(Request::parse(&invalid_session, "xcf.filter.enable", &equal, &fixture::ctl()).unwrap().prepare(&invalid_session, &fixture::ctl()).is_err());
    let (_, n) = node(&s, "gegl:hue-chroma");
    let equal = serde_json::json!({"filterId":n.id.0.to_string(),"enabled":n.enabled});
    s.active_mut().unwrap().revision = u64::MAX;
    assert!(s.execute("xcf.filter.enable", equal).is_err());
    assert!(s.execute("xcf.filter.set", p).is_err());
}
