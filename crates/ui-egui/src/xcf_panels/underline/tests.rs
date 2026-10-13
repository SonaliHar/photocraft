use crate::{PhotocraftApp, xcf_panels::text_properties};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;
#[test]
#[ignore = "requires independent unpublished underline originals"]
fn underline_character_controls_preserve_source_native_history_and_locks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let bytes = std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap();
    let original = photocraft_io::import("underline.xcf", &bytes).unwrap().document;
    let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("aa-32-whole.edit.xcf")).unwrap()).unwrap().document;
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    let hostile = crate::state::XcfUnderlineDraft {
        document: original.id.0,
        layer: layer.0,
        revision: app.session.active().unwrap().revision,
        start: 0,
        end: usize::MAX,
        content: "Ω".repeat(2048),
        ranges: vec![[usize::MAX, 0]; 16],
        validated: true,
        editable: true,
    };
    app.ui.xcf_underline_draft = Some(hostile);
    app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(!app.ui.xcf_underline_draft.as_ref().unwrap().validated);

    let mut h = Harness::builder().with_size(egui::vec2(420., 750.)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    let draft = h.state().ui.xcf_underline_draft.as_ref().unwrap();
    assert_eq!(draft.content, "HEFT");
    assert_eq!(draft.ranges, vec![[0, 4]]);
    assert_eq!(draft.end, 4);
    let inspected = crate::control::inspect(h.state(), &h.ctx).unwrap();
    assert_eq!(inspected["xcfUnderlineDraft"]["start"], 0);
    assert_eq!(inspected["xcfUnderlineDraft"]["end"], 4);

    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(1).expect("exclusive character end control").click();
    h.run_steps(2);
    for _ in 0..2 {
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(2);
    }
    h.get_by_label("Apply underline").click();
    h.run_steps(2);
    let current = h.state().session.active().unwrap().doc.as_ref().clone();
    let a = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap();
    let b = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(a, b);
    assert_eq!(current.layers, original.layers);
    let s = current.xcf_state.as_ref().unwrap();
    let o = original.xcf_state.as_ref().unwrap();
    assert_eq!(s.resources, o.resources);
    assert_eq!(s.items, o.items);
    assert_eq!(s.opaque, o.opaque);
    assert_eq!(s.source_items[0].cache, o.source_items[0].cache);
    let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), current);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, original);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, current);
    h.run_steps(2);
    h.get_by_label("Clear underline").click();
    h.run_steps(2);
    let cleared = h.state().session.active().unwrap().doc.as_ref().clone();
    assert_eq!(photocraft_engine::xcf_text_cmds::current_underline_ranges(&cleared, layer), Some(vec![]));
    let clear_reference = photocraft_io::import("clear.xcf", &std::fs::read(root.join("aa-32-single-attribute.edit.xcf")).unwrap()).unwrap().document;
    assert_eq!(
        photocraft_compose::gimp_render::try_render(&cleared, cleared.bounds(), &mut |_| Ok(())).unwrap(),
        photocraft_compose::gimp_render::try_render(&clear_reference, clear_reference.bounds(), &mut |_| Ok(())).unwrap()
    );
    let native = photocraft_format::save_to_bytes(&cleared, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), cleared);
    assert_eq!(cleared.layers, original.layers);
    assert_eq!(cleared.xcf_state.as_ref().unwrap().opaque, o.opaque);
    assert_eq!(cleared.xcf_state.as_ref().unwrap().source_items[0].cache, o.source_items[0].cache);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, current);
    h.state_mut().run("layer.setProps", json!({"layer":layer.0,"locks":{"position":true}})).unwrap();
    h.run_steps(2);
    assert!(h.get_by_label("Apply underline").accesskit_node().is_disabled());
    assert!(h.get_by_label("Clear underline").accesskit_node().is_disabled());
    assert_eq!(std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap(), bytes);
}

#[test]
fn character_positions_convert_checked_utf8_boundaries() {
    assert_eq!(super::byte_range("AΩ日本", 1, 3), Some([1, 6]));
    for (a, b) in [(0, 0), (2, 1), (0, 5), (usize::MAX, usize::MAX)] {
        assert_eq!(super::byte_range("AΩ日本", a, b), None);
    }
    assert_eq!(super::byte_range(&"x".repeat(257), 0, 1), None);
}

#[test]
fn underline_draft_roundtrip_exposes_positions_without_minting_authority() {
    let ui = crate::state::UiState {
        xcf_underline_draft: Some(crate::state::XcfUnderlineDraft {
            document: 3,
            layer: 4,
            revision: 5,
            start: 1,
            end: 3,
            content: "HEFT".into(),
            ranges: vec![[1, 3]],
            validated: true,
            editable: true,
        }),
        ..Default::default()
    };
    let json = serde_json::to_value(ui).unwrap();
    assert_eq!(json["xcf_underline_draft"]["start"], 1);
    assert!(json["xcf_underline_draft"].get("validated").is_none());
    let restored: crate::state::UiState = serde_json::from_value(json).unwrap();
    let draft = restored.xcf_underline_draft.unwrap();
    assert_eq!((draft.start, draft.end), (1, 3));
    assert!(!draft.validated && !draft.editable);
}

#[test]
fn underline_control_replacement_is_typed_bounded_and_atomic() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let call = |app: &mut PhotocraftApp, params| {
        let (request, _) = crate::control::ControlRequest::new("ui.set", params);
        let crate::control::Outcome::Done(value) = crate::control::handle(app, &ctx, &request) else { panic!("immediate view update") };
        value
    };
    assert_eq!(
        call(&mut app, json!({"xcfUnderlineDraft":{"document":1,"layer":2,"revision":3,"start":1,"end":3,"content":"HEFT","ranges":[[0,4]]}}))["ok"],
        true
    );
    assert_eq!(crate::control::inspect(&app, &ctx).unwrap()["xcfUnderlineDraft"]["start"], 1);
    let before = app.ui.clone();
    for draft in [
        json!({"unknown":1}),
        json!({"validated":true}),
        json!({"start":"1"}),
        json!({"content":"x".repeat(257)}),
        json!({"ranges":vec![[0,1];9]}),
        json!({"content":"HEFT","start":4,"end":2}),
        json!({"content":"Ω","ranges":[[0,1]]}),
    ] {
        assert_eq!(call(&mut app, json!({"tool":"move","xcfUnderlineDraft":draft}))["ok"], false);
        assert_eq!(app.ui, before);
    }
    assert!(app.session.active().is_none());
    assert_eq!(call(&mut app, json!({"xcfUnderlineDraft":null}))["ok"], true);
    assert!(app.ui.xcf_underline_draft.is_none());
}

#[test]
#[ignore = "requires independent unpublished underline originals"]
fn underline_actual_canvas_preserves_glyph_and_transparent_alpha() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let bytes = std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap();
    let doc = photocraft_io::import("underline.xcf", &bytes).unwrap().document;
    let full = photocraft_compose::gimp_render::try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
    let reduced = photocraft_compose::gimp_render::PreparedRenderer::new(&doc).unwrap().render_reduced(128, 64, None, &mut |_| Ok(())).unwrap();
    let full_alpha: Vec<_> = full.px.iter().map(|p| (p[3] * 255.0).round() as u8).collect();
    assert!(full_alpha.contains(&0) && full_alpha.contains(&255));
    let expected = reduced.to_rgba8();
    assert_eq!(expected.pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>(), full_alpha, "full→reduced alpha");
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc.clone(), None).unwrap();
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    assert_eq!(app.session.prefs().transparency_and_gamut.square(), Some(8.0));
    let ctx = egui::Context::default();
    let (texture, _) = crate::canvas::ensure_texture(&mut app, &ctx, 0, None).unwrap();
    let mut delta = ctx.tex_manager().write().take_delta();
    let image = delta.set.get(&texture).unwrap();
    let egui::ImageData::Color(image) = image.first().unwrap().image.clone();
    let actual: Vec<_> = image.pixels.iter().map(|pixel| pixel.a()).collect();
    delta.clear(); // The diagnostic inspected the upload instead of submitting it to a renderer.
    assert_eq!(actual, full_alpha, "actual canvas upload must preserve all glyph/transparent alpha");
    println!(
        "actual canvas alpha: transparent={} opaque={} intermediate={} checker={:?}",
        actual.iter().filter(|v| **v == 0).count(),
        actual.iter().filter(|v| **v == 255).count(),
        actual.iter().filter(|v| **v > 0 && **v < 255).count(),
        app.session.prefs().transparency_and_gamut.square()
    );
    assert_eq!(*app.session.active().unwrap().doc, doc);
    assert_eq!(std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap(), bytes);
}


#[test]
fn underline_hint_cache_requires_weak_identity_and_caches_absence() {
    use photocraft_engine::document_snapshot::DocumentSnapshot;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let doc = photocraft_doc::Document::new("Own underline cache", photocraft_doc::Size::new(2,2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let mut snapshot = DocumentSnapshot::from_resident(Arc::new(doc)).unwrap();
    let ctx = egui::Context::default();
    let calls = AtomicUsize::new(0);
    let derive = |_: &photocraft_doc::Document, _: photocraft_doc::LayerId| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Arc::new(super::UnderlineHints { content: "AΩ日本".into(), ranges: vec![[1,6]], editable: true })))
    };
    let first = super::hints_with(&ctx, &snapshot, 1, photocraft_doc::LayerId(7), derive).unwrap().unwrap();
    let hit = super::hints_with(&ctx, &snapshot, 1, photocraft_doc::LayerId(7), derive).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &hit));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    snapshot.try_resident_mut().unwrap().name = "Same revision, different snapshot".into();
    super::hints_with(&ctx, &snapshot, 1, photocraft_doc::LayerId(7), derive).unwrap();
    super::hints_with(&ctx, &snapshot, 2, photocraft_doc::LayerId(7), derive).unwrap();
    super::hints_with(&ctx, &snapshot, 2, photocraft_doc::LayerId(8), derive).unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 4);
    let absent = |_: &photocraft_doc::Document, _: photocraft_doc::LayerId| { calls.fetch_add(1, Ordering::Relaxed); Ok(None) };
    assert!(super::hints_with(&ctx, &snapshot, 3, photocraft_doc::LayerId(8), absent).unwrap().is_none());
    assert!(super::hints_with(&ctx, &snapshot, 3, photocraft_doc::LayerId(8), derive).unwrap().is_none());
    assert_eq!(calls.load(Ordering::Relaxed), 5);
}

#[test]
#[ignore = "retained underline runtime authority and live-context draft; not executed"]
fn runtime_validated_draft_never_overwrites_source_and_stale_apply_reports_error() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let original = photocraft_io::import("underline.xcf", &std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap()).unwrap().document;
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    let captured = app.session.active().unwrap().doc.clone();
    let rev = app.session.active().unwrap().revision;
    app.ui.xcf_underline_draft = Some(crate::state::XcfUnderlineDraft { document: original.id.0, layer: layer.0, revision: rev,
        validated: true, editable: true, content: "Ω".repeat(4096), ranges: vec![[usize::MAX,0];16], start: usize::MAX, end: usize::MAX });
    let mut harness = Harness::builder().with_size(egui::vec2(420.,720.)).build_ui_state(move |ui, app| super::properties(app, ui, layer), app);
    harness.run();
    let draft = harness.state().ui.xcf_underline_draft.as_ref().unwrap();
    assert_eq!(draft.content, "HEFT");
    assert_eq!(draft.ranges, vec![[0,4]]);
    assert_eq!((draft.start,draft.end), (4,4));
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    let app = harness.state_mut();
    app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].locks.position = true;
    assert!(!super::live_editable(app, &app.session.active().unwrap().doc, layer));
    super::apply_checked(app, &captured, rev, layer, json!({"layerId":layer.0.to_string(),"ranges":[]}));
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "xcf.properties.stale-target");
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
}
