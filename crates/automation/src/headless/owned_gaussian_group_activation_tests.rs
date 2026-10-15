//! UNRUN authentic source -> CURRENT activation edit -> paid native continuation.
//! This does not add a source predicate or claim newly authored disabled evidence.
use super::*;
#[path = "../../../ui-egui/src/canvas_gaussian_group_activation_fixture.rs"]
#[allow(dead_code)] // The UI consumer additionally reads the independent projection.
mod fixture;
use fixture::{assets, ctl, payload};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-gaussian-group-activation-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: Gaussian group synchronous exact set/enable/auth/oneundo/noopredo/paid disabled native/re-enable continuation and source custody"]
fn gaussian_group_headless_activation_is_one_synchronous_current_transaction() {
    let root = assets();
    for phase in ["source", "edit"] {
        let file = if phase == "source" { "group-28-group-mask.xcf" } else { "group-28-group-mask.edit.xcf" };
        let original = std::fs::read(root.join(file)).unwrap();
        let dir = Directory::new();
        let path = dir.0.join("Original.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        for command in ["xcf.filter.enable", "xcf.filter.set"] {
            assert!(
                matches!(h.command_start(command,json!({"filterId":id,"enabled":false}),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous"))
            );
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("activation denied".into())));
            assert!(h.command_start(command, json!({"filterId":id,"enabled":false}), true).unwrap_err().to_string().contains("activation denied"));
            h.session.authorize = None;
        }
        assert!(!h.session.has_jobs());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        // A recognized but unsupported field cannot be silently discarded.
        for request in [json!({"filterId":id,"enabled":false,"arguments":[]}), json!({"filterId":id,"enabled":false,"opacityF64Bits":1.0f64.to_bits()})] {
            assert!(h.command_run("xcf.filter.set", request).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        assert_eq!(h.command_start("xcf.filter.enable", json!({"filterId":id,"enabled":false}), true).unwrap(), json!({"filterId":id,"updated":true}));
        let disabled = h.session.active().unwrap().doc.clone();
        payload(&before, &disabled);
        assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(h.session.active().unwrap().history.past_len(), history.past_len() + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Set GIMP Group Filter Enabled"));
        assert_eq!(h.session.journal.len(), journal.len() + 1);
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let redo = h.session.active().unwrap().history.clone();
        assert_eq!(h.command_run("xcf.filter.set", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&disabled));
        let query = h.inspect_owned_filter_bounded(&json!({"filterId":id}), false).unwrap();
        assert_eq!(query.value()["enabled"], false);
        drop(query);
        let output = dir.0.join("Disabled current.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        payload(&disabled, &reopened.session.active().unwrap().doc);
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert_eq!(reopened.command_run("xcf.filter.set", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        payload(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), original);
    }
}
