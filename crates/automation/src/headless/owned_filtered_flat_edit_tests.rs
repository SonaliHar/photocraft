//! Deferred public mixed CURRENT workflows. Captures are external harness data.
use super::*;
#[path = "../../../ui-egui/src/filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-edit-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn payload(a: &photocraft_doc::Document, b: &photocraft_doc::Document) {
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.xcf_state.as_ref().unwrap().filter_masks, b.xcf_state.as_ref().unwrap().filter_masks);
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(fixture::raw(a), fixture::raw(b));
    }
}
#[test]
#[ignore = "UNRUN:two genuine source baselines→paid larger CURRENT native, all three target-aware set/enable and compound metadata/witnessed rename, auth/wait/undo/noopredo/query/source/nativeSaveAs continuation. External CURRENT captures are not GIMP numerical oracles."]
fn mixed_headless_targeted_edits_keep_native_source_and_protocol_fences() {
    for index in 0..2 {
        let directory = Directory::new();
        let source_path = fixture::path(index);
        let source_bytes = std::fs::read(&source_path).unwrap();
        let mut authored = Headless::trusted_local();
        drop(authored.open_owned_xcf_keep_current_bounded(&source_path).unwrap());
        fixture::observed(&authored.session.active().unwrap().doc, index);
        let source = Arc::new(fixture::import(index));
        let owner = fixture::larger(source);
        let input = directory.0.join("larger.pcraft");
        std::fs::write(&input, fixture::encoded(&owner)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&input).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        let filters = original.xcf_state.as_ref().unwrap().filter_stacks.iter().map(|st| (st.owner, st.nodes[0].clone())).collect::<Vec<_>>();
        for (owner, n) in filters {
            let photocraft_doc::item_identity::ItemRef::Layer(layer) = owner else { panic!("root") };
            let id = n.id.0.to_string();
            let off = json!({"filterId":id,"enabled":false});
            let before = h.session.active().unwrap().doc.clone();
            assert!(h.command_start("xcf.filter.enable", off.clone(), false).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("mixed edit denied".into())));
            assert!(h.command_run("xcf.filter.enable", off.clone()).is_err());
            h.session.authorize = None;
            let reply = h.command_start("xcf.filter.enable", off, true).unwrap();
            assert_eq!(reply["updated"], true);
            let before_patch = h.session.active().unwrap().doc.clone();
            use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
            let argument = match n.operation.name.as_str() {
                "gimp:semi-flatten" => {
                    let FilterValue::Color { components, .. } = &n.arguments[0].value else { panic!("color") };
                    let ExactNumber::Float32Bits(alpha) = components[3] else { panic!("alpha") };
                    json!({"name":"color","value":{"Color":{"components":[{"Float32Bits":0.125f32.to_bits()},{"Float32Bits":0.25f32.to_bits()},{"Float32Bits":0.375f32.to_bits()},{"Float32Bits":alpha}],"encoding":"nonlinear_profile","profile":null}}})
                }
                "gegl:hue-chroma" => json!({"name":"hue","value":{"Number":{"Float32Bits":0.0f32.to_bits()}}}),
                "gegl:vibrance" => json!({"name":"vibrance","value":{"Number":{"Float32Bits":0.0f32.to_bits()}}}),
                _ => panic!("family"),
            };
            let patch = json!({"filterId":id,"arguments":[argument],"enabled":true});
            let revision = h.session.active().unwrap().revision;
            assert_eq!(h.command_run("xcf.filter.set", patch.clone()).unwrap()["updated"], true);
            assert_eq!(h.session.active().unwrap().revision, revision + 1);
            let changed = h.session.active().unwrap().doc.clone();
            payload(&before_patch, &changed);
            assert_eq!(h.command_run("xcf.filter.set", patch).unwrap()["updated"], false);
            h.command_run("edit.undo", json!({})).unwrap();
            fixture::same(&before_patch, &h.session.active().unwrap().doc);
            assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
            assert!(h.session.active().unwrap().history.can_redo());
            h.command_run("edit.redo", json!({})).unwrap();
            fixture::same(&changed, &h.session.active().unwrap().doc);
            let inspect = h.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":id}), false, &fixture::ctl()).unwrap();
            assert_eq!(inspect.value()["filterId"], id);
            assert_eq!(inspect.value()["scalarEditCandidate"], true);
            drop(inspect);
            let before = h.session.active().unwrap().doc.clone();
            assert!(h.command_start("layer.setProps", json!({"layer":layer.0,"opacity":0.5}), false).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        let before = h.session.active().unwrap().doc.clone();
        let a = before.layers[0].id;
        let b = before.layers[3].id;
        h.command_run("layer.setProps", json!({"layer":a.0,"visible":false,"coalesce":"gesture"})).unwrap();
        h.command_run("layer.setProps", json!({"layer":b.0,"name":"Mixed top","opacity":-0.0,"coalesce":"gesture"})).unwrap();
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        let edited = h.session.active().unwrap().doc.clone();
        assert_eq!(edited.layer(b).unwrap().opacity.to_bits(), (-0.0f32).to_bits());
        h.command_run("edit.undo", json!({})).unwrap();
        fixture::same(&before, &h.session.active().unwrap().doc);
        h.command_run("edit.redo", json!({})).unwrap();
        fixture::same(&edited, &h.session.active().unwrap().doc);
        let st = h.session.active().unwrap();
        let p = json!({"layerId":a.0.to_string(),"documentId":st.doc.id.0.to_string(),"expectedRevision":st.revision.to_string(),"name":"  Named plain  "});
        assert!(h.command_start("layer.renameLayer", p.clone(), false).is_err());
        h.command_run("layer.renameLayer", p.clone()).unwrap();
        assert_eq!(h.session.active().unwrap().doc.layer(a).unwrap().name, "Named plain");
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Rename Layer"));
        assert!(h.command_run("layer.renameLayer", p).is_err());
        let before = h.session.active().unwrap().doc.clone();
        for (id, p) in [
            ("layer.translate", json!({"dx":0})),
            ("xcf.filter.remove", json!({"filterId":before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string(),"index":0})),
            ("layer.select", json!({"index":0})),
            ("layer.setProps", json!({"blend":"multiply"})),
        ] {
            assert!(h.command_run(id, p).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        let saved = directory.0.join("edited.pcraft");
        h.save_owned_native_active(Some(&saved), Some("pcraft"), &fixture::ctl()).unwrap();
        let after_save = h.session.active().unwrap().doc.clone();
        assert_eq!(after_save.name, "edited.pcraft");
        payload(&original, &after_save);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&saved).unwrap());
        fixture::same(&after_save, &reopened.session.active().unwrap().doc);
        let query = reopened.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":2}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 3);
        drop(query);
        assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
        assert!(!reopened.session.has_jobs());
    }
}
