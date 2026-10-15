//! UNRUN exact assigned Gaussian originals through authorized path/native/read-only
//! consumers. File buffers are external harness captures; production readers,
//! writers and reply carriers retain real accounting. Missing producer ICC
//! captures remain HOLD; no display or ICC equivalence waiver.
use super::*;
#[path = "../../../ui-egui/src/canvas_assigned_gaussian_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::ctl;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-assigned-gaussian-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: assigned F32 Gaussian unit/HDR source/edit strict path/native adoption, retained paid queries/auth/wait, mutation refusal and exact native/mask/profile/source persistence; no producer ICC/numerical acceptance claim"]
fn assigned_gaussian_headless_remains_read_only_across_paid_native_continuation() {
    for (original_path, bytes) in fixture::cases() {
        let dir = Directory::new();
        let path = dir.0.join("source.xcf");
        std::fs::write(&path, &bytes).unwrap();
        let mut h = Headless::trusted_local();
        let reply = h.open_owned_xcf_keep_current(&path, &ctl()).unwrap();
        assert!(reply.value().is_object());
        drop(reply);
        let before = h.session.active().unwrap().doc.clone();
        let identity = h.session.active().unwrap().source.clone();
        let q = before.accounted().unwrap().quota().clone();
        assert!(before.accounted().unwrap().is_readonly_assigned_point_storage());
        assert!(!before.accounted().unwrap().is_readonly_point_storage());
        assert!(h.session.active().unwrap().source_read_only);
        fixture::observed(&original_path, &before);
        assert!(h.has_owned_filter_inspection());
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        for wait in [false, true] {
            let project = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, wait, &ctl()).unwrap();
            assert_eq!(project.value()["filters"]["total"], 1);
            drop(project);
            let filter = h.inspect_owned_filter_with_read_control(&json!({"filterId":id}), wait, &ctl()).unwrap();
            assert_eq!(filter.value()["operation"]["name"], "gegl:gaussian-blur");
            assert_eq!(filter.value()["arguments"].as_array().unwrap().len(), 5);
            drop(filter);
        }
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("assigned inspection denied".into())));
        let denied = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, false, &ctl());
        match denied {
            Err(error) => assert!(error.to_string().contains("assigned inspection denied")),
            Ok(_) => panic!("query authorizer must run"),
        }
        h.session.authorize = None;
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        for (command, params) in [
            ("layer.setProps", json!({"name":"closed"})),
            ("layer.translate", json!({"dx":1})),
            ("xcf.filter.enable", json!({"filterId":id,"enabled":false})),
            ("xcf.filter.enable", json!({"filterId":id,"enabled":true})),
            ("xcf.filter.set", json!({"filterId":id,"enabled":true})),
            ("xcf.filter.move", json!({"filterId":id,"index":0})),
            ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"value":5,"width":32}}}}]})),
            ("select.all", Value::Null),
            ("edit.clear", Value::Null),
        ] {
            assert!(h.command_start(command, params.clone(), false).is_err(), "nonwaiting {command}");
            assert!(h.command_run(command, params).is_err(), "{command}");
        }
        assert!(!h.session.has_jobs());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        let stop = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&stop);
        let cancelled = photocraft_raster::backing::ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &cancelled).is_err());
        let no_work = photocraft_raster::backing::ReadControl {
            interrupt: &photocraft_raster::Interrupt::NONE,
            work: &|_| Err(photocraft_raster::backing::BackingError::Work),
        };
        assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &no_work).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let output = dir.0.join("current.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, identity);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = &reopened.session.active().unwrap().doc;
        assert!(current.accounted().unwrap().is_readonly_assigned_point_storage());
        assert!(!reopened.session.active().unwrap().source_read_only);
        fixture::exact(&before, current);
        // A separately opened host has its own catalog quota. Native payloads
        // still use that actual catalog, never the first host's ledger/stats.
        let photocraft_doc::LayerContent::BackedRaster(layer) = &current.layers[0].content else { panic!("native") };
        assert!(layer.quota().shares_ledger(current.accounted().unwrap().quota()));
        assert!(!q.shares_ledger(current.accounted().unwrap().quota()));
        let filter = reopened.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert_eq!(filter.value()["operation"]["name"], "gegl:gaussian-blur");
        drop(filter);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(original_path).unwrap(), bytes);
    }
}
