//! Larger synthetic CURRENT compatibility. Original pinned8x6 native inputs are
//! repeated literally, with no producer geometry or numerical oracle claim.
use super::*;
#[path = "../residual_point_geometry_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn preview(app: &PhotocraftApp, doc: &photocraft_engine::DocumentSnapshot) -> egui::ColorImage {
    let display = app.session.color.canvas_display_for(doc, None).unwrap();
    let (image, fee) = crate::canvas_backed_preview::backed_snapshot_display_image(
        doc,
        doc.size.width,
        doc.size.height,
        Some(display.as_ref()),
        &photocraft_raster::Interrupt::NONE,
    )
    .unwrap();
    assert_eq!(image.pixels.len(), doc.size.area() as usize);
    let external = image.clone();
    drop((image, fee));
    external
}
#[test]
#[ignore = "UNRUN:Seven exact residual/configured epochs and eligible diversity baselines, larger paid synthetic CURRENT/actual generic checkbox/typed Config-or-scalar request/readonly rename with unchanged visible+opacity1/history/query/title/native preview continuity; no larger producer source/numerical oracle or new mutation grant; visual testing deferred"]
fn existing_residual_epoch_ui_controls_keep_native_custody() {
    for index in 0..fixture::case_count() {
        let owner = fixture::build(index, photocraft_doc::Size::new(257, 259));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "synthetic-residual.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        fixture::same(owner.document(), &before);
        assert!(before.accounted().unwrap().quota().shares_ledger(owner.quota()));
        let initial = preview(&app, &before);
        let layer = before.layers[0].id;
        let node = fixture::node(&before);
        let id = node.id.0.to_string();
        let enabled = node.enabled;
        let query = photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":id}), &fixture::ctl()).unwrap();
        assert_eq!(query.value()["operation"]["name"], fixture::case(index).operation);
        drop(query);
        let params = fixture::changed(&before).unwrap_or_else(|| json!({"filterId":id,"enabled":!enabled}));
        let command = if fixture::changed(&before).is_some() { "xcf.filter.set" } else { "xcf.filter.enable" };
        app.run(command, params.clone()).unwrap();
        let edited = app.session.active().unwrap().doc.clone();
        fixture::retained(&before, &edited);
        assert_eq!(fixture::node(&edited).enabled, !enabled);
        app.run("edit.undo", Value::Null).unwrap();
        let history = app.session.active().unwrap().history.clone();
        assert_eq!(app.run("xcf.filter.enable", json!({"filterId":id,"enabled":enabled})).unwrap()["updated"], false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        // Actual existing generic checkbox consumer, rather than a new family widget.
        let label: String = fixture::node(&before)
            .source_display_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(fixture::case(index).operation)
            .chars()
            .take(128)
            .collect();
        let mut harness = Harness::builder().with_size(egui::vec2(900., 1000.)).build_ui_state(move |ui, app| super::filter_properties(app, ui, layer), app);
        harness.run_steps(2);
        harness.query_all_by_label(&label).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
        harness.run_steps(2);
        let app = harness.state_mut();
        assert_eq!(fixture::node(&app.session.active().unwrap().doc).enabled, enabled);
        let before_name = app.session.active().unwrap().doc.clone();
        assert!(app.run("layer.setProps", json!({"name":"Residual CURRENT metadata","visible":true,"opacity":1.0})).unwrap_err().contains("synchronous"));
        assert!(app.session.active().unwrap().doc.shares_state(&before_name));
        app.background_jobs = false;
        app.run("layer.setProps", json!({"name":"Residual CURRENT metadata","visible":true,"opacity":1.0})).unwrap();
        let current = app.session.active().unwrap().doc.clone();
        fixture::retained(&before, &current);
        let image = preview(app, &current);
        let titled = current.accounted().unwrap().try_with_document_name("Residual CURRENT continuation", &fixture::ctl()).unwrap();
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
        let snapshot = reopened.session.active().unwrap().doc.clone();
        fixture::same(titled.document(), &snapshot);
        assert_eq!(preview(&reopened, &snapshot), image);
        assert_eq!(image, initial);
        for (command, params) in [
            ("layer.translate", json!({"dx":1})),
            ("xcf.filter.move", json!({"filterId":id,"index":0})),
            ("xcf.filter.remove", json!({"filterId":id})),
            ("layer.setProps", json!({"name":"coalesce denied","coalesce":"key"})),
            ("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"unknown","value":{"Bool":true}}]})),
        ] {
            assert!(reopened.run(command, params).is_err());
            assert!(reopened.session.active().unwrap().doc.shares_state(&snapshot));
        }
    }
}
