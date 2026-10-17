//! Deferred public source/native lifecycle; no producer projection oracle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_source_activity_fixture.rs"]
mod data;
#[path = "../../../ui-egui/src/filtered_flat_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-activity-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn bytes(b: data::Binding) -> Vec<u8> {
    let raw = std::fs::read(native::workspace().join(b.path)).unwrap();
    assert_eq!(raw.len(), b.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&raw)), b.sha256);
    raw
}
fn observed(d: &photocraft_doc::Document, case: &data::Case) {
    use photocraft_doc::item_identity::ItemRef;
    assert_eq!(d.layers.len(), 4);
    assert_eq!(d.source_color_context.as_ref().unwrap().source_precision, case.precision);
    assert!(d.source_color_context.as_ref().unwrap().original_icc.is_none());
    let x = d.xcf_state.as_ref().unwrap();
    assert_eq!(x.filter_stacks.len(), 3);
    assert_eq!(x.filter_masks.len(), 3);
    for (l, r) in d.layers.iter().zip(case.roots) {
        assert_eq!(l.name, r.name);
        assert_eq!(l.visible, r.visible);
        assert_eq!(l.opacity.to_bits(), r.opacity_bits);
        assert_eq!(native::raw(l), bytes(r.native));
        for p in r.layer_properties.iter().chain(r.effect_properties) {
            let identity = format!("{}@{}:4", p.tag, p.offset);
            let source =
                x.opaque.iter().find(|v| v.owner == Some(ItemRef::Layer(l.id)) && v.namespace == "gimp/xcf/source-property" && v.identity == identity).unwrap();
            assert_eq!(source.bytes.as_slice(), p.bytes);
            assert_eq!(source.order, p.order);
            assert_eq!(source.blake3, blake3::hash(&p.bytes).to_hex().to_string());
        }
        if let Some(op) = r.operation {
            let st = x.filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(l.id)).unwrap();
            let n = &st.nodes[0];
            assert_eq!(n.operation.name, op);
            assert_eq!(Some(n.enabled), r.enabled);
            let c = n.composition.as_ref().unwrap();
            assert_eq!(Some([c.stored_blend_space, c.stored_composite_space, c.stored_operator]), r.raw3);
            assert!(c.resolved.is_none());
            let mask = x.filter_masks.iter().find(|m| m.owner == n.id).unwrap();
            assert_eq!(mask.surface.try_read_interleaved(d.bounds()).unwrap(), bytes(r.mask_native.unwrap()));
        }
    }
}
#[test]
#[ignore = "UNRUN:all12 genuine activity source paths/public owned read→authored nativeSaveAs/reopen before CURRENT enable/metadata restoration+undo/redo, rawsource/native/profile/mask/source fingerprints and auth/wait/source collision/malformed inactive input refusal. Not a numerical/profile equivalence oracle."]
fn mixed_headless_real_activity_sources_continue_without_losing_original_roles() {
    for case in &data::CASES {
        let directory = Directory::new();
        let path = native::workspace().join(case.source.path);
        let source_bytes = bytes(case.source);
        drop(bytes(case.capture));
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&path).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        assert!(h.session.active().unwrap().source_read_only);
        observed(&original, case);
        let native_path = directory.0.join("authored.pcraft");
        h.save_owned_native_active(Some(&native_path), Some("pcraft"), &native::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "authored.pcraft");
        native::storage(&original, &saved);
        let mut next = Headless::trusted_local();
        drop(next.open_owned_native_keep_current_bounded(&native_path).unwrap());
        native::same(&saved, &next.session.active().unwrap().doc);
        observed(&next.session.active().unwrap().doc, case);
        let opaque = saved.xcf_state.as_ref().unwrap().opaque.clone();
        let masks = saved.xcf_state.as_ref().unwrap().filter_masks.clone();
        let profile = saved.icc_profile.clone();
        let disabled = saved.xcf_state.as_ref().unwrap().filter_stacks.iter().filter(|s| !s.nodes[0].enabled).map(|s| s.nodes[0].id).collect::<Vec<_>>();
        for id in disabled {
            let p = json!({"filterId":id.0.to_string(),"enabled":true});
            assert!(next.command_start("xcf.filter.enable", p.clone(), false).is_err());
            assert!(next.session.active().unwrap().doc.shares_state(&saved));
            next.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("activity edit denied".into())));
            assert!(next.command_run("xcf.filter.enable", p.clone()).is_err());
            next.session.authorize = None;
            assert_eq!(next.command_start("xcf.filter.enable", p, true).unwrap()["updated"], true);
            assert_eq!(next.session.active().unwrap().doc.xcf_state.as_ref().unwrap().opaque, opaque);
            next.command_run("edit.undo", json!({})).unwrap();
            native::same(&saved, &next.session.active().unwrap().doc);
            assert_eq!(next.command_run("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).unwrap()["updated"], false);
            assert!(next.session.active().unwrap().history.can_redo());
            next.command_run("edit.redo", json!({})).unwrap();
            next.command_run("edit.undo", json!({})).unwrap();
            native::same(&saved, &next.session.active().unwrap().doc);
        }
        for layer in &saved.layers {
            if !layer.visible || layer.opacity.to_bits() != 1f32.to_bits() {
                let p = json!({"layer":layer.id.0,"visible":true,"opacity":1.0});
                assert!(next.command_start("layer.setProps", p.clone(), false).is_err());
                next.command_start("layer.setProps", p, true).unwrap();
                assert_eq!(next.session.active().unwrap().doc.xcf_state.as_ref().unwrap().opaque, opaque);
                assert_eq!(next.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_masks, masks);
                assert_eq!(next.session.active().unwrap().doc.icc_profile, profile);
                next.command_run("edit.undo", json!({})).unwrap();
                native::same(&saved, &next.session.active().unwrap().doc);
            }
        }
        let query = next.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":2}), false, &native::ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 3);
        drop(query);
        for st in &saved.xcf_state.as_ref().unwrap().filter_stacks {
            let reply = next
                .inspect_owned_command_with_read_control("xcf.filter.inspect", &json!({"filterId":st.nodes[0].id.0.to_string()}), false, &native::ctl())
                .unwrap();
            assert_eq!(reply.value()["scalarEditCandidate"], true);
            drop(reply);
        }
        let collision = directory.0.join("source-copy.pcraft");
        std::fs::write(&collision, &source_bytes).unwrap();
        assert!(next.save_owned_native_active(Some(&collision), Some("pcraft"), &native::ctl()).is_err());
        assert_eq!(std::fs::read(&collision).unwrap(), source_bytes);
        assert!(next.session.active().unwrap().doc.shares_state(&saved));
        let continued = directory.0.join("continued.pcraft");
        next.save_owned_native_active(Some(&continued), Some("pcraft"), &native::ctl()).unwrap();
        let final_saved = next.session.active().unwrap().doc.clone();
        assert_eq!(final_saved.name, "continued.pcraft");
        native::storage(&saved, &final_saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&continued).unwrap());
        native::same(&final_saved, &reopened.session.active().unwrap().doc);
        observed(&reopened.session.active().unwrap().doc, case);
        // Explicitly malformed derivative of actual inactive source; NOT a GIMP witness.
        if case.id.ends_with("all-inactive") {
            let prop = case.roots[3].effect_properties.iter().find(|p| p.tag == 33).unwrap();
            let mut malformed = source_bytes.clone();
            let at = prop.offset as usize;
            assert_eq!(&malformed[at..at + 4], &prop.bytes);
            malformed[at..at + 4].copy_from_slice(&0.5f32.to_bits().to_be_bytes());
            let bad = directory.0.join("malformed.xcf");
            std::fs::write(&bad, malformed).unwrap();
            let before = reopened.session.active().unwrap().doc.clone();
            let count = reopened.session.documents().len();
            assert!(reopened.open_owned_xcf_keep_current_bounded(&bad).is_err());
            assert_eq!(reopened.session.documents().len(), count);
            assert!(reopened.session.active().unwrap().doc.shares_state(&before));
        }
        assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
        assert!(!reopened.session.has_jobs());
    }
}
