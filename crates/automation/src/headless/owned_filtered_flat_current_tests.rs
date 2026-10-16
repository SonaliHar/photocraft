//! Deferred TrustedLocal and Workspace actualowner writer/native10 empty restoration.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_current_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-current-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn headless(dir: &Directory, trusted: bool) -> Headless {
    if trusted { Headless::trusted_local() } else { Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), Some(&dir.0)).unwrap()) }
}
fn path(dir: &Directory, trusted: bool, name: &str) -> PathBuf {
    if trusted { dir.0.join(name) } else { PathBuf::from(name) }
}
#[test]
#[ignore = "UNRUN actualsource bothdepth/TrustedLocal+Workspace writer→native10hint→automaticownedemptyclass restoration, all9removal/archive/raw/source/profile+zeroquery/title/history, single-root3→2→1→0 and strictunmarked/wronghandoff refusals; no producerempty projection oracle."]
fn empty_mixed_native_continues_both_authorized_writers() {
    for case in &fixture::data::CASES {
        for trusted in [true, false] {
            let dir = Directory::new();
            let bytes = fixture::bytes(case.source);
            std::fs::write(dir.0.join("source.xcf"), &bytes).unwrap();
            let mut h = headless(&dir, trusted);
            drop(h.open_owned_xcf_keep_current_bounded(&path(&dir, trusted, "source.xcf")).unwrap());
            let original = h.session.active().unwrap().doc.clone();
            let ids = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
            for id in &ids {
                let before = h.session.active().unwrap().doc.clone();
                let p = serde_json::json!({"filterId":id.0.to_string()});
                assert!(h.command_start("xcf.filter.remove", p.clone(), false).is_err());
                assert!(h.session.active().unwrap().doc.shares_state(&before));
                h.command_start("xcf.filter.remove", p, true).unwrap();
                fixture::preserved(&before, &h.session.active().unwrap().doc, *id);
            }
            let empty = h.session.active().unwrap().doc.clone();
            assert!(empty.accounted().unwrap().is_readonly_filtered_flat_point_storage());
            assert!(empty.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|s| s.nodes.is_empty()));
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
            assert!(h.command_run("xcf.filter.remove", serde_json::json!({"filterId":ids[0].0.to_string()})).is_err());
            h.session.authorize = None;
            let before = h.session.active().unwrap().doc.clone();
            h.save_owned_native_active(Some(&path(&dir, trusted, "empty.pcraft")), Some("pcraft"), &fixture::ctl()).unwrap();
            let saved = h.session.active().unwrap().doc.clone();
            assert_eq!(saved.name, "empty.pcraft");
            fixture::storage(&before, &saved);
            let raw = std::fs::read(dir.0.join("empty.pcraft")).unwrap();
            let archive = fixture::open(saved.accounted().unwrap(), &raw);
            assert_eq!(archive.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::FilteredFlatPoint));
            assert!(photocraft_engine::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
            assert!(photocraft_format::load_from_bytes(&raw).is_err());
            drop(archive);
            let mut reopened = headless(&dir, trusted);
            drop(reopened.open_owned_native_keep_current_bounded(&path(&dir, trusted, "empty.pcraft")).unwrap());
            fixture::same(&saved, &reopened.session.active().unwrap().doc);
            let q = reopened.inspect_owned_command_with_read_control("xcf.inspect", &serde_json::json!({"limit":128}), false, &fixture::ctl()).unwrap();
            assert_eq!(q.value()["filters"]["total"], 0);
            assert_eq!(q.value()["filterStacks"]["total"], 3);
            drop(q);
            assert!(
                reopened
                    .inspect_owned_command_with_read_control(
                        "xcf.filter.inspect",
                        &serde_json::json!({"filterId":ids[0].0.to_string()}),
                        false,
                        &fixture::ctl()
                    )
                    .is_err()
            );
            let root = saved.layers[0].id;
            reopened.command_run("layer.setProps", serde_json::json!({"layer":root.0,"name":"Empty continued","opacity":0.5})).unwrap();
            reopened.command_run("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&saved, &reopened.session.active().unwrap().doc);
            let single = fixture::single_root_three(original.accounted().unwrap());
            std::fs::write(dir.0.join("single.pcraft"), fixture::encoded(&single)).unwrap();
            let mut one = headless(&dir, trusted);
            drop(one.open_owned_native_keep_current_bounded(&path(&dir, trusted, "single.pcraft")).unwrap());
            let ids = single.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.iter().map(|n| n.id).collect::<Vec<_>>();
            for id in &ids {
                let before = one.session.active().unwrap().doc.clone();
                one.command_run("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
                fixture::preserved(&before, &one.session.active().unwrap().doc, *id);
            }
            let emptyone = one.session.active().unwrap().doc.clone();
            assert!(emptyone.accounted().unwrap().is_readonly_filtered_flat_point_storage());
            assert_eq!(emptyone.xcf_state.as_ref().unwrap().filter_stacks.len(), 1);
            one.save_owned_native_active(Some(&path(&dir, trusted, "one-empty.pcraft")), Some("pcraft"), &fixture::ctl()).unwrap();
            let finalone = one.session.active().unwrap().doc.clone();
            let mut continued = headless(&dir, trusted);
            drop(continued.open_owned_native_keep_current_bounded(&path(&dir, trusted, "one-empty.pcraft")).unwrap());
            fixture::same(&finalone, &continued.session.active().unwrap().doc);
            let unmarked = fixture::unmarked_archive(finalone.accounted().unwrap());
            assert_eq!(unmarked.current_storage_hint(), None);
            let old = continued.session.active().unwrap().doc.clone();
            assert!(continued.session.add_owned_native_filtered_flat_archive(unmarked, "unmarked.pcraft", None, &fixture::ctl()).is_err());
            assert!(continued.session.active().unwrap().doc.shares_state(&old));
            assert_eq!(std::fs::read(dir.0.join("source.xcf")).unwrap(), bytes);
            assert!(!continued.session.has_jobs());
        }
    }
}
