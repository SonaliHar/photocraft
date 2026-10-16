use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn preview_app() -> PhotocraftApp {
    use base64::Engine;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut manifest: Value = serde_json::from_str(include_str!("../../../../../examples/extensions/quick-tools/manifest.json")).unwrap();
    manifest["panel"]["preview"] = json!(true);
    manifest["panel"]["buttons"][0]["label"] = json!("Generate preview");
    let data = base64::engine::general_purpose::STANDARD.encode(crate::extension_ui::tests::module(&manifest.to_string()));
    app.run("extension.install", json!({"data": data})).unwrap();
    app.run("file.new", json!({"width":512,"height":384,"background":"white"})).unwrap();
    crate::extension_ui::menu(&mut app, "window.extension.org.photocraft.quick-tools").unwrap().unwrap();
    app
}

fn panel_harness(size: egui::Vec2, app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    // Font registration takes effect on the following pass. Match the scoped native-panel
    // harnesses elsewhere in the shell: set up first, then draw themed controls.
    let mut ready = false;
    Harness::builder().with_size(size).build_ui_state(
        move |ui, app| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Pro);
                ready = true;
                return;
            }
            crate::extension_ui::windows(app, ui.ctx());
        },
        app,
    )
}

#[test]
fn comparison_maps_output_pixels_to_the_same_source_region() {
    let output = PixelRect::new(7, 11, 512, 398);
    let source = mapped_crop(output, 2048, 1536, 512, 384);
    assert_eq!(source, PixelRect::new(1, 2, 128, 100));
    let uv = mapped_uv(output, source, 2048, 1536, 512, 384);
    assert!((uv.left() * source.width() as f32 + source.x0 as f32 - 1.75).abs() < 0.0001);
    assert!((uv.top() * source.height() as f32 + source.y0 as f32 - 2.75).abs() < 0.0001);
    assert!((uv.right() * source.width() as f32 + source.x0 as f32 - 128.0).abs() < 0.0001);
    assert_eq!(crop(200_000, 120_000, egui::vec2(1.0, 1.0), egui::vec2(1536.0, 900.0)), PixelRect::new(198_464, 119_100, 200_000, 120_000));
}

#[test]
fn comparison_reuses_textures_and_its_slider_accepts_keyboard_input() {
    struct State {
        app: PhotocraftApp,
        view: View,
        source: Arc<Document>,
        result: Arc<Document>,
    }
    let app = preview_app();
    let source = app.session.active().unwrap().doc.clone();
    let mut output = (*source).clone();
    output.size.width *= 2;
    output.size.height *= 2;
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, state: &mut State| comparison(&state.app, ui, &mut state.view, Some(&state.source), Some(&state.result), 560.0),
        State { app, view: View::default(), source, result: Arc::new(output) },
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    let original = h.state().view.before.as_ref().unwrap().texture.id();
    let enhanced = h.state().view.after.as_ref().unwrap().texture.id();
    h.run_steps(3);
    assert_eq!(h.state().view.before.as_ref().unwrap().texture.id(), original);
    assert_eq!(h.state().view.after.as_ref().unwrap().texture.id(), enhanced);
    let from = h.get_by_label("Image comparison").rect().center();
    let to = from - egui::vec2(100.0, 0.0);
    h.hover_at(from);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().view.split < 0.5, "the divider moves with a pointer drag");
    let split = h.state().view.split;
    h.get_by_label("Comparison").focus();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    assert!(h.state().view.split > split, "comparison has a keyboard-operable slider");
    h.get_by_label("100%").click();
    h.run_steps(3);
    assert!(h.state().view.actual_size);
    let crop = h.state().view.after.as_ref().unwrap().crop.unwrap();
    assert!(crop.width() <= THUMBNAIL_SIDE && crop.height() <= THUMBNAIL_SIDE);
    assert_ne!(h.state().view.after.as_ref().unwrap().texture.id(), enhanced);
}

#[test]
fn preview_panel_keeps_apply_disabled_until_a_current_result_and_done_visible() {
    use egui_kittest::kittest::NodeT;
    for size in [egui::vec2(1100.0, 800.0), egui::vec2(640.0, 480.0)] {
        let mut h = panel_harness(size, preview_app());
        h.run_steps(4);
        assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
        let done = h.get_by_label("Done");
        assert!(done.rect().bottom() <= size.y, "Done remains visible at {size:?}: {:?}", done.rect());
        done.click();
        h.run_steps(3);
        assert!(!h.state().ui.extensions.panels["org.photocraft.quick-tools"].open);
    }
}

fn drag_panel(h: &mut Harness<'_, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=6 {
        h.hover_at(from.lerp(to, step as f32 / 6.0));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn preview_window_can_shrink_and_grow_without_changing_the_image() {
    let viewport = egui::vec2(1800.0, 1100.0);
    let mut h = panel_harness(viewport, preview_app());
    h.run_steps(5);
    let id = egui::Id::new(("extension-preview", "org.photocraft.quick-tools"));
    let window_rect = |h: &Harness<'_, PhotocraftApp>| h.ctx.memory(|memory| memory.area_rect(id)).unwrap();
    let source = h.state().session.active().unwrap().doc.clone();
    let original = window_rect(&h);
    // Keep the wide layout active: changing width as well would switch to the compact
    // layout and miss a divider that makes a wide window grow back to full height.
    let bottom = original.center_bottom() - egui::vec2(0.0, 1.0);
    drag_panel(&mut h, bottom, bottom - egui::vec2(0.0, 180.0));
    let shorter = window_rect(&h);
    assert!(shorter.height() < original.height() - 100.0, "wide window shrinks vertically: {original:?} -> {shorter:?}");
    h.run_steps(20);
    let initial = window_rect(&h);
    assert!((initial.height() - shorter.height()).abs() < 2.0, "height must not snap back: {shorter:?} -> {initial:?}");
    let image = h.get_by_label("Image comparison").rect();
    let corner = initial.right_bottom() - egui::vec2(1.0, 1.0);
    drag_panel(&mut h, corner, corner - egui::vec2(350.0, 100.0));
    let smaller = window_rect(&h);
    assert!(smaller.width() < initial.width() - 250.0, "window shrinks horizontally: {initial:?} -> {smaller:?}");
    assert!(smaller.height() < initial.height() - 50.0, "window shrinks vertically: {initial:?} -> {smaller:?}");
    h.run_steps(15);
    let stable = window_rect(&h);
    assert!((stable.size() - smaller.size()).length() < 2.0, "resized window stays the chosen size: {smaller:?} -> {stable:?}");
    assert!(Rect::from_min_size(Pos2::ZERO, viewport).contains_rect(h.get_by_label("Done").rect()));
    let corner = stable.right_bottom() - egui::vec2(1.0, 1.0);
    drag_panel(&mut h, corner, corner + egui::vec2(250.0, 120.0));
    let larger = window_rect(&h);
    assert!(larger.width() > stable.width() + 150.0 && larger.height() > stable.height() + 60.0, "window grows in both directions: {stable:?} -> {larger:?}");
    h.run_steps(20);
    assert!((window_rect(&h).size() - larger.size()).length() < 2.0, "the wide layout must keep the enlarged size");
    assert_ne!(h.get_by_label("Image comparison").rect().size(), image.size());
    assert!(Arc::ptr_eq(&source, &h.state().session.active().unwrap().doc));
    assert_eq!(h.state().session.documents().len(), 1);
}

#[test]
fn preview_apply_respects_open_dialogs_and_pending_crop_without_consuming_the_result() {
    use egui_kittest::kittest::NodeT;

    struct State {
        app: PhotocraftApp,
        panel: PanelState,
        manifest: Manifest,
    }
    for blocking in ["dialog", "crop"] {
        let mut app = preview_app();
        let manifest = app.session.extensions.installed["org.photocraft.quick-tools"].extension.manifest.clone();
        let source = app.session.active().unwrap().doc.clone();
        if blocking == "dialog" {
            crate::filter_dialog::open(&mut app, "image.imageSize").unwrap();
        } else {
            app.ui.tool = crate::state::Tool::Crop;
            app.ui.crop_rect = Some([10.0, 10.0, 100.0, 80.0]);
            app.crop.default_frame = false;
            assert!(crate::crop_ui::pending(&app));
        }
        assert!(!crate::menus::modal_allows(&app, "image.pluginPreview.apply"));
        // Rendering the button independently of validate() lets this test exercise the shell
        // gate without requiring a native processing package or accepting a fabricated token.
        let panel = PanelState { open: true, preview: Some(42), ..Default::default() };
        let mut ready = false;
        let mut h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(
            move |ui, state: &mut State| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Pro);
                    ready = true;
                    return;
                }
                window(&state.app, ui.ctx(), &state.manifest, &mut state.panel, None);
            },
            State { app, panel, manifest },
        );
        h.run_steps(3);
        assert!(h.get_by_label("Apply").accesskit_node().is_disabled(), "{blocking}");
        let state = h.state_mut();
        apply(&mut state.app, &mut state.panel, "document");
        assert_eq!(state.panel.preview, Some(42));
        assert!(state.panel.message.contains("Finish or cancel"), "{blocking}");
        assert_eq!(state.app.session.documents().len(), 1);
        assert!(Arc::ptr_eq(&source, &state.app.session.active().unwrap().doc));
    }
}

#[test]
fn changing_settings_discards_a_staged_token_and_never_applies_it() {
    let mut app = preview_app();
    let mut panel = PanelState { preview: Some(42), values: json!({"percent":300}), preview_values: json!({"percent":200}), ..Default::default() };
    validate(&mut app, &mut panel);
    assert!(panel.preview.is_none());
    assert!(panel.message.contains("Settings changed"));
    assert!(!panel.error);
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn saved_panel_settings_never_restore_session_only_preview_tokens() {
    let panel = PanelState {
        open: true,
        values: json!({"percent":200}),
        preview: Some(42),
        job: Some(8),
        preview_values: json!({"percent":200}),
        ..Default::default()
    };
    let serialized = serde_json::to_value(&panel).unwrap();
    assert!(serialized.get("preview").is_none());
    assert!(serialized.get("job").is_none());
    let restored: PanelState = serde_json::from_value(serialized).unwrap();
    assert!(restored.open);
    assert_eq!(restored.values, panel.values);
    assert!(restored.preview.is_none() && restored.job.is_none());
    assert!(restored.preview_view.before.is_none() && restored.preview_view.after.is_none());
}

#[test]
fn cached_display_texture_does_not_keep_a_full_size_result_alive() {
    let app = preview_app();
    let document = Arc::new((*app.session.active().unwrap().doc).clone());
    let weak = Arc::downgrade(&document);
    let context = egui::Context::default();
    let mut view = View::default();
    cached_image(&app, &context, &mut view.after, &document, None);
    assert!(view.after.is_some());
    drop(document);
    assert!(weak.upgrade().is_none(), "closing, replacing or disabling a plugin must release its full-size result even if the display cache remains");
    assert!(view.after.is_some(), "only the bounded display texture remains cached");
}

#[test]
fn clearing_disabled_plugin_textures_preserves_comparison_preferences() {
    let app = preview_app();
    let document = app.session.active().unwrap().doc.clone();
    let context = egui::Context::default();
    let mut view = View { split: 0.75, actual_size: true, center: egui::vec2(0.2, 0.6), new_layer: true, ..Default::default() };
    cached_image(&app, &context, &mut view.before, &document, None);
    cached_image(&app, &context, &mut view.after, &document, None);
    assert!(view.before.is_some() && view.after.is_some());
    view.clear_textures();
    assert!(view.before.is_none() && view.after.is_none());
    assert_eq!(view.split, 0.75);
    assert!(view.actual_size && view.new_layer);
    assert_eq!(view.center, egui::vec2(0.2, 0.6));
}

/// This invokes the bundled Vulkan engine and is run explicitly on a supported machine.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
#[ignore = "requires PHOTOCRAFT_UPSCALE_PACKAGE and a working Vulkan GPU"]
fn installed_upscale_package_previews_then_applies_a_tab_or_same_size_layer() {
    use egui_kittest::kittest::NodeT;
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Scratch(std::env::temp_dir().join(format!(
        "photocraft-upscale-preview-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let package = std::env::var("PHOTOCRAFT_UPSCALE_PACKAGE").expect("set PHOTOCRAFT_UPSCALE_PACKAGE to the rebuilt .pcplugin");
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.extensions.attach_folder(dir.0.join("Extensions")).unwrap();
    app.run("extension.install", json!({"path": package, "allowNative": true})).unwrap();
    app.run("file.new", json!({"width":32,"height":24,"depth":16,"background":"white"})).unwrap();
    let id = "org.photocraft.ai-upscale";
    assert!(app.session.extensions.installed.contains_key(id));
    crate::extension_ui::menu(&mut app, &format!("window.extension.{id}")).unwrap().unwrap();
    app.background_jobs = true;
    let mut h = panel_harness(egui::vec2(1200.0, 900.0), app);
    h.run_steps(3);
    for target in ["document", "layer", "discard", "stale"] {
        let source = h.state().session.active().unwrap().doc.clone();
        let documents = h.state().session.documents().len();
        let layer_count = source.layers.len();
        let panel = h.state_mut().ui.extensions.panels.get_mut(id).unwrap();
        // The package exposes 1× as enhancement; it retains the source canvas dimensions.
        panel.values["scale"] = json!("1× · Enhance only");
        h.run_steps(2);
        let button = h.get_by_label("Generate preview");
        let button_rect = button.rect();
        let button_enabled = !button.accesskit_node().is_disabled();
        assert!(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 900.0)).contains_rect(button_rect), "Generate preview must be visible: {button_rect:?}");
        assert!(button_enabled, "Generate preview must be enabled before processing starts");
        button.click();
        h.run_steps(2);
        let panel = &h.state().ui.extensions.panels[id];
        assert!(!panel.error, "{}", panel.message);
        let job = photocraft_engine::jobs::JobId(panel.job.unwrap_or_else(|| {
            panic!(
                "Generate preview did not start background work for {target}: button={button_rect:?}, enabled={button_enabled}, message={:?}, preview={:?}, status={:?}, jobs={:?}",
                panel.message, panel.preview, h.state().ui.status, h.state().session.jobs_with_recent()
            )
        }));
        h.state_mut().session.wait_job(job).unwrap();
        h.run_steps(3);
        let panel = &h.state().ui.extensions.panels[id];
        assert!(!panel.error, "{}", panel.message);
        let token = panel.preview.expect("the completed job must offer a preview");
        assert!(h.state().session.plugin_preview(token).is_some());
        assert_eq!(h.state().session.documents().len(), documents, "preview must not open a document");
        assert!(Arc::ptr_eq(&source, &h.state().session.active().unwrap().doc));
        match target {
            "stale" => {
                h.state_mut().session.execute("layer.new.layer", json!({"name":"Edit after preview"})).unwrap();
                h.run_steps(3);
                assert!(h.state().ui.extensions.panels[id].preview.is_none());
                assert!(h.state().session.plugin_preview(token).is_none());
                assert!(h.state().ui.extensions.panels[id].message.contains("changed"));
            }
            "discard" => {
                h.get_by_label("Discard preview").click();
                h.run_steps(3);
                assert!(h.state().ui.extensions.panels[id].open);
                assert!(h.state().ui.extensions.panels[id].preview.is_none());
                assert!(h.state().session.plugin_preview(token).is_none());
                assert_eq!(h.state().session.documents().len(), documents);
            }
            target => {
                if target == "layer" {
                    h.query_all_by_role(egui::accesskit::Role::ComboBox)
                        .find(|node| node.value().as_deref() == Some("New tab"))
                        .expect("the output dropdown displays New tab")
                        .click();
                    h.run_steps(2);
                    h.get_by_label("New layer").click();
                    h.run_steps(2);
                }
                h.get_by_label("Apply").click();
                h.run_steps(3);
                let panel = &h.state().ui.extensions.panels[id];
                assert!(!panel.error, "{}", panel.message);
                assert!(panel.preview.is_none());
                if target == "document" {
                    assert_eq!(h.state().session.documents().len(), documents + 1);
                    assert_eq!(h.state().session.active().unwrap().doc.size, source.size);
                    assert!(h.state().session.documents().iter().any(|doc| Arc::ptr_eq(&doc.doc, &source)));
                } else {
                    assert_eq!(h.state().session.documents().len(), documents);
                    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layer_count + 1);
                    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
                    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layer_count);
                }
            }
        }
    }
    h.get_by_label("Done").click();
    h.run_steps(2);
    assert!(!h.state().ui.extensions.panels[id].open);
}
