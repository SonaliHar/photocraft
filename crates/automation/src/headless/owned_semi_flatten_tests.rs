//! Deferred exact authored RGB literal/native lifecycle; no numerical acceptance.
use super::*;
#[path = "../../../ui-egui/src/semi_flatten_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use photocraft_doc::{SampleEncoding, xcf_filters::FilterValue};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-semi-flatten-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "UNRUN: 12 actual RGB F32/F64 Linear/NL Color Semi-Flatten source/edit files through paid source/native/query/edit/enable/history/individual metadata/title/save; exact raw source/native/profile/Color words, auth/waitfalse/failure controls, no CSS/Gray/HDR/Move/shared gesture grants or numerical acceptance"]
fn authored_semi_flatten_source_and_current_native_continue_with_exact_color_roles() {
    for case in fixture::cases() {
        let directory = Directory::new();
        let mut source = Headless::trusted_local();
        drop(source.open_owned_xcf_keep_current_bounded(&case.path).unwrap());
        let original = source.session.active().unwrap().doc.clone();
        fixture::observed(&original, &case);
        assert!(source.session.active().unwrap().source_read_only);
        let protected = directory.0.join("source-copy.pcraft");
        std::fs::write(&protected, &case.bytes).unwrap();
        assert!(source.save_owned_native_active(Some(&protected), Some("pcraft"), &fixture::ctl()).is_err());
        assert_eq!(std::fs::read(&protected).unwrap(), case.bytes);
        let initial = directory.0.join("original.pcraft");
        source.save_owned_native_active(Some(&initial), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&initial).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
        fixture::storage(&original, &before);
        let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let color = fixture::color(&before).clone();
        let changed = fixture::changed(&color);
        let wire = fixture::wire(&filter, &changed);
        assert!(h.command_start("xcf.filter.set", wire.clone(), false).unwrap_err().to_string().contains("synchronous"));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("semi edit denied".into())));
        assert!(h.command_start("xcf.filter.set", wire.clone(), true).unwrap_err().to_string().contains("semi edit denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(h.command_start("xcf.filter.set", wire, true).unwrap()["updated"], true);
        let edited = h.session.active().unwrap().doc.clone();
        fixture::storage(&before, &edited);
        assert_eq!(fixture::color(&edited), &changed);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        h.command_run("edit.undo", Value::Null).unwrap();
        let history = h.session.active().unwrap().history.clone();
        assert_eq!(h.command_run("xcf.filter.set", fixture::wire(&filter, &color)).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&edited));
        assert_eq!(fixture::color_bits(&changed).0[3], case.color[3]);
        assert_eq!(fixture::color_bits(&changed).1, case.encoding);
        // Deliberate CURRENT encoding transition, not an authored readback alias.
        let mut opposite = changed.clone();
        if let FilterValue::Color { encoding, .. } = &mut opposite {
            *encoding = if *encoding == SampleEncoding::Linear { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        }
        h.command_run("xcf.filter.set", fixture::wire(&filter, &opposite)).unwrap();
        assert_eq!(fixture::color(&h.session.active().unwrap().doc), &opposite);
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&edited));
        h.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":false})).unwrap();
        let disabled = h.session.active().unwrap().doc.clone();
        fixture::storage(&edited, &disabled);
        let output = directory.0.join("disabled.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut continued = Headless::trusted_local();
        drop(continued.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::storage(&disabled, &continued.session.active().unwrap().doc);
        assert!(!continued.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        continued.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":true})).unwrap();
        let anchor = continued.session.active().unwrap().doc.clone();
        for value in [
            json!({"Color":{"components":[{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":f32::NAN.to_bits()}],"encoding":"linear","profile":null}}),
            json!({"Color":{"components":vec![json!({"Float32Bits":0});4],"encoding":"perceptual","profile":null}}),
            json!({"Color":{"components":vec![json!({"Float32Bits":0});4],"encoding":"linear","profile":[1]}}),
            json!({"Color":{"components":vec![json!({"Float64Bits":"0x0000000000000000"});4],"encoding":"linear","profile":null}}),
        ] {
            assert!(continued.command_run("xcf.filter.set", json!({"filterId":filter,"arguments":[{"name":"color","value":value}]})).is_err());
        }
        assert!(continued.session.active().unwrap().doc.shares_state(&anchor));
        continued.command_run("layer.setProps", json!({"name":"Semi current"})).unwrap();
        continued.command_run("layer.setProps", json!({"visible":false})).unwrap();
        continued.command_run("edit.undo", Value::Null).unwrap();
        continued.command_run("layer.setProps", json!({"name":"Semi combined CURRENT","visible":true,"opacity":0.375})).unwrap();
        let current = continued.session.active().unwrap().doc.clone();
        fixture::storage(&anchor, &current);
        for (id, p) in [
            ("layer.translate", json!({"dx":1})),
            ("layer.setProps", json!({"blend":"multiply"})),
            ("select.all", Value::Null),
            ("xcf.filter.set", json!({"filterId":filter,"arguments":[{"name":"unknown","value":{"Bool":true}}]})),
        ] {
            assert!(continued.command_run(id, p).is_err());
        }
        assert!(continued.session.active().unwrap().doc.shares_state(&current));
        let query = continued.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":filter}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gimp:semi-flatten");
        assert_eq!(query.value()["argumentCount"], 1);
        assert_eq!(query.value()["arguments"][0]["state"]["value"], serde_json::to_value(&changed).unwrap());
        drop(query);
        let project = continued.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &fixture::ctl()).unwrap();
        assert_eq!(project.value()["items"]["total"], 2);
        assert_eq!(project.value()["filters"]["total"], 1);
        drop(project);
        let titled = current.accounted().unwrap().try_with_document_name("Semi native continued", &fixture::ctl()).unwrap();
        // External encoded capture; actual reader/publication remains genuinely paid.
        let mut final_session = photocraft_engine::Session::new();
        final_session.add_owned_native_point_archive(fixture::archive(&titled), "continued.pcraft", None, &fixture::ctl()).unwrap();
        let final_doc = &final_session.active().unwrap().doc;
        fixture::storage(&current, final_doc);
        assert_eq!(final_doc.name, "Semi native continued");
        assert_eq!(fixture::color(final_doc), &changed);
        // Separate original-state singleton stack lifecycle, never a new empty source.
        let mut stack = Headless::trusted_local();
        drop(stack.open_owned_native_keep_current_bounded(&initial).unwrap());
        let retained = stack.session.active().unwrap().doc.clone();
        let history = stack.session.active().unwrap().history.clone();
        assert_eq!(stack.command_run("xcf.filter.move", json!({"filterId":filter,"index":0})).unwrap()["updated"], false);
        assert!(stack.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(stack.command_run("xcf.filter.remove", json!({"filterId":filter})).unwrap()["updated"], true);
        let empty = stack.session.active().unwrap().doc.clone();
        let (old_state, empty_state) = (retained.xcf_state.as_ref().unwrap(), empty.xcf_state.as_ref().unwrap());
        assert_eq!(empty_state.filter_stacks.len(), 1);
        assert!(empty_state.filter_stacks[0].nodes.is_empty());
        assert_eq!(empty_state.filter_stacks[0].owner, old_state.filter_stacks[0].owner);
        assert_eq!(empty_state.filter_stacks[0].revision, old_state.filter_stacks[0].revision + 1);
        assert!(empty_state.filter_masks.is_empty());
        assert_eq!(empty_state.items.len(), 1);
        assert_eq!(fixture::native_bytes(&empty), fixture::native_bytes(&retained));
        assert_eq!(empty.icc_profile, retained.icc_profile);
        assert_eq!(empty.source_color_context, retained.source_color_context);
        assert_eq!(empty_state.provenance, old_state.provenance);
        assert_eq!(empty_state.opaque.len(), old_state.opaque.len());
        let removed_mask = old_state.filter_masks[0].id;
        for (old, new) in old_state.opaque.iter().zip(&empty_state.opaque) {
            assert_eq!(old.bytes, new.bytes);
            assert_eq!(old.blake3, new.blake3);
            assert_eq!(old.namespace, new.namespace);
            assert_eq!(old.source_version, new.source_version);
            assert_eq!(old.order, new.order);
            assert_eq!(old.flags, new.flags);
            assert_eq!(old.rendering_required, new.rendering_required);
            if old.owner == Some(photocraft_doc::item_identity::ItemRef::Mask(removed_mask)) {
                assert!(new.owner.is_none());
                assert_eq!(new.identity, format!("deleted-filter-mask/{}/{}", removed_mask.0, old.identity));
            } else {
                assert_eq!(old.owner, new.owner);
                assert_eq!(old.identity, new.identity);
            }
        }
        assert_eq!(stack.session.active().unwrap().history.undo_label(), Some("Remove GIMP Stock Filter"));
        assert!(stack.command_run("xcf.filter.remove", json!({"filterId":filter})).is_err());
        stack.command_run("edit.undo", Value::Null).unwrap();
        assert!(stack.session.active().unwrap().doc.shares_state(&retained));
        let history = stack.session.active().unwrap().history.clone();
        assert_eq!(stack.command_run("xcf.filter.set", fixture::wire(&filter, fixture::color(&retained))).unwrap()["updated"], false);
        assert!(stack.session.active().unwrap().history.shares_accounted_history(&history));
        stack.command_run("edit.redo", Value::Null).unwrap();
        assert!(stack.session.active().unwrap().doc.shares_state(&empty));
        let empty_path = directory.0.join("removed.pcraft");
        stack.save_owned_native_active(Some(&empty_path), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut empty_reopen = Headless::trusted_local();
        drop(empty_reopen.open_owned_native_keep_current_bounded(&empty_path).unwrap());
        assert_eq!(empty_reopen.session.active().unwrap().doc.xcf_state, empty.xcf_state);
        assert_eq!(fixture::native_bytes(&empty_reopen.session.active().unwrap().doc), fixture::native_bytes(&empty));
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
        assert!(!continued.session.has_jobs());
    }
}
