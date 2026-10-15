//! UNRUN authored group packets with synthetic CURRENT metadata/mask edits.
//! Changed canvas comparisons are actual-adapter continuation, not GIMP oracles.
use super::*;
#[path = "../../canvas_gaussian_group_metadata_fixture.rs"]
mod fixture;
use fixture::{assets, ctl, ids, payload};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::io::Cursor;
fn display(app: &PhotocraftApp) -> egui::ColorImage {
    let (image, fee) = crate::canvas::assigned_geometry_test_support::display(&app.session.active().unwrap().doc).unwrap();
    let out = image.clone();
    drop((image, fee));
    out
}
#[test]
#[ignore = "UNRUN: Gaussian group all4 current LayerIDs combined metadata/key/reverseDFS/noopredo, actual physical mask control/stale/filtermask refusal, native continuation"]
fn gaussian_group_metadata_and_physical_mask_controls_keep_native_payloads() {
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
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, file, &carrier, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let targets = ids(&before);
        let group = targets[0];
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.setProps", json!({"layer":group.0,"visible":false,"coalesce":"group-current"})).unwrap();
        for (index, id) in targets.iter().enumerate() {
            app.run(
                "layer.setProps",
                json!({"layer":id.0,"name":format!("Current {index}"),"visible":index!=2,"opacity":if index==3{0.375}else{0.5},"coalesce":"group-current"}),
            )
            .unwrap();
        }
        let changed = app.session.active().unwrap().doc.clone();
        assert!(!app.session.has_jobs());
        payload(&before, &changed);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        for (index, id) in targets.iter().enumerate() {
            let layer = changed.layer(*id).unwrap();
            assert_eq!(layer.name, format!("Current {index}"));
            assert_eq!(layer.visible, index != 2);
            assert_eq!(layer.opacity.to_bits(), if index == 3 { 0.375f32.to_bits() } else { 0.5f32.to_bits() });
        }
        let enabled_image = display(&app);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        let old = before.layer(targets[3]).unwrap();
        app.run("layer.setProps", json!({"layer":targets[3].0,"name":old.name,"visible":old.visible,"opacity":f64::from(old.opacity),"coalesce":"other"}))
            .unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        let target = capture(&app, group).unwrap();
        assert!(permitted(&app, &target));
        let filter_mask = changed.xcf_state.as_ref().unwrap().filter_masks[0].id;
        assert_ne!(target.mask, filter_mask);
        let journal = app.session.journal.clone();
        let state = app.session.active().unwrap().doc.clone();
        assert!(app.run("xcf.mask.enabled.set", json!({"maskId":filter_mask.0.to_string(),"enabled":false})).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&state));
        let reply = apply(&mut app, &target, false).unwrap();
        assert_eq!(reply, json!({"maskId":target.mask.0.to_string(),"layerId":group.0.to_string(),"enabled":false,"changed":true}));
        assert_eq!(app.session.journal, journal);
        let disabled = app.session.active().unwrap().doc.clone();
        payload(&before, &disabled);
        assert!(!disabled.layer(group).unwrap().mask.as_ref().unwrap().enabled);
        assert!(disabled.xcf_state.as_ref().unwrap().filter_masks[0].surface.default_bytes().iter().all(|v| *v == 0));
        assert_eq!(apply(&mut app, &target, false).unwrap_err(), stale());
        let disabled_image = display(&app);
        assert_ne!(enabled_image.pixels, disabled_image.pixels);
        app.run("edit.undo", json!({})).unwrap();
        let history = app.session.active().unwrap().history.clone();
        let equal = capture(&app, group).unwrap();
        assert_eq!(apply(&mut app, &equal, true).unwrap()["changed"], false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", json!({})).unwrap();
        let saved = photocraft_format::owned_document::save_current_document_archive(
            &app.session.active().unwrap().doc,
            Cursor::new(Vec::new()),
            Default::default(),
            &q,
            &ctl(),
        )
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
        crate::owned_native_open::adopt_bytes(&mut reopened, "CURRENT metadata.pcraft", archive, &ctl()).unwrap();
        payload(&disabled, &reopened.session.active().unwrap().doc);
        assert_eq!(display(&reopened).pixels, disabled_image.pixels);
        // Duplicate current names exercise the existing reverse DFS resolver:
        // children[1], children[0], group, then backdrop; no new source names.
        for id in targets {
            reopened.run("layer.setProps", json!({"layer":id.0,"name":"Duplicate"})).unwrap();
        }
        reopened.run("layer.setProps", json!({"layer":"Duplicate","opacity":0.25})).unwrap();
        assert_eq!(reopened.session.active().unwrap().doc.layer(targets[2]).unwrap().opacity.to_bits(), 0.25f32.to_bits());
        assert_eq!(reopened.session.active().unwrap().doc.layer(targets[1]).unwrap().opacity.to_bits(), 0.5f32.to_bits());
        payload(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), bytes);
    }
}
