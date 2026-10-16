//! Actual readonly control consumer; producer math/profile variation stays HOLD.
use super::*;
#[path = "../filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
use std::sync::Arc;
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
    let external = image.clone();
    drop((image, fee));
    external
}
#[test]
#[ignore = "UNRUN:twoactualauthoredmixedsource files pluspaidlargersyntheticCURRENT/readonlyUIlabel/publictypedsourceandnative/query/title/displaypersistence andmutationrefusal; display continuity only, no producerprojection/profile/numericoracle; visualtestingdeferred"]
fn filtered_flat_readonly_ui_keeps_every_root() {
    for index in 0..2 {
        let imported = Arc::new(fixture::import(index));
        let mut source_app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut source_app, "author-mixed.xcf", &imported, &fixture::ctl()).unwrap();
        let source = source_app.session.active().unwrap().doc.clone();
        fixture::observed(&source, index);
        assert!(source_app.session.active().unwrap().source_read_only);
        let image = preview(&source_app, &source);
        let mut native_app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut native_app, "author.pcraft", fixture::archive(source.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&source, &native_app.session.active().unwrap().doc);
        assert_eq!(preview(&native_app, &native_app.session.active().unwrap().doc), image);
        let current = fixture::activity(&fixture::larger(imported));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "current.pcraft", fixture::archive(&current), &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let layer = before.layers[1].id;
        let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        app.background_jobs = true;
        let mut h = Harness::builder().with_size(egui::vec2(600., 400.)).build_ui_state(move |ui, app| super::filter_properties(app, ui, layer), app);
        h.run_steps(2);
        assert!(h.query_all_by_label("Filter editing is unavailable").next().is_some());
        let app = h.state_mut();
        for (id, p) in [
            ("xcf.filter.enable", json!({"filterId":filter,"enabled":false})),
            ("layer.setProps", json!({"name":"denied"})),
            ("layer.translate", json!({"dx":0})),
            ("layer.select", json!({"index":0})),
        ] {
            assert!(app.run(id, p).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
        }
        let reply = photocraft_engine::accounted_xcf_inspect::prepare(&app.session, &json!({"offset":1,"limit":2}), &fixture::ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 7);
        assert_eq!(reply.value()["filters"]["total"], 3);
        drop(reply);
        let expected = preview(app, &before);
        let titled = before.accounted().unwrap().try_with_document_name("Readonly mixed title", &fixture::ctl()).unwrap();
        let mut final_app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut final_app, "titled.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
        let after = final_app.session.active().unwrap().doc.clone();
        fixture::same(titled.document(), &after);
        assert_eq!(preview(&final_app, &after), expected);
    }
}
