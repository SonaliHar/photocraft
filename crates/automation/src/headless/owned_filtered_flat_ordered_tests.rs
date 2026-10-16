//! Deferred public ordered native/source workflows, not arithmetic qualification.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_ordered_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-ordered-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN genuine2 ordered source paths/raw/public9node queries, nonfirst indexed activation/history/native storage, paid synthetic1root2 automaticnative+title/save/query and strictmalformed nofallback; auth/wait/closedoperations unchanged, no perstage numeric/format oracle."]
fn ordered_headless_sources_and_single_root_native_keep_actual_ids() {
    for case in &fixture::data::CASES {
        let directory = Directory::new();
        let source_path = fixture::workspace().join(case.source.path);
        let source_bytes = fixture::bytes(case.source);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&source_path).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let ids = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        assert_eq!(ids.len(), 9);
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":2}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 9);
        assert_eq!(query.value()["filters"]["entries"][0]["id"], ids[1].0.to_string());
        assert_eq!(query.value()["filters"]["entries"][1]["id"], ids[2].0.to_string());
        drop(query);
        let authored = directory.0.join("authored.pcraft");
        h.save_owned_native_active(Some(&authored), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "authored.pcraft");
        fixture::storage(&original, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&authored).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        let targets = saved
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|s| s.nodes.iter().enumerate().skip(1).map(move |(index, n)| (s.owner, index, n.id, n.enabled)))
            .collect::<Vec<_>>();
        for (owner, index, id, enabled) in targets {
            let p = json!({"filterId":id.0.to_string(),"enabled":!enabled});
            let before = reopened.session.active().unwrap().doc.clone();
            let revision = reopened.session.active().unwrap().revision;
            assert!(reopened.command_start("xcf.filter.enable", p.clone(), false).is_err());
            assert!(reopened.session.active().unwrap().doc.shares_state(&before));
            reopened.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("ordered denied".into())));
            assert!(reopened.command_run("xcf.filter.enable", p.clone()).is_err());
            reopened.session.authorize = None;
            assert_eq!(reopened.command_start("xcf.filter.enable", p, true).unwrap()["updated"], true);
            let changed = reopened.session.active().unwrap().doc.clone();
            assert_eq!(reopened.session.active().unwrap().revision, revision + 1);
            fixture::payload(&before, &changed);
            let old = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == owner).unwrap();
            let next = changed.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == owner).unwrap();
            assert_eq!(next.revision, old.revision + 1);
            for (at, (a, b)) in old.nodes.iter().zip(&next.nodes).enumerate() {
                assert_eq!(a.id, b.id);
                if at == index {
                    assert_eq!(b.id, id);
                    assert_eq!(b.enabled, !enabled);
                } else {
                    assert_eq!(a, b);
                }
            }
            let reply =
                reopened.inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":id.0.to_string()}), false, &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["filterId"], id.0.to_string());
            assert_eq!(reply.value()["enabled"], !enabled);
            assert_eq!(reply.value()["scalarEditCandidate"], true);
            drop(reply);
            reopened.command_run("edit.undo", json!({})).unwrap();
            fixture::same(&before, &reopened.session.active().unwrap().doc);
            assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":enabled})).unwrap()["updated"], false);
            assert!(reopened.session.active().unwrap().history.can_redo());
            reopened.command_run("edit.redo", json!({})).unwrap();
            fixture::same(&changed, &reopened.session.active().unwrap().doc);
            reopened.command_run("edit.undo", json!({})).unwrap();
        }
        let single = fixture::single_root_two(saved.accounted().unwrap());
        let path = directory.0.join("single.pcraft");
        std::fs::write(&path, fixture::encoded(&single)).unwrap();
        let mut one = Headless::trusted_local();
        drop(one.open_owned_native_keep_current_bounded(&path).unwrap());
        fixture::same(single.document(), &one.session.active().unwrap().doc);
        let node = single.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].clone();
        let p = json!({"filterId":node.id.0.to_string(),"enabled":false});
        assert_eq!(one.command_run("xcf.filter.set", p).unwrap()["updated"], true);
        let before = one.session.active().unwrap().doc.clone();
        for (id, p) in [
            ("layer.translate", json!({"dx":0})),
            ("xcf.filter.move", json!({"filterId":node.id.0.to_string(),"index":0})),
            ("xcf.filter.remove", json!({"filterId":node.id.0.to_string()})),
            ("layer.select", json!({"index":0})),
        ] {
            assert!(one.command_run(id, p).is_err());
            assert!(one.session.active().unwrap().doc.shares_state(&before));
        }
        let root = before.layers[0].id;
        one.command_run("layer.setProps", json!({"layer":root.0,"name":"One ordered root","opacity":0.5})).unwrap();
        fixture::payload(&before, &one.session.active().unwrap().doc);
        let continued = directory.0.join("continued.pcraft");
        one.save_owned_native_active(Some(&continued), Some("pcraft"), &fixture::ctl()).unwrap();
        let after = one.session.active().unwrap().doc.clone();
        assert_eq!(after.name, "continued.pcraft");
        let mut final_h = Headless::trusted_local();
        drop(final_h.open_owned_native_keep_current_bounded(&continued).unwrap());
        fixture::same(&after, &final_h.session.active().unwrap().doc);
        let query = final_h.inspect_owned_command_with_read_control("xcf.inspect", &json!({}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 2);
        assert_eq!(query.value()["items"]["total"], 3);
        drop(query);
        let bad = fixture::malformed(&single);
        let bad_path = directory.0.join("bad.pcraft");
        std::fs::write(&bad_path, fixture::encoded(&bad)).unwrap();
        let baseline = final_h.session.active().unwrap().doc.clone();
        let count = final_h.session.documents().len();
        assert!(final_h.open_owned_native_keep_current_bounded(&bad_path).is_err());
        assert_eq!(final_h.session.documents().len(), count);
        assert!(final_h.session.active().unwrap().doc.shares_state(&baseline));
        assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
        assert!(!final_h.session.has_jobs());
    }
}
