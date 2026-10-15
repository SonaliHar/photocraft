//! UNRUN registered mask toggle on genuine originally enabled geometry sources.
//! Disabled states are synthetic CURRENT/native controls, not authored GIMP
//! evidence. Source/output files are external harness captures.
use super::owned_analytic_geometry_command_tests::{cases, ctl, plane_words, profiles};
use super::*;
use photocraft_doc::{
    Document, LayerId,
    item_identity::{ItemRef, MaskId},
};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-mask-current-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn retained(before: &Document, current: &Document, id: MaskId, enabled: bool) {
    assert_eq!(plane_words(before), plane_words(current));
    assert_eq!(before.icc_profile, current.icc_profile);
    assert_eq!(before.source_color_context, current.source_color_context);
    assert_eq!(before.working_profile_interpretation, current.working_profile_interpretation);
    let mut expected = before.xcf_state.as_ref().unwrap().clone();
    expected.items.iter_mut().find(|item| item.item == ItemRef::Mask(id)).unwrap().mask.as_mut().unwrap().enabled = enabled;
    assert_eq!(current.xcf_state.as_ref().unwrap(), &expected);
    for (a, b) in before.layers.iter().zip(&current.layers) {
        assert_eq!(
            (a.id, &a.name, a.visible, a.opacity.to_bits(), a.fill_opacity.to_bits()),
            (b.id, &b.name, b.visible, b.opacity.to_bits(), b.fill_opacity.to_bits())
        );
        if let Some(mask) = &b.mask {
            assert_eq!(mask.enabled, enabled);
        }
    }
}
fn reply(value: &Value, mask: MaskId, layer: LayerId, enabled: bool, changed: bool) {
    assert_eq!(value, &json!({"maskId":mask.0.to_string(),"layerId":layer.0.to_string(),"enabled":enabled,"changed":changed}));
    assert_eq!(value.as_object().unwrap().len(), 4);
}
#[test]
#[ignore = "UNRUN:64 genuine enabled geometry sources, registered wait/auth/toggle/undo/noopredo and disabled native continuation"]
fn assigned_geometry_headless_mask_enable_is_synchronous_and_retains_exact_planes() {
    let selected: Vec<_> = cases().into_iter().filter(|(root, row)| root.ends_with("left-top") && row["id"].as_str().unwrap().ends_with("-source")).collect();
    assert_eq!(selected.len(), 64);
    for (root, row) in selected {
        let original = std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        let dir = Directory::new();
        let path = dir.0.join("source.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        profiles(&before, &row);
        let source = h.session.active().unwrap().source.clone();
        assert!(h.session.active().unwrap().source_read_only);
        let owner = before.layers.iter().find(|layer| layer.mask.is_some()).unwrap().id;
        let item =
            before.xcf_state.as_ref().unwrap().items.iter().find(|item| item.mask.as_ref().is_some_and(|mask| mask.owner == ItemRef::Layer(owner))).unwrap();
        let ItemRef::Mask(mask) = item.item else { panic!("actual canonical mask identity") };
        assert!(item.mask.as_ref().unwrap().enabled);
        assert!(before.layers.iter().find_map(|layer| layer.mask.as_ref()).unwrap().enabled);
        let params = json!({"maskId":mask.0.to_string(),"enabled":false});
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        assert!(
            matches!(h.command_start("xcf.mask.enabled.set",params.clone(),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
        );
        assert!(!h.session.has_jobs());
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("mask denied".into())));
        let error = h.command_run("xcf.mask.enabled.set", params.clone()).unwrap_err();
        assert!(error.to_string().contains("mask denied"));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        h.session.authorize = None;
        for invalid in [json!({"maskId":format!("0{}",mask.0),"enabled":false}), json!({"maskId":mask.0.to_string(),"enabled":false,"coalesce":null})] {
            assert!(h.command_run("xcf.mask.enabled.set", invalid).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        reply(&h.command_start("xcf.mask.enabled.set", params, true).unwrap(), mask, owner, false, true);
        assert!(!h.session.has_jobs());
        assert_eq!(h.session.journal.len(), journal, "upstream journal:false preserved");
        let disabled = h.session.active().unwrap().doc.clone();
        retained(&before, &disabled, mask, false);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Set XCF Layer Mask Enabled"));
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        reply(&h.command_run("xcf.mask.enabled.set", json!({"maskId":mask.0.to_string(),"enabled":true})).unwrap(), mask, owner, true, false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&disabled));
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 3);
        let mask_row = query.value()["items"]["entries"].as_array().unwrap().iter().find(|item| item["hasMask"] == true).unwrap();
        assert_eq!(mask_row["mask"]["enabled"], false);
        drop(query);
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Disabled current.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        assert!(h.session.active().unwrap().source_read_only);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let state = reopened.session.active().unwrap();
        assert!(!state.source_read_only);
        retained(&before, &state.doc, mask, false);
        reply(&reopened.command_start("xcf.mask.enabled.set", json!({"maskId":mask.0.to_string(),"enabled":true}), true).unwrap(), mask, owner, true, true);
        retained(&before, &reopened.session.active().unwrap().doc, mask, true);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap(), original);
    }
}
