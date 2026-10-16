//! Real authored source endpoints and paid synthetic CURRENT native continuation.
//! Producer projection/profile/Float variability remains separate HOLD evidence.
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
        let p = std::env::temp_dir().join(format!("photocraft-filtered-flat-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN:twoauthoredsources/publicownedpath pluspaidlargersyntheticCURRENT/native/title/querypagination/retainedall3filterIDs/auth/wait/externalcontrol/sourcecollisionprotection andreadonlyrefusal; no producerlargergeometry/numeric/profileoracle"]
fn filtered_flat_headless_sources_and_native_services_are_readonly() {
    for index in 0..2 {
        let directory = Directory::new();
        let path = fixture::path(index);
        let source_bytes = std::fs::read(&path).unwrap();
        let mut original = Headless::trusted_local();
        drop(original.open_owned_xcf_keep_current_bounded(&path).unwrap());
        let snapshot = original.session.active().unwrap().doc.clone();
        fixture::observed(&snapshot, index);
        assert!(original.session.active().unwrap().source_read_only);
        let authored_native = directory.0.join("authored.pcraft");
        original.save_owned_native_active(Some(&authored_native), Some("pcraft"), &fixture::ctl()).unwrap();
        let authored_saved = original.session.active().unwrap().doc.clone();
        assert_eq!(authored_saved.name, "authored.pcraft");
        fixture::storage(&snapshot, &authored_saved);
        let mut authored = Headless::trusted_local();
        drop(authored.open_owned_native_keep_current_bounded(&authored_native).unwrap());
        fixture::same(&authored_saved, &authored.session.active().unwrap().doc);
        let source = Arc::new(fixture::import(index));
        let current = fixture::activity(&fixture::larger(source));
        let input = directory.0.join("current.pcraft");
        std::fs::write(&input, fixture::encoded(&current)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&input).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::same(current.document(), &before);
        let history = h.session.active().unwrap().history.clone();
        let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        assert!(h.command_start("xcf.filter.enable", json!({"filterId":filter,"enabled":false}), false).is_err());
        assert!(h.command_start("xcf.filter.enable", json!({"filterId":filter,"enabled":false,"unsupported":true}), true).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("filtered query denied".into())));
        assert!(
            h.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &fixture::ctl())
                .err()
                .unwrap()
                .to_string()
                .contains("filtered query denied")
        );
        h.session.authorize = None;
        for params in [Value::Null, json!({"offset":1,"limit":2}), json!({"offset":100000,"limit":1})] {
            let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &params, false, &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["items"]["total"], 7);
            assert_eq!(reply.value()["filters"]["total"], 3);
            assert_eq!(reply.value()["selection"]["selected"]["total"], 1);
            drop(reply);
        }
        for stack in &before.xcf_state.as_ref().unwrap().filter_stacks {
            let id = stack.nodes[0].id.0.to_string();
            let reply = h.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":id}), false, &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["filterId"], id);
            assert_eq!(reply.value()["scalarEditCandidate"], true);
            drop(reply);
        }
        let calls = AtomicU64::new(0);
        let deny = |_: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(photocraft_raster::backing::BackingError::Work)
        };
        let c = photocraft_raster::backing::ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &c).is_err());
        assert!(calls.load(Ordering::Relaxed) > 0);
        for (id, p) in [
            ("layer.setProps", json!({"blend":"denied"})),
            ("layer.translate", json!({"dx":0})),
            ("layer.select", json!({"index":0})),
            ("xcf.filter.move", json!({"filterId":filter,"index":0})),
            ("xcf.filter.remove", json!({"filterId":filter})),
        ] {
            assert!(h.command_run(id, p).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        }
        let collision = directory.0.join("source-collision.pcraft");
        std::fs::write(&collision, &source_bytes).unwrap();
        assert!(h.save_owned_native_active(Some(&collision), Some("pcraft"), &fixture::ctl()).is_err());
        assert_eq!(std::fs::read(&collision).unwrap(), source_bytes);
        assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
        let cancelled = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        let c = photocraft_raster::backing::ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(h.save_owned_native_active(Some(&directory.0.join("cancelled.pcraft")), Some("pcraft"), &c).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let titled = before.accounted().unwrap().try_with_document_name("Filtered flat continued", &fixture::ctl()).unwrap();
        let mut next = Headless::trusted_local();
        next.session.add_owned_native_filtered_flat_archive(fixture::archive(&titled), "continued.pcraft", None, &fixture::ctl()).unwrap();
        let output = directory.0.join("continued.pcraft");
        next.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        let continued_saved = next.session.active().unwrap().doc.clone();
        assert_eq!(continued_saved.name, "continued.pcraft");
        fixture::storage(titled.document(), &continued_saved);
        let mut final_h = Headless::trusted_local();
        drop(final_h.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::same(&continued_saved, &final_h.session.active().unwrap().doc);
        assert!(!final_h.session.has_jobs());
    }
}
