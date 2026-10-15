//! UNRUN registered CURRENT metadata; source qualification remains unchanged.
use super::*;
#[path = "canvas_separable_metadata_fixture.rs"]
mod fixture;
#[path = "canvas_separable_blend_fixture.rs"]
#[allow(dead_code)]
mod original;
use fixture::{payload, reference};
use original::{cases, ctl};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
use std::io::Cursor;
fn preview(doc: &photocraft_engine::DocumentSnapshot, root: &std::path::Path, case: &original::Case) {
    let expected = reference(root, case, doc);
    let (image, fee) = backed_snapshot_display_image(doc, 16, 12, None, &Interrupt::NONE).unwrap();
    for (actual, p) in image.pixels.iter().zip(expected) {
        let q = p.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        let q = egui::Color32::from_rgba_unmultiplied(q[0], q[1], q[2], q[3]).to_array();
        for c in 0..4 {
            assert!((i16::from(actual.to_array()[c]) - i16::from(q[c])).abs() <= 1);
        }
    }
    drop((image, fee));
}
#[test]
#[ignore = "UNRUN:50 authored flat blend CURRENT metadata via real bg-enabled UI dispatch, bothIDs/coalesce/firstlabel/noopredo/-0/topmost/name/native continuation; source/raw policies exact"]
fn flat_composite_ui_current_metadata_is_synchronous_and_keeps_native_payloads() {
    let (root, cases) = cases();
    for case in cases {
        let bytes = std::fs::read(root.join(&case.file)).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let carrier = photocraft_io::import_owned_xcf_from_reader_in_quota(
            &case.file,
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(matches!(carrier, photocraft_io::OwnedXcfImport::SeparableBlend(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, &case.file, &carrier, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let base = before.layers[0].id;
        let top = before.layers[1].id;
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.setProps", json!({"layer":top.0,"visible":false,"coalesce":"flat-metadata"})).unwrap();
        app.run("layer.setProps", json!({"layer":base.0,"name":"Current base","opacity":0.375,"coalesce":"flat-metadata"})).unwrap();
        app.run("layer.setProps", json!({"layer":top.0,"name":"Current top","visible":true,"opacity":0.25,"coalesce":"flat-metadata"})).unwrap();
        assert!(!app.session.has_jobs());
        assert!(app.jobs.last_started.is_none());
        let changed = app.session.active().unwrap().doc.clone();
        payload(&before, &changed);
        preview(&changed, &root, &case);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let redo = app.session.active().unwrap().history.clone();
        let layer = before.layer(top).unwrap();
        app.run("layer.setProps", json!({"layer":top.0,"name":layer.name,"visible":layer.visible,"opacity":f64::from(layer.opacity),"coalesce":"noop-other"}))
            .unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&redo));
        app.run("edit.redo", json!({})).unwrap();
        for id in [base, top] {
            app.run("layer.setProps", json!({"layer":id.0,"name":"Duplicate"})).unwrap();
        }
        app.run("layer.setProps", json!({"layer":"Duplicate","opacity":-0.0})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(top).unwrap().opacity.to_bits(), (-0.0f32).to_bits());
        assert_eq!(app.session.active().unwrap().doc.layer(base).unwrap().opacity.to_bits(), 0.375f32.to_bits());
        let zero = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.clone();
        app.run("layer.setProps", json!({"layer":top.0,"opacity":-0.0,"name":false})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&zero));
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        payload(&before, &zero);
        preview(&zero, &root, &case);
        for (id, p) in [("layer.translate", json!({"dx":1})), ("select.all", Value::Null), ("xcf.filter.enable", json!({"filterId":"1","enabled":false}))] {
            assert!(app.run(id, p).is_err());
        }
        assert!(app.session.active().unwrap().doc.shares_state(&zero));
        let title = zero.accounted().unwrap().try_with_document_name("Metadata native", &ctl()).unwrap();
        let encoded =
            photocraft_format::owned_document::save_current_document_archive(title.document(), Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                .unwrap()
                .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(encoded.as_slice()),
            encoded.len() as u64,
            Default::default(),
            q,
            &ctl(),
        )
        .unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Different label.pcraft", archive, &ctl()).unwrap();
        let current = reopened.session.active().unwrap().doc.clone();
        assert_eq!(current.name, "Metadata native");
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(reopened.session.active().unwrap().path.is_none());
        assert_eq!(zero.xcf_state, current.xcf_state);
        payload(&zero, &current);
        preview(&current, &root, &case);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), bytes);
    }
}
