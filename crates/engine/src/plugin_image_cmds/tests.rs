use super::*;
use crate::extension_cmds::Installed;
use photocraft_plugins::extension::Extension;

const ID: &str = "org.example.generic-image";
const FILTER: &str = "org.photocraft.example.invert";
const INVERT: &[u8] = include_bytes!("../../../plugins/tests/fixtures/invert.wasm");

fn extension() -> Arc<Extension> {
    let manifest = json!({"apiVersion":1,"id":ID,"name":"General image fixture","version":"1.0.0",
        "permissions":{"commands":["plugin.processImage"]},"panel":{"buttons":[{"id":"process","label":"Process"}]}})
    .to_string();
    let response = json!({"commands":[{"command":"plugin.processImage","params":{"options":{}}}],"state":{"runs":1}}).to_string();
    let quoted = |s: &str| s.bytes().map(|b| format!("\\{b:02x}")).collect::<String>();
    let bytes = wat::parse_str(format!(
        r#"(module
        (memory (export "memory") 12 512)
        (data (i32.const 32) "{}") (data (i32.const 65536) "{}")
        (func (export "pce_api_version") (result i32) i32.const 1)
        (func (export "pce_manifest") (result i64) i64.const {})
        (func (export "pce_alloc") (param i32) (result i32) i32.const 393216)
        (func (export "pce_handle") (param i32 i32) (result i64) i64.const {})
    )"#,
        quoted(&manifest),
        quoted(&response),
        ((manifest.len() as u64) << 32) | 32,
        ((response.len() as u64) << 32) | 65536
    ))
    .unwrap();
    Arc::new(Extension::load(&bytes).unwrap())
}

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":6,"depth":depth,"background":"white"})).unwrap();
    let filter = Arc::new(Plugin::load(INVERT, Default::default()).unwrap());
    s.extensions.installed.insert(
        ID.into(),
        Installed {
            extension: extension(),
            enabled: true,
            state: json!({"runs":0}),
            storage: Default::default(),
            filters: [(FILTER.into(), filter)].into_iter().collect(),
            #[cfg(not(target_arch = "wasm32"))]
            package: None,
            #[cfg(not(target_arch = "wasm32"))]
            native: None,
        },
    );
    s
}

fn completion(s: &Session) -> Completion {
    (ID.into(), Arc::clone(&s.extensions.installed[ID].extension), json!({"runs":1}))
}

#[test]
fn settings_reject_unbounded_options_unknown_keys_and_non_objects() {
    for p in [
        json!(null),
        json!([]),
        json!(1),
        json!("settings"),
        json!({"input":"secret.png"}),
        json!({"path":"worker.exe"}),
        json!({"options":null}),
        json!({"options":[]}),
        json!({"width":-1}),
        json!({"height":4294967296u64}),
    ] {
        assert!(params(&p).is_err(), "{p}");
    }
    assert!(params(&json!({"options":{"payload":"x".repeat(MAX_OPTIONS_BYTES)}})).is_err());
    assert!(params(&json!({"options":{"payload":"x".repeat(MAX_OPTIONS_BYTES-32)}})).is_ok());
    assert!(params(&json!({})).unwrap().preview);
}

#[test]
fn image_bounds_cover_source_target_and_working_memory() {
    assert_eq!(dimensions((8, 6), (None, None)).unwrap(), (8, 6));
    assert_eq!(dimensions((8, 6), (Some(16), Some(12))).unwrap(), (16, 12));
    for source in [(0, 6), (8, 0), (32769, 1), (10001, 10000), (u32::MAX, u32::MAX)] {
        assert!(dimensions(source, (None, None)).is_err());
    }
    assert!(dimensions((8, 6), (Some(0), None)).is_err());
    assert!(dimensions((8, 6), (Some(32769), None)).is_err());
    if cfg!(target_pointer_width = "64") {
        assert_eq!(dimensions((4240, 2832), (Some(16960), Some(11328))).unwrap(), (16960, 11328));
        assert_eq!(dimensions((5000, 3000), (Some(20000), Some(12000))).unwrap(), (20000, 12000));
        assert!(dimensions((10000, 10000), (Some(16000), Some(16000))).unwrap_err().to_string().contains("8 GiB"));
    } else {
        assert!(dimensions((8000, 6000), (None, None)).unwrap_err().to_string().contains("2 GiB"));
    }
    assert!(dimensions((8, 6), (Some(16001), Some(16000))).unwrap_err().to_string().contains("256 megapixels"));
}

#[test]
fn only_the_invoking_installation_can_supply_processors() {
    let mut s = session(8);
    let own = completion(&s);
    assert!(run(&mut s, &json!({"id":"another.plugin"}), Some(own)).unwrap_err().to_string().contains("own installation"));
    let own = completion(&s);
    assert!(run(&mut s, &json!({"processor":"another.filter"}), Some(own)).unwrap_err().to_string().contains("does not belong"));
    let own = completion(&s);
    assert!(run(&mut s, &json!({"processor":"native"}), Some(own)).unwrap_err().to_string().contains("native"));
    assert!(run(&mut s, &json!({}), None).unwrap_err().to_string().contains("`id`"));
    let mut stale = completion(&s);
    stale.1 = extension();
    assert!(run(&mut s, &json!({}), Some(stale)).unwrap_err().to_string().contains("replaced"));
    s.extensions.installed.get_mut(ID).unwrap().enabled = false;
    let own = completion(&s);
    assert!(run(&mut s, &json!({}), Some(own)).is_err());
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn unsupported_source_and_filter_resizing_do_not_change_the_document() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        let mut s = session(8);
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        doc.mode = mode;
        if mode == ColorMode::Rgb {
            doc.depth = SampleType::F32;
        }
        let source = Arc::clone(&s.active().unwrap().doc);
        assert!(run(&mut s, &json!({"id":ID}), None).unwrap_err().to_string().contains("8/16-bit"));
        assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
    }
    let mut s = session(8);
    let source = Arc::clone(&s.active().unwrap().doc);
    assert!(run(&mut s, &json!({"id":ID,"width":16}), None).unwrap_err().to_string().contains("dimensions"));
    assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
}

#[test]
fn packaged_filter_previews_then_applies_undoable_layer_at_source_depth() {
    for depth in [8, 16] {
        let mut s = session(depth);
        let source = Arc::clone(&s.active().unwrap().doc);
        let own = completion(&s);
        let result = run(&mut s, &json!({}), Some(own)).unwrap();
        let token = result["preview"].as_u64().unwrap();
        assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.extensions.installed[ID].state["runs"], 0);
        let preview = s.plugin_preview(token).unwrap();
        assert!(preview.can_apply_as_layer());
        assert_eq!(preview.result.depth, SampleType::U8);
        assert_eq!(preview.result.mode, ColorMode::Rgb);
        assert_eq!(preview.result.icc_profile, Some(Builtin::Srgb.profile().to_bytes()));
        let px = preview.result.layers[0].surface().unwrap().pixel(0, 0);
        assert!(px[0] < 0.01 && px[1] < 0.01 && px[2] < 0.01 && px[3] > 0.99);
        s.execute("image.pluginPreview.apply", json!({"preview":token,"target":"layer"})).unwrap();
        assert_eq!(s.active().unwrap().doc.depth, source.depth);
        assert_eq!(s.active().unwrap().doc.layers.len(), source.layers.len() + 1);
        assert_eq!(s.extensions.installed[ID].state["runs"], 1);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, source);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.layers.len(), source.layers.len() + 1);
    }
}

#[test]
fn generic_processing_uses_visible_composite_and_honors_filter_selection_alpha() {
    let mut s = session(16);
    s.edit("Fixture", |doc, _| {
        doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
        let mut visible = Layer::raster("Visible", doc.pixel_format());
        visible.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.2, 0.4, 0.6, 0.5]);
        let mut hidden = Layer::raster("Hidden", doc.pixel_format());
        hidden.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.9, 0.1, 0.2, 1.0]);
        hidden.visible = false;
        doc.layers = vec![visible, hidden];
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(photocraft_geom::Rect::new(0, 0, 4, 6), &[1.0]);
        doc.selection = Some(selection);
        Ok(())
    })
    .unwrap();
    let result = run(&mut s, &json!({"id":ID}), None).unwrap();
    let preview = s.plugin_preview(result["preview"].as_u64().unwrap()).unwrap();
    let surface = preview.result.layers[0].surface().unwrap();
    let selected = surface.pixel(1, 1);
    let outside = surface.pixel(6, 1);
    assert!((selected[0] - 0.8).abs() < 0.01 && (selected[1] - 0.6).abs() < 0.01);
    assert!((outside[0] - 0.2).abs() < 0.01 && (outside[1] - 0.4).abs() < 0.01);
    assert!((selected[3] - 0.5).abs() < 0.01 && (outside[3] - 0.5).abs() < 0.01);
    assert_eq!(s.extensions.installed[ID].state["runs"], 0);
}

#[test]
fn flatten_converts_profile_before_quantization_and_supports_gray() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let mut doc = Document::new("Profile fixture", photocraft_doc::Size::new(3, 2), mode, SampleType::U16);
        doc.icc_profile = Some(if mode == ColorMode::Rgb { Builtin::DisplayP3.profile().to_bytes() } else { Builtin::SGray.profile().to_bytes() });
        let mut layer = Layer::raster("Color", doc.pixel_format());
        let color = if mode == ColorMode::Rgb { vec![0.7, 0.3, 0.2, 0.4] } else { vec![0.3, 0.4] };
        layer.surface_mut().unwrap().fill_rect(doc.bounds(), &color);
        doc.layers.push(layer);
        let mut expected = if mode == ColorMode::Rgb { vec![0.7, 0.3, 0.2, 0.4] } else { vec![0.3, 0.3, 0.3, 0.4] };
        Transform::new(&crate::color_cmds::composite_profile(&doc), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true)
            .unwrap()
            .apply(&mut expected, 4);
        let result = flatten_srgb(&doc, &JobCtx::new()).unwrap();
        let actual = result.layers[0].surface().unwrap().pixel(0, 0);
        for (a, b) in actual.iter().zip(expected) {
            assert!((a - b).abs() < 0.01, "{mode:?}: {actual:?}");
        }
        assert_eq!(result.mode, ColorMode::Rgb);
        assert_eq!(result.depth, SampleType::U8);
    }
}

#[test]
fn cancelled_filter_job_and_discard_leave_document_and_guest_state_unchanged() {
    let mut s = session(8);
    let source = Arc::clone(&s.active().unwrap().doc);
    let own = completion(&s);
    let token = run(&mut s, &json!({}), Some(own)).unwrap()["preview"].as_u64().unwrap();
    s.execute("image.pluginPreview.discard", json!({"preview":token})).unwrap();
    assert!(s.plugin_preview(token).is_none());
    assert_eq!(s.extensions.installed[ID].state["runs"], 0);
    let ctx = JobCtx::new();
    ctx.cancel();
    s.set_inline_job_ctx(Some(ctx));
    let own = completion(&s);
    assert!(matches!(run(&mut s, &json!({}), Some(own)), Err(EngineError::Cancelled)));
    s.set_inline_job_ctx(None);
    assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
    assert_eq!(s.extensions.installed[ID].state["runs"], 0);
}

#[test]
fn direct_processing_opens_result_without_overwriting_guest_state() {
    let mut s = session(8);
    let source = Arc::clone(&s.active().unwrap().doc);
    let result = run(&mut s, &json!({"id":ID,"preview":false}), None).unwrap();
    assert_eq!(result["document"], 1);
    assert_eq!(s.documents().len(), 2);
    assert!(Arc::ptr_eq(&source, &s.documents()[0].doc));
    assert_eq!(s.extensions.installed[ID].state["runs"], 0);
}

#[test]
fn extension_invoke_reaches_its_packaged_filter_without_global_registry() {
    let mut s = session(8);
    let result = s.execute("extension.invoke", json!({"id":ID,"event":"process"})).unwrap();
    let token = result["preview"].as_u64().unwrap();
    assert_eq!(s.extensions.installed[ID].state["runs"], 0);
    s.execute("image.pluginPreview.apply", json!({"preview":token})).unwrap();
    assert_eq!(s.extensions.installed[ID].state["runs"], 1);
    assert_eq!(s.documents().len(), 2);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn source_changes_and_plugin_disable_reject_background_results_and_preview_apply() {
    let mut s = session(8);
    let started = s.start("plugin.processImage", json!({"id":ID})).unwrap();
    let crate::jobs::Started::Job(job) = started else { panic!("expected a background job") };
    Arc::make_mut(&mut s.active_mut().unwrap().doc).name = "Changed".into();
    assert!(s.wait_job(job).unwrap_err().to_string().contains("changed"));
    let own = completion(&s);
    let token = run(&mut s, &json!({}), Some(own)).unwrap()["preview"].as_u64().unwrap();
    s.execute("extension.enable", json!({"id":ID,"enabled":false})).unwrap();
    assert!(s.execute("image.pluginPreview.apply", json!({"preview":token})).is_err());
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.extensions.installed[ID].state["runs"], 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn disabling_then_reenabling_revokes_preview_and_immediate_jobs_and_leases_block_uninstall() {
    for preview in [true, false] {
        let mut s = session(8);
        let source = Arc::clone(&s.active().unwrap().doc);
        let started = s.start("plugin.processImage", json!({"id":ID,"preview":preview})).unwrap();
        let crate::jobs::Started::Job(job) = started else { panic!("expected a background job") };
        assert!(s.execute("extension.remove", json!({"id":ID})).unwrap_err().to_string().contains("processing"));
        s.execute("extension.enable", json!({"id":ID,"enabled":false})).unwrap();
        s.execute("extension.enable", json!({"id":ID,"enabled":true})).unwrap();
        assert!(s.wait_job(job).unwrap_err().to_string().contains("discarded or replaced"));
        assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.extensions.installed[ID].state["runs"], 0);
    }
}

/// Independent SDK package proof: no matching id, service-specific options or worker path
/// appears in the host. Restart loads only the installed generation, after the download is gone.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires PHOTOCRAFT_GENERAL_PACKAGE built from the independent native starter"]
fn independent_native_package_restarts_processes_resizes_and_rejects_invalid_options() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("download.pcplugin");
    std::fs::copy(std::env::var("PHOTOCRAFT_GENERAL_PACKAGE").unwrap(), &archive).unwrap();
    let folder = dir.path().join("Extensions");
    let mut installer = Session::new();
    installer.extensions.attach_folder(folder.clone()).unwrap();
    let result = installer.execute("extension.install", json!({"path":archive,"allowNative":true})).unwrap();
    let id = result["manifest"]["id"].as_str().unwrap().to_string();
    drop(installer);
    std::fs::remove_file(archive).unwrap();
    let mut s = Session::new();
    s.extensions.attach_folder(folder).unwrap();
    assert!(s.extensions.load_errors.is_empty(), "{:?}", s.extensions.load_errors);
    s.execute("file.new", json!({"width":8,"height":6,"depth":16,"background":"white"})).unwrap();
    s.edit("Native color fixture", |doc, _| {
        doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
        let canvas = doc.bounds();
        doc.layers.first_mut().unwrap().surface_mut().unwrap().fill_rect(canvas, &[0.2, 0.4, 0.6, 0.5]);
        Ok(())
    })
    .unwrap();
    let source = Arc::clone(&s.active().unwrap().doc);
    let worker = s.extensions.installed[&id].native.as_ref().unwrap().worker.clone();
    assert!(worker.is_file());
    assert!(worker.starts_with(std::fs::canonicalize(dir.path()).unwrap()));
    let response = s.execute("plugin.processImage", json!({"id":id,"options":{"gain":1.5}})).unwrap();
    let token = response["preview"].as_u64().unwrap();
    let preview = s.plugin_preview(token).unwrap();
    assert_eq!(preview.result.size, source.size);
    assert_eq!(preview.result.depth, SampleType::U8);
    assert_eq!(preview.result.mode, ColorMode::Rgb);
    assert_eq!(preview.result.icc_profile, Some(Builtin::Srgb.profile().to_bytes()));
    let actual = preview.result.layers[0].surface().unwrap().pixel(1, 1);
    for (a, b) in actual.iter().zip([0.3, 0.6, 0.9, 0.5]) {
        assert!((a - b).abs() < 0.01, "{actual:?}");
    }
    assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
    s.execute("image.pluginPreview.discard", json!({"preview":token})).unwrap();
    let state = s.extensions.installed[&id].state.clone();
    assert!(s.execute("plugin.processImage", json!({"id":id,"options":{"gain":-1.0}})).is_err());
    assert_eq!(s.extensions.installed[&id].state, state);
    assert!(Arc::ptr_eq(&source, &s.active().unwrap().doc));
    let response = s.execute("plugin.processImage", json!({"id":id,"width":16,"height":12,"options":{"gain":1.0}})).unwrap();
    let token = response["preview"].as_u64().unwrap();
    assert_eq!(s.plugin_preview(token).unwrap().result.size, photocraft_doc::Size::new(16, 12));
    assert!(!s.plugin_preview(token).unwrap().can_apply_as_layer());
    s.execute("image.pluginPreview.apply", json!({"preview":token})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert!(Arc::ptr_eq(&source, &s.documents()[0].doc));
}
