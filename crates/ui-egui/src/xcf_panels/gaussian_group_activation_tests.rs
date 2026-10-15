//! UNRUN actual guarded group activation control; no parameter or source-policy edits.
use super::*;
#[path = "../canvas_gaussian_group_activation_fixture.rs"]
mod fixture;
use fixture::{assets, ctl, disabled_projection, payload};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::io::Cursor;
fn compare(app: &PhotocraftApp, root: &std::path::Path, phase: &str) {
    let expected = disabled_projection(root, phase);
    let (image, fee) = crate::canvas::assigned_geometry_test_support::display(&app.session.active().unwrap().doc).unwrap();
    assert_eq!(image.size, [16, 12]);
    for (pixel, expected) in image.pixels.iter().zip(expected) {
        let p: [u8; 4] = expected.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        let reference = egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]).to_array();
        for (a, b) in pixel.to_array().iter().zip(reference) {
            assert!((i16::from(*a) - i16::from(b)).abs() <= 2);
        }
    }
    drop((image, fee));
}
#[test]
#[ignore = "UNRUN: real Gaussian group guarded activation/disabled independent projection/stale target/noopredo/native continuation, no new source or parameter grant"]
fn gaussian_group_guarded_activation_keeps_raw_tree_and_profiles() {
    let root = assets();
    for phase in ["source", "edit"] {
        let file = if phase == "source" { "group-28-group-mask.xcf" } else { "group-28-group-mask.edit.xcf" };
        let bytes = std::fs::read(root.join(file)).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let carrier = photocraft_io::import_owned_xcf_from_reader_in_quota(
            file,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(matches!(&carrier, photocraft_io::OwnedXcfImport::IsolatedGaussianGroup(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, file, &carrier, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let source = app.session.active().unwrap().source.clone();
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        assert!(photocraft_engine::xcf_filter_cmds::activation_edit_candidate(&before, node));
        let journal = app.session.journal.len();
        let past = app.session.active().unwrap().history.past_len();
        run_checked(&mut app, &before, revision, "xcf.filter.enable", json!({"filterId":id,"enabled":false}));
        assert!(!app.ui.status_error);
        let disabled = app.session.active().unwrap().doc.clone();
        assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        payload(&before, &disabled);
        compare(&app, &root, phase);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Set GIMP Group Filter Enabled"));
        assert_eq!(app.session.journal.len(), journal + 1);
        assert!(!app.session.has_jobs());
        run_checked(&mut app, &before, revision, "xcf.filter.enable", json!({"filterId":id,"enabled":true}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&disabled));
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        assert_eq!(app.run("xcf.filter.set", json!({"filterId":id,"enabled":true})).unwrap()["updated"], false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&disabled));
        // External Cursor output; reader/native owner retain real same-q charges.
        let saved = photocraft_format::owned_document::save_current_document_archive(&disabled, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
            .unwrap()
            .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Current disabled group.pcraft", archive, &ctl()).unwrap();
        payload(&disabled, &reopened.session.active().unwrap().doc);
        compare(&reopened, &root, phase);
        assert_eq!(reopened.run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).unwrap()["updated"], false);
        assert_eq!(reopened.run("xcf.filter.set", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        payload(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(app.session.active().unwrap().source, source);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
}
