//! UNRUN widget/captured-target drafts; no numerical or visual acceptance claim.
use super::*;
#[path = "../canvas_assigned_box_fixture.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::json;
use std::io::Cursor;
fn app(name: &str, bytes: &[u8]) -> crate::PhotocraftApp {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let source = photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &fixture::ctl(),
    )
    .unwrap();
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_xcf_open::adopt_bytes(&mut app, name, &source, &fixture::ctl()).unwrap();
    app.background_jobs = true;
    app
}
#[test]
#[ignore = "UNRUN: genuine assigned Box checkbox, synchronous registered activation/history/raw profile/native preservation; visual/runtime checks deferred"]
fn actual_box_checkbox_uses_registered_activation_without_parameter_edit_authority() {
    for (path, bytes) in fixture::cases() {
        let app = app(path.file_name().unwrap().to_str().unwrap(), &bytes);
        let before = app.session.active().unwrap().doc.clone();
        let source = app.session.active().unwrap().source.clone();
        let layer = before.layers[0].id;
        let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
        let node = &stack.nodes[0];
        assert!(candidate(&before, layer, node));
        let caption = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).to_owned();
        let past = app.session.active().unwrap().history.past_len();
        let journal = app.session.journal.len();
        let mut h = Harness::builder().with_size(egui::vec2(900., 1000.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, layer), app);
        h.run();
        h.query_all_by_label(&caption).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
        h.run();
        let active = h.state().session.active().unwrap();
        assert!(!h.state().ui.status_error);
        assert!(!h.state().session.has_jobs());
        assert!(!active.doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(active.doc.xcf_state.as_ref().unwrap().filter_stacks[0].revision, stack.revision + 1);
        assert_eq!(active.history.past_len(), past + 1);
        assert_eq!(h.state().session.journal.len(), journal + 1);
        assert_eq!(active.source, source);
        assert_eq!(fixture::native(&active.doc), fixture::native(&before));
        assert!(std::sync::Arc::ptr_eq(active.doc.icc_profile.as_ref().unwrap(), before.icc_profile.as_ref().unwrap()));
        assert_eq!(active.doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, node.arguments);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert!(h.state().session.active().unwrap().doc.shares_state(&before));
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert!(!h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    }
}
#[test]
#[ignore = "UNRUN: stale captured Box snapshot even with restored revision, wrong layer hint and redo preservation"]
fn captured_activation_refuses_stale_snapshot_and_wrong_owner_without_mutation() {
    let (path, bytes) = fixture::cases().remove(0);
    let mut app = app(path.file_name().unwrap().to_str().unwrap(), &bytes);
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let layer = before.layers[0].id;
    let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert!(!candidate(&before, LayerId(layer.0 + 1), node));
    super::super::run_checked(&mut app, &before, revision, "xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false}));
    assert!(!app.ui.status_error);
    let changed = app.session.active().unwrap().doc.clone();
    assert!(!changed.shares_state(&before));
    // Revision alone is not a captured-model fence.
    app.session.active_mut().unwrap().revision = revision;
    let journal = app.session.journal.len();
    let past = app.session.active().unwrap().history.past_len();
    super::super::run_checked(&mut app, &before, revision, "xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":true}));
    assert_eq!(app.ui.status, "xcf.properties.stale-target");
    assert!(app.ui.status_error);
    assert!(app.session.active().unwrap().doc.shares_state(&changed));
    assert_eq!(app.session.journal.len(), journal);
    assert_eq!(app.session.active().unwrap().history.past_len(), past);
    assert!(!app.session.has_jobs());
}
