//! Synthetic native CURRENT compatibility, not independently authored geometry.
use super::*;
#[path = "../../../ui-egui/src/pure_point_geometry_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::Family;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-origin-point-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: genuine paid synthetic origin257x259/1x257 native CURRENT inputs for Semi/Hue/Vibrance F32/F64, registered params+enable/readonlyprops/history/query/title/save/reopen/remove-empty and auth/waitfalse/offset/inactive-invalidinput refusal; no larger producer source/geometry, display oracle or numerical/profile acceptance"]
fn origin_pure_point_native_current_commands_preserve_custody() {
    for family in [Family::Semi, Family::Hue, Family::Vibrance] {
        for sample in [photocraft_doc::SampleType::F32, photocraft_doc::SampleType::F64] {
            for variant in 0..2 {
                let directory = Directory::new();
                let owner = fixture::fixture(family, sample, variant);
                let input = directory.0.join("synthetic-current.pcraft");
                let bytes = fixture::encoded(&owner); // External harness capture; writer and reader are genuinely paid.
                std::fs::write(&input, &bytes).unwrap();
                let mut h = Headless::trusted_local();
                drop(h.open_owned_native_keep_current_bounded(&input).unwrap());
                let before = h.session.active().unwrap().doc.clone();
                fixture::storage(owner.document(), &before);
                assert!(before.accounted().unwrap().is_readonly_point_storage());
                let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
                let params = family.changed(&filter);
                assert!(h.command_start("xcf.filter.set", params.clone(), false).unwrap_err().to_string().contains("synchronous"));
                h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("origin edit denied".into())));
                assert!(h.command_start("xcf.filter.set", params.clone(), true).unwrap_err().to_string().contains("origin edit denied"));
                h.session.authorize = None;
                assert!(h.session.active().unwrap().doc.shares_state(&before));
                let past = h.session.active().unwrap().history.past_len();
                assert_eq!(h.command_start("xcf.filter.set", params, true).unwrap()["updated"], true);
                let disabled = h.session.active().unwrap().doc.clone();
                fixture::storage(&before, &disabled);
                assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
                assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
                assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
                h.command_run("edit.undo", Value::Null).unwrap();
                let history = h.session.active().unwrap().history.clone();
                assert_eq!(
                    h.command_run("xcf.filter.set", json!({"filterId":filter,"arguments":family.arguments(),"enabled":true})).unwrap()["updated"],
                    false
                );
                assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
                h.command_run("edit.redo", Value::Null).unwrap();
                assert!(h.session.active().unwrap().doc.shares_state(&disabled));
                let output = directory.0.join("disabled.pcraft");
                h.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
                let mut continued = Headless::trusted_local();
                drop(continued.open_owned_native_keep_current_bounded(&output).unwrap());
                fixture::storage(&disabled, &continued.session.active().unwrap().doc);
                assert_eq!(continued.session.active().unwrap().doc.xcf_state, disabled.xcf_state);
                continued.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":true})).unwrap();
                continued.command_run("layer.setProps", json!({"name":"Origin CURRENT metadata","visible":true,"opacity":0.375})).unwrap();
                let current = continued.session.active().unwrap().doc.clone();
                fixture::storage(&before, &current);
                for (command, params) in [
                    ("layer.translate", json!({"dx":1})),
                    ("layer.setProps", json!({"blend":"multiply"})),
                    ("select.all", Value::Null),
                    ("xcf.filter.set", json!({"filterId":filter,"arguments":[{"name":"unknown","value":{"Bool":true}}]})),
                ] {
                    assert!(continued.command_run(command, params).is_err());
                }
                assert!(continued.session.active().unwrap().doc.shares_state(&current));
                let query =
                    continued.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":filter}), false, &fixture::ctl()).unwrap();
                assert_eq!(query.value()["operation"]["name"], family.name());
                drop(query);
                let query = continued.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &fixture::ctl()).unwrap();
                assert_eq!(query.value()["items"]["total"], 2);
                assert_eq!(query.value()["filters"]["total"], 1);
                drop(query);
                let titled = current.accounted().unwrap().try_with_document_name("Origin native continuation", &fixture::ctl()).unwrap();
                let mut final_session = photocraft_engine::Session::new();
                final_session.add_owned_native_point_archive(fixture::archive(&titled), "continued.pcraft", None, &fixture::ctl()).unwrap();
                fixture::storage(&current, &final_session.active().unwrap().doc);
                assert_eq!(final_session.active().unwrap().doc.name, "Origin native continuation");
                // Separate derived empty CURRENT role, no recovered operation/source identity.
                assert_eq!(continued.command_run("xcf.filter.move", json!({"filterId":filter,"index":0})).unwrap()["updated"], false);
                continued.command_run("xcf.filter.remove", json!({"filterId":filter})).unwrap();
                let empty = continued.session.active().unwrap().doc.clone();
                let state = empty.xcf_state.as_ref().unwrap();
                assert!(state.filter_stacks[0].nodes.is_empty());
                assert!(state.filter_masks.is_empty());
                assert_eq!(state.items.len(), 1);
                assert_eq!(state.filter_stacks[0].revision, current.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
                assert_eq!(fixture::raw(&empty), fixture::raw(&current));
                assert_eq!(empty.icc_profile, current.icc_profile);
                assert_eq!(empty.source_color_context, current.source_color_context);
                continued.command_run("edit.undo", Value::Null).unwrap();
                assert!(continued.session.active().unwrap().doc.shares_state(&current));
                let history = continued.session.active().unwrap().history.clone();
                assert_eq!(continued.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":true})).unwrap()["updated"], false);
                assert!(continued.session.active().unwrap().history.shares_accounted_history(&history));
                continued.command_run("edit.redo", Value::Null).unwrap();
                let removed = directory.0.join("removed.pcraft");
                continued.save_owned_native_active(Some(&removed), Some("pcraft"), &fixture::ctl()).unwrap();
                let mut empty_reopen = Headless::trusted_local();
                drop(empty_reopen.open_owned_native_keep_current_bounded(&removed).unwrap());
                assert_eq!(empty_reopen.session.active().unwrap().doc.xcf_state, empty.xcf_state);
                assert_eq!(fixture::raw(&empty_reopen.session.active().unwrap().doc), fixture::raw(&empty));
                assert_eq!(std::fs::read(&input).unwrap(), bytes);
                assert!(!continued.session.has_jobs());
            }
            for invalid in [fixture::offset_fixture(family, sample), fixture::malformed_fixture(family, sample)] {
                let directory = Directory::new();
                let path = directory.0.join("invalid.pcraft");
                let bytes = fixture::encoded(&invalid);
                std::fs::write(&path, &bytes).unwrap();
                let mut refused = Headless::trusted_local();
                assert!(refused.open_owned_native_keep_current_bounded(&path).is_err());
                assert!(refused.session.active().is_none());
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            }
        }
    }
}
