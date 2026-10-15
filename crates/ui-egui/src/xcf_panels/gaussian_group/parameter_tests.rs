//! UNRUN genuine source and typed UI parameter controls/current persistence.
//! Output checks are current-renderer continuity; numerical acceptance deferred.
use super::*;
#[path = "../../canvas_gaussian_group_parameter_fixture.rs"]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use fixture::{assets, bits, ctl, matrix, payload, wire};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::io::Cursor;
#[test]
#[ignore = "UNRUN: Gaussian CURRENT typed x/y UI parser/guarded controls, atomic optionalenabled/noopredo/stale/native and signedzero/max boundaries"]
fn typed_gaussian_group_parameters_keep_source_roles_and_current_bits() {
    assert_eq!(parse("-0").unwrap().to_bits(), (-0.0f64).to_bits());
    assert_eq!(parse("15"), Some(15.));
    for invalid in ["NaN", "inf", "-1", "15.000001"] {
        assert!(parse(invalid).is_none());
    }
    let root = assets();
    for phase in ["source", "edit"] {
        let file = if phase == "source" { "group-28-group-mask.xcf" } else { "group-28-group-mask.edit.xcf" };
        let source = std::fs::read(root.join(file)).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let carrier = photocraft_io::import_owned_xcf_from_reader_in_quota(
            file,
            &mut Cursor::new(source.as_slice()),
            source.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, file, &carrier, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id.0.to_string();
        let photocraft_doc::item_identity::ItemRef::Layer(group) = before.xcf_state.as_ref().unwrap().filter_stacks[0].owner else { panic!("group") };
        assert!(candidate(&before, group, node));
        assert!(!candidate(&before, before.layers[0].id, node));
        let copied_node = node.clone(); // External harness copy is not retained-node authority.
        assert!(!candidate(&before, group, &copied_node));
        let original = bits(&before);
        for (index, axes) in matrix().into_iter().enumerate() {
            let revision = app.session.active().unwrap().revision;
            let history = app.session.active().unwrap().history.past_len();
            let enabled = index != 3;
            run_checked(&mut app, &before, revision, "xcf.filter.set", wire(&id, axes.map(Some), Some(enabled)));
            assert!(!app.ui.status_error);
            let changed = app.session.active().unwrap().doc.clone();
            assert_eq!(bits(&changed), axes);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled, enabled);
            assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0].revision, before.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
            payload(&before, &changed);
            assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
            assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Set GIMP Group Filter Parameters"));
            assert!(!app.session.has_jobs());
            let (image, fee) = crate::canvas::assigned_geometry_test_support::display(&changed).unwrap();
            let reference = image.clone();
            drop((image, fee));
            run_checked(&mut app, &before, revision, "xcf.filter.set", wire(&id, [Some(original[0]), None], None));
            assert_eq!(app.ui.status, "xcf.properties.stale-target");
            assert!(app.session.active().unwrap().doc.shares_state(&changed));
            app.run("edit.undo", json!({})).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            let redo = app.session.active().unwrap().history.clone();
            assert_eq!(app.run("xcf.filter.set", wire(&id, original.map(Some), Some(true))).unwrap()["updated"], false);
            assert!(app.session.active().unwrap().history.shares_accounted_history(&redo));
            app.run("edit.redo", json!({})).unwrap();
            let saved = photocraft_format::owned_document::save_current_document_archive(&changed, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
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
            let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut reopened, "Current Gaussian parameters.pcraft", archive, &ctl()).unwrap();
            let current = &reopened.session.active().unwrap().doc;
            payload(&changed, current);
            assert_eq!(bits(current), axes);
            let (image, fee) = crate::canvas::assigned_geometry_test_support::display(current).unwrap();
            assert_eq!(image.pixels, reference.pixels);
            drop((image, fee));
            app.run("edit.undo", json!({})).unwrap();
        }
        // Render the actual joined panel and commit through its labelled F32
        // DragValue, rather than treating a command-only call as widget coverage.
        let mut widget_app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut widget_app, file, &carrier, &ctl()).unwrap();
        widget_app.background_jobs = true;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(520., 600.))
            .build_ui_state(move |ui, app| crate::xcf_panels::filter_properties(app, ui, group), widget_app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        assert!(!harness.get_by_label("Horizontal standard deviation").accesskit_node().is_disabled());
        assert!(!harness.get_by_label("Vertical standard deviation").accesskit_node().is_disabled());
        harness.get_by_label("Horizontal standard deviation").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Horizontal standard deviation").type_text("-0");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert_eq!(bits(&harness.state().session.active().unwrap().doc)[0], (-0.0f32).to_bits());
        payload(&before, &harness.state().session.active().unwrap().doc);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
        assert!(!harness.state().session.has_jobs());
        assert_eq!(std::fs::read(root.join(file)).unwrap(), source);
    }
}
