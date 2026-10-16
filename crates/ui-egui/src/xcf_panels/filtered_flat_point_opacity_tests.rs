//! Deferred percent UI and rawF64/loadedblack transaction controls, not visual/numeric acceptance.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_opacity_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN percentopacity widget explicit37.5→unit.375, arbitraryF64 passive/family-only preservation, loadedblack endpoint untouched, compound stop+enable+opacity one revision, sharedframe stale rejection and native10. No producer partial Source/visual oracle."]
fn opacity_percent_keeps_raw_words_until_intent() {
    for endpoint in [0x3dcc_cccd, 0xbdcc_cccd] {
        let original = fixture::loaded_mixed_current(0, endpoint);
        let root = original.document().layers[1].id;
        let raw = 0x3fe4000000000001u64;
        let owner = fixture::current(&original, |d| {
            d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().find(|s| s.owner == ItemRef::Layer(root)).unwrap().nodes[0].opacity_f64_bits = raw
        });
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "derived-current.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let mut h = Harness::builder().with_size(egui::vec2(800., 1200.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(3);
        assert!(h.state_mut().session.active().unwrap().doc.shares_state(&before));
        h.get_by_label("Exposure").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Exposure").type_text("0.75");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        {
            let app = h.state_mut();
            assert!(!app.ui.status_error);
            let changed = app.session.active().unwrap().doc.clone();
            let node = &changed.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap().nodes[0];
            assert_eq!(node.opacity_f64_bits, raw);
            assert_eq!(
                node.arguments.iter().find(|a| a.name == "black-level").unwrap().value,
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(endpoint))
            );
            let revision = app.session.active().unwrap().revision;
            let first = capture_snapshot(&changed, revision, root, 0).unwrap();
            let second = capture_snapshot(&changed, revision, root, 0).unwrap();
            let base = Some((
                "xcf.filter.set",
                json!({"filterId":node.id.0.to_string(),"enabled":false,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.875f32.to_bits()}}}]}),
            ));
            let params = opacity_request(node, base, 37.5 / 100.0, true).unwrap();
            assert_eq!(params.1["arguments"].as_array().unwrap().len(), 1);
            let stackrev = changed.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap().revision;
            submit(app, &first, params.0, params.1);
            assert!(!app.ui.status_error);
            let once = app.session.active().unwrap().doc.clone();
            assert_eq!(app.session.active().unwrap().revision, revision + 1);
            let stack = once.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap();
            assert_eq!(stack.revision, stackrev + 1);
            assert_eq!(stack.nodes[0].opacity_f64_bits, 0.375f64.to_bits());
            assert!(!stack.nodes[0].enabled);
            submit(app, &second, "xcf.filter.set", json!({"filterId":second.filter.0.to_string(),"opacityF64Bits":"0x3fe0000000000000"}));
            assert!(app.session.active().unwrap().doc.shares_state(&once));
            app.session.execute("edit.undo", json!({})).unwrap();
            fixture::same(&changed, &app.session.active().unwrap().doc);
        }
        h.run_steps(2);
        h.get_by_label("Filter opacity").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Filter opacity").type_text("37.5");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        let current = app.session.active().unwrap().doc.clone();
        let node = &current.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap().nodes[0];
        assert_eq!(node.opacity_f64_bits, 0.375f64.to_bits());
        assert_eq!(
            node.arguments.iter().find(|a| a.name == "black-level").unwrap().value,
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(endpoint))
        );
        fixture::payload(&before, &current);
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "current.pcraft", fixture::archive(current.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&current, &reopened.session.active().unwrap().doc);
    }
}
