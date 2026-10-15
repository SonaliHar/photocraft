//! UNRUN registered paid CURRENT composition host transaction/native continuation.
//! Synthetic fixture/output/model captures never claim GIMP producer edits.
use super::*;
#[path = "../../../ui-egui/src/canvas_flat_composition_fixture.rs"]
mod common;
#[path = "../../../ui-egui/src/canvas_flat_current_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::ctl;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-flat-composition-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn captured_file(doc: &photocraft_doc::Document, q: &photocraft_raster::backing::quota::BackingQuota, path: &std::path::Path) {
    let bytes = photocraft_format::owned_document::save_current_document_archive(doc, std::io::Cursor::new(Vec::new()), Default::default(), q, &ctl())
        .unwrap()
        .into_inner();
    std::fs::write(path, bytes).unwrap();
}
#[test]
#[ignore = "UNRUN: paid FlatComposite20mode/explicitspace/operator wire through Headless, auth/wait/capturedrev/contentlocks/noopupdatedtrue/history/native/paid project; source roles unchanged"]
fn headless_current_composition_uses_exact_synchronous_transaction() {
    for (mode, blend, composite, operator, rawblend, rawcomposite, rawop) in
        common::MODES.into_iter().map(|m| (m, "linear-rgb", "linear-rgb", "union", 1, 1, 1)).chain([
            (30, "profile-rgb", "linear-rgb", "clip-to-backdrop", 2, 1, 2),
            (30, "perceptual-rgb", "profile-rgb", "clip-to-layer", 4, 2, 3),
            (30, "linear-rgb", "perceptual-rgb", "intersection", 1, 4, 4),
        ])
    {
        let dir = Directory::new();
        let (owner, _) = fixture::fixture();
        let input = dir.0.join("Synthetic CURRENT.pcraft");
        captured_file(owner.document(), owner.quota(), &input);
        let input_bytes = std::fs::read(&input).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&input, &ctl()).unwrap());
        let target = h.session.active().unwrap().doc.layers[4].id;
        h.command_run("layer.setProps", json!({"layer":target.0,"opacity":0.125})).unwrap();
        // Real registered setup synchronizes the fixture's inert physical
        // Normal surrogate before exact raw+physical no-op expectations.
        let setup = h.session.active().unwrap().doc.clone();
        let setup_rev = h.session.active().unwrap().revision;
        h.command_run("xcf.composition.set", common::wire(&setup, setup_rev, target, 33, "linear-rgb", "linear-rgb", "union")).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        let revision = h.session.active().unwrap().revision;
        let request = common::wire(&before, revision, target, mode, blend, composite, operator);
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        assert!(matches!(h.command_start("xcf.composition.set",request.clone(),false),Err(AutomationError::BadRequest(ref e))if e.contains("synchronous")));
        assert!(!h.session.has_jobs());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("composition denied".into())));
        assert!(h.command_run("xcf.composition.set", request.clone()).unwrap_err().to_string().contains("composition denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal, journal);
        assert_eq!(
            h.command_start("xcf.composition.set", request.clone(), true).unwrap(),
            json!({"layerId":target.0.to_string(),"sourceMode":mode,"updated":true})
        );
        let changed = h.session.active().unwrap().doc.clone();
        common::preserved(&before, &changed);
        common::tuple(&changed, target, mode, rawblend, rawcomposite, rawop);
        let did_change = !changed.shares_state(&before);
        assert_eq!(h.session.active().unwrap().revision, revision + u64::from(did_change));
        assert_eq!(h.session.active().unwrap().history.past_len(), history.past_len() + usize::from(did_change));
        if did_change {
            assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Set GIMP Composition Policy"));
            assert!(h.command_run("xcf.composition.set", request).is_err());
            h.command_run("edit.undo", json!({})).unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            let redo = h.session.active().unwrap().history.clone();
            let now = h.session.active().unwrap().revision;
            let noop = common::wire(&before, now, target, 33, "linear-rgb", "linear-rgb", "union");
            assert_eq!(h.command_run("xcf.composition.set", noop).unwrap()["updated"], true);
            assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
            h.command_run("edit.redo", json!({})).unwrap();
        } else {
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        }
        let live = h.session.active().unwrap().doc.clone();
        let rev = h.session.active().unwrap().revision;
        let good = common::wire(&live, rev, target, mode, blend, composite, operator);
        for (field, value) in [
            ("sourceMode", json!(46)),
            ("sourceMode", json!(47)),
            ("blendSpace", json!("lab")),
            ("legacy", json!(true)),
            ("operator", json!("auto")),
            ("documentId", json!("0")),
            ("expectedRevision", json!((rev + 1).to_string())),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(h.command_run("xcf.composition.set", bad).is_err());
        }
        let mut pair = good.clone();
        pair.as_object_mut().unwrap().remove("documentId");
        assert!(h.command_run("xcf.composition.set", pair).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&live));
        let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &json!({"limit":128}), false, &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 5);
        assert_eq!(reply.value()["filters"]["total"], 0);
        drop(reply);
        let output = dir.0.join("Saved composition.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        let saved = h.session.active().unwrap().doc.clone();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap().doc.clone();
        assert_eq!(current.xcf_state, saved.xcf_state);
        common::preserved(&saved, &current);
        assert!(!reopened.session.active().unwrap().source_read_only);
        let noop = common::wire(&current, reopened.session.active().unwrap().revision, target, mode, blend, composite, operator);
        assert_eq!(reopened.command_run("xcf.composition.set", noop).unwrap()["updated"], true);
        assert!(reopened.session.active().unwrap().doc.shares_state(&current));
        assert_eq!(std::fs::read(&input).unwrap(), input_bytes);
    }
    // Paid synthetic CURRENT locks come from real native reload, not source proof.
    for lock in 0..3 {
        let dir = Directory::new();
        let (owner, _) = fixture::fixture();
        let q = owner.quota();
        let measured = photocraft_io::retained_native_model_census(owner.document(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        let _index = q.reserve(photocraft_raster::backing::quota::QuotaKind::Index, measured.retained_bytes() as u64 * 2 + 8192).unwrap();
        let _resident = q.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, 65536).unwrap();
        let mut locked = owner.document().clone();
        locked.layers[4].blend = photocraft_doc::BlendMode::LinearDodge;
        let id = locked.layers[4].id;
        match lock {
            0 => locked.layers[4].locks.all = true,
            1 => locked.layers[4].locks.pixels = true,
            _ => {
                locked
                    .xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(id))
                    .unwrap()
                    .locks
                    .content = true
            }
        };
        let path = dir.0.join("Synthetic CURRENT content locks.pcraft");
        captured_file(&locked, q, &path);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        let rev = h.session.active().unwrap().revision;
        let exact = common::wire(&before, rev, id, 33, "linear-rgb", "linear-rgb", "union");
        assert!(h.command_run("xcf.composition.set", exact).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal, journal);
        assert_eq!(h.session.active().unwrap().revision, rev);
    }
}
