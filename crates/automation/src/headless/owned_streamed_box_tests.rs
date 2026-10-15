//! UNRUN synthetic CURRENT geometry through genuine paid native readers/writers.
//! This is not an authored Box geometry/source or numerical-acceptance oracle.
//! Encoded Cursor/file bytes and literal samples are external harness captures.
use super::*;
#[path = "../../../ui-egui/src/canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::ctl;
use photocraft_doc::SampleType;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-streamed-box-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: synthetic float Box negative/tile-crossing/offcanvas CURRENT geometry, real paid native adoption/query/writer, implicit radius and assigned enable-only history/noopredo, authorization/sync and hidden-disabled invalid native refusal"]
fn streamed_box_native_headless_continues_existing_transactions_without_source_grant() {
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for variant in 0..2 {
            let dir = Directory::new();
            let (owner, literal) = fixture::fixture(sample, assigned, variant);
            let mut seed = Headless::trusted_local();
            seed.session.add_owned_native_point_archive(fixture::archive(&owner), "Synthetic Box CURRENT", None, &ctl()).unwrap();
            let initial = dir.0.join("initial.pcraft");
            seed.save_owned_native_active(Some(&initial), Some("pcraft"), &ctl()).unwrap();
            let initial_bytes = std::fs::read(&initial).unwrap();
            let mut h = Headless::trusted_local();
            let reply = h.open_owned_native_keep_current(&initial, &ctl()).unwrap();
            assert!(reply.value().is_object());
            drop(reply);
            let before = h.session.active().unwrap().doc.clone();
            let identity = h.session.active().unwrap().source.clone();
            let paid = before.accounted().unwrap();
            assert_eq!(paid.is_readonly_assigned_point_storage(), assigned);
            assert_eq!(paid.is_readonly_point_storage(), !assigned);
            assert!(!h.session.active().unwrap().source_read_only);
            assert_eq!(fixture::native(&before), literal.raw);
            let photocraft_doc::LayerContent::BackedRaster(layer) = &before.layers[0].content else { panic!("native") };
            assert!(layer.quota().shares_ledger(paid.quota()));
            assert_eq!(layer.operative_bounds(), literal.bounds);
            let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
            for wait in [false, true] {
                let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, wait, &ctl()).unwrap();
                assert_eq!(query.value()["items"]["total"], 2);
                assert_eq!(query.value()["filters"]["total"], 1);
                drop(query);
            }
            assert!(
                matches!(h.command_start("xcf.filter.enable", json!({"filterId":id,"enabled":false}), false), Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
            );
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("streamed Box denied".into())));
            assert!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap_err().to_string().contains("streamed Box denied"));
            h.session.authorize = None;
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            let past = h.session.active().unwrap().history.past_len();
            let radius = json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"value":5,"width":32}}}}]});
            if assigned {
                assert!(h.command_run("xcf.filter.set", radius).is_err());
                assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], true);
            } else {
                assert_eq!(h.command_run("xcf.filter.set", radius).unwrap()["updated"], true);
            }
            assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
            assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Edit GIMP Stock Filter"));
            let changed = h.session.active().unwrap().doc.clone();
            assert_eq!(fixture::native(&changed), literal.raw);
            assert_eq!(changed.icc_profile, before.icc_profile);
            assert_eq!(changed.source_color_context, before.source_color_context);
            assert_eq!(h.session.active().unwrap().source, identity);
            for (command, params) in [
                ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"radius","value":{"Number":{"Signed":{"value":6,"width":32}}}}]})),
                ("layer.translate", json!({"dx":1})),
                ("xcf.filter.remove", json!({"filterId":id})),
            ] {
                assert!(h.command_run(command, params).is_err());
            }
            assert!(h.session.active().unwrap().doc.shares_state(&changed));
            h.command_run("edit.undo", Value::Null).unwrap();
            let history = h.session.active().unwrap().history.clone();
            let revision = h.session.active().unwrap().revision;
            assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.active().unwrap().revision, revision);
            h.command_run("edit.redo", Value::Null).unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&changed));
            assert!(!h.session.has_jobs());
            let output = dir.0.join("continued.pcraft");
            h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
            assert_eq!(std::fs::read(&initial).unwrap(), initial_bytes);
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
            fixture::exact(&changed, &reopened.session.active().unwrap().doc);
            let filter = reopened.inspect_owned_filter_bounded(&json!({"filterId":id}), false).unwrap();
            assert_eq!(filter.value()["operation"]["name"], "gegl:box-blur");
            drop(filter);
            // A real charged native read must reject this hidden, disabled,
            // wholly offcanvas non-unit alpha, without a different route retry.
            let malformed = fixture::malformed_fixture(sample, assigned);
            let bad = dir.0.join("invalid.pcraft");
            std::fs::write(&bad, fixture::encoded(&malformed)).unwrap();
            let count = reopened.session.documents().len();
            let current = reopened.session.active().unwrap().doc.clone();
            assert!(reopened.open_owned_native_keep_current(&bad, &ctl()).is_err());
            assert_eq!(reopened.session.documents().len(), count);
            assert!(reopened.session.active().unwrap().doc.shares_state(&current));
        }
    }
}

#[path = "owned_observed_box_tests.rs"]
mod observed_epoch_tests;
