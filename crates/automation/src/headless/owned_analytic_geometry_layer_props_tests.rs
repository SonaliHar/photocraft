//! UNRUN genuine assigned geometry -> registered CURRENT metadata gestures ->
//! paid native continuation. Harness files are external captures, not receipts.
use super::owned_analytic_geometry_command_tests::{cases, ctl, native, operators, profiles};
use super::*;
use photocraft_doc::Document;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-geometry-properties-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn retained(before: &Document, current: &Document) {
    assert_eq!(native(current), native(before));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let a = before.xcf_state.as_ref().unwrap();
    let b = current.xcf_state.as_ref().unwrap();
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.filter_stacks, b.filter_stacks);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.current_filter_output_precision, b.current_filter_output_precision);
}

#[test]
#[ignore = "UNRUN:64 genuine assigned geometry domains; synchronous authorized registered compound edits, cross-layer coalescing and native continuation"]
fn assigned_geometry_headless_properties_share_one_gesture_and_preserve_source() {
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
        operators(&before, &row);
        let source = h.session.active().unwrap().source.clone();
        let foreground = before.layers.iter().find(|layer| layer.mask.is_some()).unwrap().id;
        let backdrop = before.layers.iter().find(|layer| layer.mask.is_none()).unwrap().id;
        let old_backdrop = before.layers.iter().find(|layer| layer.id == backdrop).unwrap();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        assert!(
            matches!(h.command_start("layer.setProps",json!({"layer":foreground.0,"visible":false}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
        );
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("geometry denied".into())));
        let error = h.command_run("layer.setProps", json!({"layer":foreground.0,"visible":false})).unwrap_err();
        assert!(error.to_string().contains("geometry denied"));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.journal.len(), journal);
        h.session.authorize = None;
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(h.command_run("layer.setProps", json!({"layer":foreground.0,"visible":false,"coalesce":"geometry-gesture"})).unwrap(), Value::Null);
        assert_eq!(
            h.command_run("layer.setProps", json!({"layer":backdrop.0,"name":"Backdrop host current","opacity":0.375,"coalesce":"geometry-gesture"})).unwrap(),
            Value::Null
        );
        let changed = h.session.active().unwrap().doc.clone();
        retained(&before, &changed);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        assert!(!changed.layers.iter().find(|layer| layer.id == foreground).unwrap().visible);
        let background = changed.layers.iter().find(|layer| layer.id == backdrop).unwrap();
        assert_eq!(background.name, "Backdrop host current");
        assert_eq!(background.opacity.to_bits(), 0.375f32.to_bits());
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        h.command_run("layer.setProps",json!({"layer":backdrop.0,"name":old_backdrop.name,"visible":old_backdrop.visible,"opacity":f64::from(old_backdrop.opacity),"coalesce":"geometry-gesture"})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&changed));
        h.command_run("layer.setProps", json!({"layer":foreground.0,"name":"Foreground host current"})).unwrap();
        let edited = h.session.active().unwrap().doc.clone();
        retained(&before, &edited);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 2);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Layer Properties"));
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 3);
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Properties current.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        assert!(h.session.active().unwrap().source_read_only);
        let saved = h.session.active().unwrap().doc.clone();
        retained(&before, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(!current.source_read_only);
        assert_eq!(current.doc.name, saved.name);
        assert_eq!(current.doc.xcf_state, edited.xcf_state);
        retained(&edited, &current.doc);
        assert_eq!(
            current.doc.layers.iter().map(|layer| (&layer.name, layer.visible, layer.opacity.to_bits())).collect::<Vec<_>>(),
            edited.layers.iter().map(|layer| (&layer.name, layer.visible, layer.opacity.to_bits())).collect::<Vec<_>>()
        );
        reopened.command_run("layer.setProps", json!({"layer":foreground.0,"visible":true})).unwrap();
        retained(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap(), original);
    }
}
