//! Authenticated registered workflows over paid larger synthetic CURRENT roots.
//! Pinned original8x6 files supply native/metadata only; no larger GIMP oracle.
use super::*;
#[path = "../../../ui-egui/src/nine_point_geometry_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-nine-current-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN:13 nine-epoch authentic baseline roles in paid synthetic larger CURRENT/native registered set-enable/readonlyprops/query/title/save/reopen/history/noop with sync/auth/work/cancel atomic refusal; no producer geometry or display oracle, Move/remove/directcompound/empty stay closed"]
fn nine_epoch_registered_native_services_preserve_authority() {
    for index in 0..fixture::case_count() {
        let directory = Directory::new();
        let owner = fixture::build(index, photocraft_doc::Size::new(257, 259));
        let input = directory.0.join("current.pcraft");
        std::fs::write(&input, fixture::encoded(&owner)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&input).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::same(owner.document(), &before);
        let id = fixture::node(&before).id.0.to_string();
        let enabled = fixture::node(&before).enabled;
        let params = fixture::changed(&before).unwrap_or_else(|| json!({"filterId":id,"enabled":!enabled}));
        let command = if fixture::changed(&before).is_some() { "xcf.filter.set" } else { "xcf.filter.enable" };
        assert!(h.command_start(command, params.clone(), false).unwrap_err().to_string().contains("synchronous"));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("nine edit denied".into())));
        assert!(h.command_start(command, params.clone(), true).unwrap_err().to_string().contains("nine edit denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(h.command_start(command, params.clone(), true).unwrap()["updated"], true);
        let edited = h.session.active().unwrap().doc.clone();
        fixture::retained(&before, &edited);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.command_run(command, params).unwrap()["updated"], false);
        h.command_run("edit.undo", Value::Null).unwrap();
        let history = h.session.active().unwrap().history.clone();
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":enabled})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&edited));
        let query = h.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":id}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], fixture::case(index).operation);
        drop(query);
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 2);
        assert_eq!(query.value()["filters"]["total"], 1);
        drop(query);
        h.command_run("layer.setProps", json!({"name":"Nine native metadata","visible":true,"opacity":1.0})).unwrap();
        let current = h.session.active().unwrap().doc.clone();
        fixture::retained(&before, &current);
        // Direct caller controls remain authoritative; failure leaves actual state,
        // history, redo and source custody intact, not just an equal model clone.
        let calls = AtomicU64::new(0);
        let deny = |_: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(photocraft_raster::backing::BackingError::Work)
        };
        let control = photocraft_raster::backing::ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &control).is_err());
        assert!(calls.load(Ordering::Relaxed) > 0);
        assert!(h.session.active().unwrap().doc.shares_state(&current));
        let cancelled = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        let control = photocraft_raster::backing::ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(h.save_owned_native_active(Some(&directory.0.join("cancelled.pcraft")), Some("pcraft"), &control).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&current));
        for (command, params) in [
            ("layer.translate", json!({"dx":1})),
            ("xcf.filter.move", json!({"filterId":id,"index":0})),
            ("xcf.filter.remove", json!({"filterId":id})),
            ("layer.setProps", json!({"name":"closed","coalesce":"x"})),
            ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"unknown","value":{"Bool":true}}]})),
        ] {
            assert!(h.command_run(command, params).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&current));
        }
        let output = directory.0.join("continued.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut continued = Headless::trusted_local();
        drop(continued.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::same(&current, &continued.session.active().unwrap().doc);
        let titled = current.accounted().unwrap().try_with_document_name("Nine titled native", &fixture::ctl()).unwrap();
        let mut s = photocraft_engine::Session::new();
        s.add_owned_native_point_archive(fixture::archive(&titled), "titled.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(titled.document(), &s.active().unwrap().doc);
        assert_eq!(s.active().unwrap().doc.name, "Nine titled native");
        assert!(!continued.session.has_jobs());
    }
}
