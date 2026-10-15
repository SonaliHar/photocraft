//! UNRUN assigned Box activation/native lifecycle. Disabled state is a CURRENT
//! edit; original native/source/ICC captures remain independent and unchanged.
use super::*;
#[path = "../../../ui-egui/src/canvas_assigned_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::ctl;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-assigned-box-enable-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN: assigned Box source/edit authorized synchronous set/enable, exact disabled native/profile/source persistence, one history/noopredo, query and native reenable continuation"]
fn assigned_box_headless_activation_preserves_source_and_noop_redo() {
    for (original_path, bytes) in fixture::cases() {
        let dir = Directory::new();
        let path = dir.0.join("source.xcf");
        std::fs::write(&path, &bytes).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let p = json!({"filterId":id,"enabled":false});
        assert!(matches!(h.command_start("xcf.filter.enable",p.clone(),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous")));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("assigned activation denied".into())));
        assert!(h.command_run("xcf.filter.set", p.clone()).unwrap_err().to_string().contains("assigned activation denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(h.command_run("xcf.filter.set", p).unwrap()["updated"], true);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Edit GIMP Stock Filter"));
        let disabled = h.session.active().unwrap().doc.clone();
        let mut expected: photocraft_doc::Document = (*before).clone();
        let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
        stack.nodes[0].enabled = false;
        stack.revision += 1;
        fixture::exact(&expected, &disabled);
        fixture::observed(&disabled);
        h.command_run("edit.undo", Value::Null).unwrap();
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&disabled));
        let output = dir.0.join("disabled.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_assigned_point_storage());
        assert!(!reopened.session.active().unwrap().source_read_only);
        fixture::exact(&disabled, &reopened.session.active().unwrap().doc);
        let query = reopened.inspect_owned_filter_bounded(&json!({"filterId":id}), false).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:box-blur");
        drop(query);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.command_run("xcf.filter.set", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        fixture::observed(&reopened.session.active().unwrap().doc);
        assert_eq!(reopened.session.active().unwrap().doc.icc_profile, before.icc_profile);
        assert_eq!(reopened.session.active().unwrap().doc.source_color_context, before.source_color_context);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(std::fs::read(original_path).unwrap(), bytes);
    }
}
