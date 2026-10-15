//! UNRUN authored source/edit with synthetic CURRENT all-target metadata/mask
//! transactions. Harness files/native captures do not grant source authority.
use super::*;
#[path = "../../../ui-egui/src/canvas_gaussian_group_metadata_fixture.rs"]
mod fixture;
use fixture::{assets, ctl, ids, payload, physical};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-gaussian-group-metadata-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: real Gaussian group all4 current metadata/coalesce/nameDFS/mask distinction/paid query/auth/sync/native continuation with exact source/native/profile custody"]
fn gaussian_group_headless_metadata_and_physical_mask_keep_source_roles() {
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
        let targets = ids(&before);
        let group = targets[0];
        let source = h.session.active().unwrap().source.clone();
        let physical = physical(&before);
        let filter = before.xcf_state.as_ref().unwrap().filter_masks[0].id;
        let past = h.session.active().unwrap().history.past_len();
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.clone();
        let revision = h.session.active().unwrap().revision;
        for (command, params) in
            [("layer.setProps", json!({"layer":group.0,"visible":false})), ("xcf.mask.enabled.set", json!({"maskId":physical.0.to_string(),"enabled":false}))]
        {
            assert!(matches!(h.command_start(command,params.clone(),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous")));
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("metadata denied".into())));
            assert!(h.command_start(command, params, true).unwrap_err().to_string().contains("metadata denied"));
            h.session.authorize = None;
        }
        assert!(!h.session.has_jobs());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal, journal);
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_start("layer.setProps", json!({"layer":group.0,"visible":false,"coalesce":"all-four"}), true).unwrap();
        for (index, id) in targets.iter().enumerate() {
            h.command_run(
                "layer.setProps",
                json!({"layer":id.0,"name":format!("Current {index}"),"visible":index!=2,"opacity":if index==3{0.375}else{0.5},"coalesce":"all-four"}),
            )
            .unwrap();
        }
        let changed = h.session.active().unwrap().doc.clone();
        payload(&before, &changed);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        for (index, id) in targets.iter().enumerate() {
            let layer = changed.layer(*id).unwrap();
            assert_eq!(layer.name, format!("Current {index}"));
            assert_eq!(layer.visible, index != 2);
            assert_eq!(layer.opacity.to_bits(), if index == 3 { 0.375f32.to_bits() } else { 0.5f32.to_bits() });
        }
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let redo = h.session.active().unwrap().history.clone();
        let old = before.layer(targets[3]).unwrap();
        h.command_run(
            "layer.setProps",
            json!({"layer":targets[3].0,"name":old.name,"visible":old.visible,"opacity":f64::from(old.opacity),"coalesce":"ignored-for-noop"}),
        )
        .unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.command_run("xcf.mask.enabled.set", json!({"maskId":filter.0.to_string(),"enabled":false})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&changed));
        let journal = h.session.journal.clone();
        assert_eq!(
            h.command_start("xcf.mask.enabled.set", json!({"maskId":physical.0.to_string(),"enabled":false}), true).unwrap(),
            json!({"maskId":physical.0.to_string(),"layerId":group.0.to_string(),"enabled":false,"changed":true})
        );
        assert_eq!(h.session.journal, journal);
        let disabled = h.session.active().unwrap().doc.clone();
        payload(&before, &disabled);
        assert!(!disabled.layer(group).unwrap().mask.as_ref().unwrap().enabled);
        assert!(
            disabled
                .xcf_state
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Mask(filter))
                .unwrap()
                .mask
                .as_ref()
                .unwrap()
                .enabled
        );
        h.command_run("edit.undo", json!({})).unwrap();
        let redo = h.session.active().unwrap().history.clone();
        assert_eq!(h.command_run("xcf.mask.enabled.set", json!({"maskId":physical.0.to_string(),"enabled":true})).unwrap()["changed"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
        h.command_run("edit.redo", json!({})).unwrap();
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, false, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 6);
        assert_eq!(query.value()["filters"]["total"], 1);
        drop(query);
        let output = dir.0.join("CURRENT metadata.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        payload(&disabled, &reopened.session.active().unwrap().doc);
        assert!(!reopened.session.active().unwrap().source_read_only);
        for id in targets {
            reopened.command_run("layer.setProps", json!({"layer":id.0,"name":"Duplicate"})).unwrap();
        }
        reopened.command_run("layer.setProps", json!({"layer":"Duplicate","opacity":0.25})).unwrap();
        assert_eq!(reopened.session.active().unwrap().doc.layer(targets[2]).unwrap().opacity.to_bits(), 0.25f32.to_bits());
        assert_eq!(reopened.session.active().unwrap().doc.layer(targets[1]).unwrap().opacity.to_bits(), 0.5f32.to_bits());
        payload(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), original);
    }
}
