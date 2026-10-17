//! Deferred Headless commands over existing qualified actual source carriers.
use super::*;
#[path = "../../../engine/src/accounted_programme_paint_test_support.rs"]
mod f;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-programme-paint-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN/uncompiled nine actual class Headless registered brush, source exclusion, same-class native SaveAs/title/typed reopen/undo with complete programme/mask/raw preservation. Existing genuine source loaders and labelled paidCURRENT only, no Source/producer oracle expansion."]
fn programme_headless_command_and_native_continue_without_flattening() {
    for &case in f::cases() {
        let input = f::load(case);
        let mut h = Headless::trusted_local();
        f::adopt(&mut h.session, case, &input);
        h.session.tools.brush.pressure_size = false;
        let before = h.session.active().unwrap().doc.clone();
        let id = f::target(&before);
        let raw = f::raw(&before, id);
        let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
        let source = h.session.active().unwrap().source.clone();
        drop(h.command_run("paint.stroke", f::patch(&before, id)).unwrap());
        let changed = h.session.active().unwrap().doc.clone();
        assert_ne!(f::raw(&changed, id), raw);
        witness.preserved(&changed, id, &f::ctl());
        assert_eq!(h.session.active().unwrap().source, source);
        if let Some(identity) = &source
            && let Some(path) = &identity.path
        {
            assert!(identity.check_destination(path).is_err());
        }
        let dir = Directory::new();
        let path = dir.0.join("programme.pcraft");
        h.save_owned_native_active(Some(&path), Some("pcraft"), &f::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "programme.pcraft");
        let mut reopened = Session::new();
        f::reopen(&mut reopened, case, f::archive(saved.accounted().unwrap()));
        let now = reopened.active().unwrap().doc.clone();
        assert_eq!(now.name, saved.name);
        assert_eq!(f::raw(&now, id), f::raw(&saved, id));
        witness.preserved(&now, id, &f::ctl());
        h.command_run("edit.undo", json!({})).unwrap();
        assert_eq!(f::raw(&h.session.active().unwrap().doc, id), raw);
        h.command_run("edit.redo", json!({})).unwrap();
        assert_eq!(f::raw(&h.session.active().unwrap().doc, id), f::raw(&saved, id));
        assert_eq!(h.session.active().unwrap().source, source);
        let stable = h.session.active().unwrap().doc.clone();
        let mut unsupported = f::patch(&stable, id);
        unsupported["autoErase"] = json!(true);
        assert!(h.command_run("paint.pencil", unsupported).is_err());
        assert!(stable.shares_state(&h.session.active().unwrap().doc));
    }
}
