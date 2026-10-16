//! UNRUN targeted mixed controls. Display/native continuity is not a GIMP oracle.
use super::*;
#[path = "../filtered_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::Value;
fn app(index: usize) -> PhotocraftApp {
    let source = fixture::import(index);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_xcf_open::adopt_bytes(&mut app, "mixed.xcf", &source, &fixture::ctl()).unwrap();
    app.background_jobs = true;
    app
}
fn root(snapshot: &DocumentSnapshot, operation: &str) -> LayerId {
    let stack = snapshot.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.nodes[0].operation.name == operation).unwrap();
    let ItemRef::Layer(root) = stack.owner else { panic!("root") };
    root
}
fn native_unchanged(a: &DocumentSnapshot, b: &DocumentSnapshot) {
    assert!(b.accounted().unwrap().is_readonly_filtered_flat_point_storage());
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.layers.len(), b.layers.len());
    for (x, y) in a.layers.iter().zip(&b.layers) {
        assert_eq!(x.id, y.id);
        assert_eq!(fixture::raw(x), fixture::raw(y));
    }
    assert_eq!(a.xcf_state.as_ref().unwrap().filter_masks, b.xcf_state.as_ref().unwrap().filter_masks);
}
fn preview(app: &PhotocraftApp, doc: &DocumentSnapshot) -> egui::ColorImage {
    let display = app.session.color.canvas_display_for(doc, None).unwrap();
    let (image, fee) =
        crate::canvas_backed_preview::backed_snapshot_display_image(doc, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
    let copy = image.clone();
    drop((image, fee));
    copy // external display capture only
}
#[test]
#[ignore = "UNRUN actual mixed nonzero-root typed widgets/enable, target-only revisions, history/native/display continuity; visual testing deferred"]
fn three_widgets_target_actual_roots_and_continue_native() {
    for index in 0..2 {
        for (operation, label, value) in [("gimp:semi-flatten", "Red", "0.4375"), ("gegl:hue-chroma", "Hue", "17.25"), ("gegl:vibrance", "Vibrance", "-12.5")] {
            let app = app(index);
            let before = app.session.active().unwrap().doc.clone();
            let layer = root(&before, operation);
            assert_ne!(before.layers[0].id, layer);
            let old_revision = app.session.active().unwrap().revision;
            let (_, old_stack_revision) = node(&before, layer).unwrap();
            let mut h =
                Harness::builder().with_size(egui::vec2(800., 600.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, layer), app);
            h.run_steps(2);
            assert!(h.query_all_by_label("Filter editing is unavailable").next().is_none());
            h.get_by_label(label).click();
            h.run_steps(2);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get_by_label(label).type_text(value);
            h.key_press(egui::Key::Enter);
            h.run_steps(2);
            let app = h.state_mut();
            let after = app.session.active().unwrap().doc.clone();
            assert_eq!(app.session.active().unwrap().revision, old_revision + 1);
            let (changed, revision) = node(&after, layer).unwrap();
            assert_eq!(revision, old_stack_revision + 1);
            let expected = value.parse::<f32>().unwrap().to_bits();
            if operation == "gimp:semi-flatten" {
                assert_eq!(semi_flatten::bits(&changed.arguments[0].value).unwrap().0[0], expected);
            } else {
                let arg = changed.arguments.iter().find(|a| a.name == if operation == "gegl:hue-chroma" { "hue" } else { "vibrance" }).unwrap();
                assert_eq!(arg.value, FilterValue::Number(ExactNumber::Float32Bits(expected)));
            }
            for old in &before.xcf_state.as_ref().unwrap().filter_stacks {
                if old.owner != ItemRef::Layer(layer) {
                    assert_eq!(Some(old), after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == old.owner));
                }
            }
            native_unchanged(&before, &after);
            let image = preview(app, &after);
            app.run("edit.undo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            app.run("edit.redo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&after));
            let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut reopened, "edited.pcraft", fixture::archive(after.accounted().unwrap()), &fixture::ctl()).unwrap();
            fixture::same(&after, &reopened.session.active().unwrap().doc);
            assert_eq!(preview(&reopened, &reopened.session.active().unwrap().doc), image);
            let target = capture(&reopened, layer).unwrap();
            let filter = target.filter;
            submit(&mut reopened, &target, "xcf.filter.enable", json!({"filterId":filter.0.to_string(),"enabled":false}));
            assert!(!reopened.ui.status_error);
            assert!(!node(&reopened.session.active().unwrap().doc, layer).unwrap().0.enabled);
        }
    }
}
#[test]
#[ignore = "UNRUN captured doc/root/stack/filter/revision fences, combined typed request atomicity, no-op and unrelated action closures"]
fn captured_targets_and_literal_words_are_not_retargeted() {
    let mut app = app(1);
    let before = app.session.active().unwrap().doc.clone();
    let layer = root(&before, "gegl:hue-chroma");
    let target = capture(&app, layer).unwrap();
    let original = node(&before, layer).unwrap().0;
    let (command, params) = hue_chroma::request(original, [-0., 12.5, -8.25], false).unwrap();
    assert_eq!(command, "xcf.filter.set");
    assert_eq!(params["enabled"], false);
    let past = app.session.active().unwrap().history.past_len();
    submit(&mut app, &target, command, params);
    assert!(!app.ui.status_error);
    let after = app.session.active().unwrap().doc.clone();
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
    let changed = node(&after, layer).unwrap().0;
    assert_eq!(hue_chroma::words(changed).unwrap(), [-0f32, 12.5, -8.25].map(f32::to_bits));
    assert!(hue_chroma::request(changed, [-0., 12.5, -8.25], false).is_none());
    submit(&mut app, &target, "xcf.filter.enable", json!({"filterId":target.filter.0.to_string(),"enabled":true}));
    assert_eq!(app.ui.status, "xcf.properties.stale-target");
    assert!(app.session.active().unwrap().doc.shares_state(&after));
    for fault in 0..3 {
        let mut target = capture(&app, layer).unwrap();
        if fault == 0 {
            target.root = root(&after, "gegl:vibrance");
        } else if fault == 1 {
            target.stack_revision += 1;
        } else {
            target.filter = node(&after, root(&after, "gimp:semi-flatten")).unwrap().0.id;
        }
        let revision = app.session.active().unwrap().revision;
        let past = app.session.active().unwrap().history.past_len();
        submit(&mut app, &target, "xcf.filter.enable", json!({"filterId":changed.id.0.to_string(),"enabled":true}));
        assert!(app.ui.status_error);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.active().unwrap().history.past_len(), past);
        assert!(app.session.active().unwrap().doc.shares_state(&after));
    }
    let target = capture(&app, layer).unwrap();
    app.session.active_mut().unwrap().revision += 1;
    assert!(!validate(&app, &target));
    app.session.active_mut().unwrap().revision -= 1;
    let color = node(&after, root(&after, "gimp:semi-flatten")).unwrap().0;
    let original = &color.arguments[0].value;
    let (words, encoding) = semi_flatten::bits(original).unwrap();
    let patched = semi_flatten::patch(original, [-0., 0.25, 0.75]).unwrap();
    let (next, next_encoding) = semi_flatten::bits(&patched).unwrap();
    assert_eq!(next[3], words[3]);
    assert_eq!(encoding, next_encoding);
    for (id, params) in
        [("xcf.filter.remove", json!({"filterId":changed.id.0.to_string(),"index":0})), ("layer.translate", json!({"dx":0})), ("layer.select", json!({"index":0}))]
    {
        assert!(app.run(id, params).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&after));
    }
}
