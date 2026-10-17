//! UNRUN actual implicit Linear source and explicit CURRENT/native command lanes.
use super::*;
#[path = "../../../ui-egui/src/linear_box_fixture.rs"]
#[allow(dead_code)]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-implicit-linear-box-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn radius(id: &str, value: i32, enabled: bool) -> Value {
    json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":value}}}}],"enabled":enabled})
}
#[test]
#[ignore = "UNRUN: four authored implicitLinear Box original/edit files through held source/native/query, exact HDR/signedzero words/profile/source guards; existing radius/activation/metadata/Move/undo/noopredo/auth/wait and native continuation, no eager/otherfamily grant"]
fn implicit_linear_box_source_native_and_current_commands_keep_exact_storage() {
    for (path, bytes, old_radius) in fixture::cases() {
        assert!(fixture::try_import("malformed-linear.xcf", fixture::malformed_radius(&bytes, old_radius)).is_err());
        let directory = Directory::new();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&path).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original);
        assert!(original.accounted().unwrap().is_readonly_point_storage());
        assert!(h.session.active().unwrap().source_read_only);
        let protected = directory.0.join("source-copy.pcraft");
        std::fs::write(&protected, &bytes).unwrap();
        assert!(h.save_owned_native_active(Some(&protected), Some("pcraft"), &fixture::ctl()).is_err());
        assert_eq!(std::fs::read(&protected).unwrap(), bytes);
        let initial = directory.0.join("original.pcraft");
        h.save_owned_native_active(Some(&initial), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut current = Headless::trusted_local();
        drop(current.open_owned_native_keep_current_bounded(&initial).unwrap());
        let before = current.session.active().unwrap().doc.clone();
        fixture::exact(&original, &before, 0, 0);
        fixture::observed(&before);
        let layer = before.layers[0].id;
        let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let target = if old_radius == 4 { 5 } else { 4 };
        let patch = radius(&filter, target, true);
        assert!(current.command_start("xcf.filter.set", patch.clone(), false).unwrap_err().to_string().contains("synchronous"));
        current.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("implicit Linear edit denied".into())));
        assert!(current.command_start("xcf.filter.set", patch.clone(), true).unwrap_err().to_string().contains("implicit Linear edit denied"));
        current.session.authorize = None;
        assert!(current.session.active().unwrap().doc.shares_state(&before));
        let past = current.session.active().unwrap().history.past_len();
        assert_eq!(current.command_start("xcf.filter.set", patch, true).unwrap()["updated"], true);
        let edited = current.session.active().unwrap().doc.clone();
        fixture::native_roles(&before, &edited);
        fixture::observed(&edited);
        assert_eq!(current.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(current.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        current.command_run("edit.undo", Value::Null).unwrap();
        let history = current.session.active().unwrap().history.clone();
        assert_eq!(current.command_run("xcf.filter.set", radius(&filter, old_radius, true)).unwrap()["updated"], false);
        assert!(current.session.active().unwrap().history.shares_accounted_history(&history));
        current.command_run("edit.redo", Value::Null).unwrap();
        assert!(current.session.active().unwrap().doc.shares_state(&edited));
        assert_eq!(current.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":false})).unwrap()["updated"], true);
        let disabled = current.session.active().unwrap().doc.clone();
        fixture::native_roles(&edited, &disabled);
        current.command_run("edit.undo", Value::Null).unwrap();
        assert!(current.session.active().unwrap().doc.shares_state(&edited));
        // Endpoint edits keep the same existing wire and full CURRENT checks.
        current.command_run("xcf.filter.set", radius(&filter, 0, true)).unwrap();
        fixture::observed(&current.session.active().unwrap().doc);
        current.command_run("xcf.filter.set", radius(&filter, 1000, false)).unwrap();
        fixture::observed(&current.session.active().unwrap().doc);
        current.command_run("xcf.filter.set", radius(&filter, target, true)).unwrap();
        for p in [
            radius(&filter, -1, true),
            radius(&filter, 1001, true),
            json!({"filterId":filter,"arguments":[]}),
            json!({"filterId":filter,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":64,"value":5}}}}]}),
        ] {
            assert!(current.command_run("xcf.filter.set", p).is_err());
        }
        let anchor = current.session.active().unwrap().doc.clone();
        current.command_run("layer.setProps", json!({"layer":layer.0,"name":"ImplicitLinear current","opacity":0.375,"coalesce":"F64\u{0}gesture"})).unwrap();
        current.command_run("layer.translate", json!({"layer":"ImplicitLinear current","dx":1,"dy":-1,"coalesce":"F64\u{0}gesture"})).unwrap();
        let moved = current.session.active().unwrap().doc.clone();
        fixture::exact(&anchor, &moved, 1, -1);
        fixture::observed(&moved);
        assert_eq!(current.session.active().unwrap().history.undo_label(), Some("Layer Properties"));
        current.command_run("edit.undo", Value::Null).unwrap();
        assert!(current.session.active().unwrap().doc.shares_state(&anchor));
        current.command_run("edit.redo", Value::Null).unwrap();
        assert!(current.session.active().unwrap().doc.shares_state(&moved));
        for (id, p) in [("layer.translate", json!({"target":"mask","dx":1})), ("xcf.filter.remove", json!({"filterId":filter})), ("select.all", Value::Null)] {
            assert!(current.command_run(id, p).is_err());
        }
        assert!(current.session.active().unwrap().doc.shares_state(&moved));
        let query = current.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":filter}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:box-blur");
        drop(query);
        let output = directory.0.join("continued.pcraft");
        current.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::exact(&moved, &reopened.session.active().unwrap().doc, 0, 0);
        fixture::observed(&reopened.session.active().unwrap().doc);
        reopened.command_run("layer.translate", json!({"dx":-1,"dy":1})).unwrap();
        fixture::exact(&anchor, &reopened.session.active().unwrap().doc, 0, 0);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(!reopened.session.has_jobs());
    }
}
