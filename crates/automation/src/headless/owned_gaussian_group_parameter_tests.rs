//! UNRUN exact owned CURRENT axes/activation; numerical GIMP acceptance remains
//! separate. Source/native/archive files are external harness captures.
use super::*;
#[path = "../../../ui-egui/src/canvas_gaussian_group_parameter_fixture.rs"]
mod fixture;
use fixture::{assets, bits, ctl, matrix, payload, wire};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-gaussian-group-parameters-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: real Gaussian source/edit CURRENT exact axes+optionalactivation atomic commands/auth/sync/inspect/raw signedzero/history/native continuation"]
fn gaussian_group_headless_axis_patch_is_one_paid_current_transaction() {
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
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let initial = bits(&before);
        let source = h.session.active().unwrap().source.clone();
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        let revision = h.session.active().unwrap().revision;
        let request = wire(&id, [Some(1.0f32.to_bits()), None], None);
        assert!(matches!(h.command_start("xcf.filter.set",request.clone(),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous")));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("parameter denied".into())));
        assert!(h.command_start("xcf.filter.set", request, true).unwrap_err().to_string().contains("parameter denied"));
        h.session.authorize = None;
        assert!(!h.session.has_jobs());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal, journal);
        assert_eq!(h.session.active().unwrap().revision, revision);
        for invalid in [(-1.0f32).to_bits(), 15.000001f32.to_bits(), f32::INFINITY.to_bits(), f32::NAN.to_bits()] {
            assert!(h.command_run("xcf.filter.set", wire(&id, [Some(invalid), None], None)).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        for invalid in [
            json!({"filterId":id,"arguments":[]}),
            json!({"filterId":id,"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}},{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}}]}),
            json!({"filterId":id,"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}},{"name":"std-dev-y","value":{"Number":{"Float64Bits":"0x0000000000000000"}}}]}),
            json!({"filterId":id,"arguments":[{"name":"filter","value":{"Enum":{"domain":"gegl:gaussian-blur::filter","value":0}}}]}),
            json!({"filterId":id,"arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}}],"opacityF64Bits":1.0f64.to_bits()}),
        ] {
            assert!(h.command_run("xcf.filter.set", invalid).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.journal, journal);
        }
        for (index, axes) in matrix().into_iter().enumerate() {
            let enabled = index != 3;
            let past = h.session.active().unwrap().history.past_len();
            assert_eq!(h.command_start("xcf.filter.set", wire(&id, axes.map(Some), Some(enabled)), true).unwrap(), json!({"filterId":id,"updated":true}));
            let changed = h.session.active().unwrap().doc.clone();
            payload(&before, &changed);
            assert_eq!(bits(&changed), axes);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled, enabled);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0].revision, before.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
            assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
            assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Set GIMP Group Filter Parameters"));
            let query = h.inspect_owned_filter_bounded(&json!({"filterId":id}), false).unwrap();
            for (name, expected) in ["std-dev-x", "std-dev-y"].into_iter().zip(axes) {
                let row = query.value()["arguments"].as_array().unwrap().iter().find(|r| r["name"] == name).unwrap();
                assert_eq!(row["state"]["value"]["Number"]["Float32Bits"], expected);
            }
            assert_eq!(query.value()["enabled"], enabled);
            drop(query);
            h.command_run("edit.undo", json!({})).unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            let redo = h.session.active().unwrap().history.clone();
            assert_eq!(h.command_run("xcf.filter.set", wire(&id, initial.map(Some), Some(true))).unwrap()["updated"], false);
            assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
            h.command_run("edit.redo", json!({})).unwrap();
            let output = dir.0.join(format!("CURRENT parameters{index}.pcraft"));
            assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
            h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
            assert_eq!(h.session.active().unwrap().source, source);
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
            let current = &reopened.session.active().unwrap().doc;
            payload(&changed, current);
            assert_eq!(bits(current), axes);
            assert!(!reopened.session.active().unwrap().source_read_only);
            assert_eq!(reopened.command_run("xcf.filter.set", wire(&id, axes.map(Some), Some(enabled))).unwrap()["updated"], false);
            h.command_run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), original);
    }
}
