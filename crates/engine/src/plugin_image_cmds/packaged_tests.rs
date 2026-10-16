use super::*;

/// Regression for a 4240×2832 source rejected at 4× by the former 100 MP cap.
/// Uses the shipped software path to cover full-size transport/import without a long GPU job.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
#[ignore = "requires PHOTOCRAFT_UPSCALE_PACKAGE and several GiB of available RAM"]
fn large_photo_package_preview_and_apply_opens_full_size_result() {
    let package = std::env::var("PHOTOCRAFT_UPSCALE_PACKAGE").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    session.extensions.attach_folder(dir.path().join("Extensions")).unwrap();
    session.execute("extension.install", json!({"path":package,"allowNative":true})).unwrap();
    session.execute("file.new", json!({"width":4240,"height":2832,"depth":8,"background":"white"})).unwrap();
    let source = Arc::clone(&session.active().unwrap().doc);
    let started = session
        .start("extension.invoke", json!({"id":"org.photocraft.ai-upscale","event":"upscale","values":{"engine":"Conventional resize (Lanczos)","scale":"4×"}}))
        .unwrap();
    let crate::jobs::Started::Job(job) = started else { panic!("processing must run in the background") };
    let result = session.wait_job(job).unwrap();
    let token = result["preview"].as_u64().unwrap();
    assert_eq!(session.documents().len(), 1, "preview must not open the result before Apply");
    assert!(Arc::ptr_eq(&source, &session.active().unwrap().doc));
    let preview = session.plugin_preview(token).unwrap();
    assert_eq!((preview.result.size.width, preview.result.size.height), (16960, 11328));
    assert_eq!(preview.result.depth.bits(), 8);
    assert!(!preview.can_apply_as_layer(), "an enlarged image needs its own canvas");
    session.execute("image.pluginPreview.apply", json!({"preview":token,"target":"document"})).unwrap();
    assert_eq!(session.documents().len(), 2);
    assert!(Arc::ptr_eq(&source, &session.documents()[0].doc));
    let output = &session.active().unwrap().doc;
    assert_eq!((output.size.width, output.size.height), (16960, 11328));
    assert_eq!(output.resolution_dpi, source.resolution_dpi);
    assert!(session.plugin_preview(token).is_none());
}

/// Installs the complete independent package, discards its download and restarts the host
/// before processing. Run with PHOTOCRAFT_UPSCALE_PACKAGE; PHOTOCRAFT_UPSCALE_AI enables Vulkan.
#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "requires the built AI Upscale .pcplugin package"]
fn installed_plugin_package_roundtrip_and_cancellation() {
    let package = std::env::var("PHOTOCRAFT_UPSCALE_PACKAGE").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let download = dir.path().join("download.pcplugin");
    std::fs::copy(package, &download).unwrap();
    let folder = dir.path().join("Extensions");
    let mut installer = Session::new();
    installer.extensions.attach_folder(folder.clone()).unwrap();
    let review = installer.execute("extension.inspect", json!({"path":download})).unwrap();
    installer.execute("extension.install", json!({"path":download,"expectedHash":review["hash"],"allowNative":true})).unwrap();
    assert!(installer.extensions.installed["org.photocraft.ai-upscale"].native.as_ref().unwrap().worker.is_file());
    drop(installer);
    std::fs::remove_file(download).unwrap();
    let mut s = Session::new();
    s.extensions.attach_folder(folder).unwrap();
    assert!(s.extensions.load_errors.is_empty(), "{:?}", s.extensions.load_errors);
    let depth = std::env::var("PHOTOCRAFT_UPSCALE_DEPTH").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8);
    s.execute("file.new", json!({"width":32,"height":24,"background":"white","depth":depth})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Keep this layer"})).unwrap();
    let source = Arc::clone(&s.active().unwrap().doc);
    let ai = std::env::var_os("PHOTOCRAFT_UPSCALE_AI").is_some();
    let params = json!({"id":"org.photocraft.ai-upscale","event":"upscale","values":{
        "scale":"2×", "model":"Photo · Fast", "engine":if ai {"Local AI (Vulkan)"} else {"Conventional resize (Lanczos)"}
    }});
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params.clone()).unwrap() else { panic!("upscale must run in background") };
    let result = s.wait_job(job).unwrap();
    let preview = result["preview"].as_u64().unwrap();
    assert_eq!(s.documents().len(), 1);
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &source));
    assert!(s.extensions.installed["org.photocraft.ai-upscale"].state.is_null());
    assert_eq!(s.plugin_preview(preview).unwrap().result.size, photocraft_doc::Size::new(64, 48));
    let result = s.execute("image.pluginPreview.apply", json!({"preview":preview})).unwrap();
    assert!(result["message"].as_str().unwrap().contains("64 × 48"));
    assert_eq!(s.documents().len(), 2);
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &source));
    assert_eq!((s.active().unwrap().doc.size.width, s.active().unwrap().doc.size.height), (64, 48));
    assert_eq!(s.extensions.installed["org.photocraft.ai-upscale"].state["runs"], 1);
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params).unwrap() else { panic!("background job expected") };
    s.cancel_job(job);
    assert!(s.wait_job(job).is_err());
    s.join_cancelled_jobs();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.extensions.installed["org.photocraft.ai-upscale"].state["runs"], 1);
    let params = json!({"id":"org.photocraft.ai-upscale","event":"upscale","values":{"engine":"Conventional resize (Lanczos)"}});
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params).unwrap() else { panic!("background job expected") };
    Arc::make_mut(&mut s.active_mut().unwrap().doc).name = "Changed while worker ran".into();
    assert!(s.wait_job(job).unwrap_err().to_string().contains("changed while it ran"));
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.extensions.installed["org.photocraft.ai-upscale"].state["runs"], 1);
    let before_layer = Arc::clone(&s.active().unwrap().doc);
    let params = json!({"id":"org.photocraft.ai-upscale","event":"upscale","values":{
        "scale":"1× · Enhance only","model":"Photo · Fast", "engine":if ai {"Local AI (Vulkan)"} else {"Conventional resize (Lanczos)"}
    }});
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params).unwrap() else { panic!("background job expected") };
    let result = s.wait_job(job).unwrap();
    let preview = result["preview"].as_u64().unwrap();
    assert!(s.plugin_preview(preview).unwrap().can_apply_as_layer());
    assert!(Arc::ptr_eq(&before_layer, &s.active().unwrap().doc));
    assert_eq!(s.extensions.installed["org.photocraft.ai-upscale"].state["runs"], 1);
    s.execute("image.pluginPreview.apply", json!({"preview":preview,"target":"layer"})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active().unwrap().doc.layers.len(), before_layer.layers.len() + 1);
    assert_eq!(s.active().unwrap().doc.size, before_layer.size);
    assert_eq!(s.extensions.installed["org.photocraft.ai-upscale"].state["runs"], 2);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before_layer);
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.layers.len(), before_layer.layers.len() + 1);
}

/// Proves that the complete installed bundle survives download removal and restart. Run with
/// PHOTOCRAFT_BACKGROUND_PACKAGE. Optional MODEL, OUTPUT and DEPTH exercise each delivered model.
#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "requires the built Background Removal .pcplugin package and bundled model assets"]
fn installed_package_roundtrip_cancellation_and_source_safety() {
    let package = std::env::var("PHOTOCRAFT_BACKGROUND_PACKAGE").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let download = dir.path().join("download.pcplugin");
    std::fs::copy(package, &download).unwrap();
    let folder = dir.path().join("Extensions");
    let mut installer = Session::new();
    installer.extensions.attach_folder(folder.clone()).unwrap();
    let review = installer.execute("extension.inspect", json!({"path":download})).unwrap();
    assert_eq!(review["package"]["native"]["service"], "plugin.processImage");
    installer.execute("extension.install", json!({"path":download,"expectedHash":review["hash"],"allowNative":true})).unwrap();
    let id = review["manifest"]["id"].as_str().unwrap().to_owned();
    drop(installer);
    std::fs::remove_file(download).unwrap();
    let mut s = Session::new();
    s.extensions.attach_folder(folder).unwrap();
    assert!(s.extensions.load_errors.is_empty(), "{:?}", s.extensions.load_errors);
    let depth = std::env::var("PHOTOCRAFT_BACKGROUND_DEPTH").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(16);
    s.execute("file.new", json!({"width":32,"height":24,"background":"white","depth":depth})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Keep this layer"})).unwrap();
    let source = Arc::clone(&s.active().unwrap().doc);
    // Panel defaults and opaque package-owned model settings both use the shared processor API.
    let panel = &s.extensions.installed[&id].extension.manifest.panel;
    let event = panel.buttons.first().unwrap().id.clone();
    let params = json!({"id":id,"event":event,"values":{}});
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params.clone()).unwrap() else { panic!("background job expected") };
    assert!(s.execute("extension.remove", json!({"id":id})).is_err());
    assert!(s.execute("image.adjustments.invert", json!({})).is_err());
    let result = s.wait_job(job).unwrap();
    let preview = result["preview"].as_u64().unwrap();
    assert_eq!(s.documents().len(), 1);
    s.execute("image.pluginPreview.apply", json!({"preview":preview})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &source));
    assert_eq!((s.active().unwrap().doc.size.width, s.active().unwrap().doc.size.height), (32, 24));
    assert_eq!(s.active().unwrap().doc.depth.bits(), 8);
    assert_eq!(s.active().unwrap().doc.resolution_dpi, source.resolution_dpi);
    let completed_state = s.extensions.installed[&id].state.clone();
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params.clone()).unwrap() else { panic!("background job expected") };
    s.cancel_job(job);
    assert!(s.wait_job(job).is_err());
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.extensions.installed[&id].state, completed_state);
    let crate::jobs::Started::Job(job) = s.start("extension.invoke", params).unwrap() else { panic!("background job expected") };
    Arc::make_mut(&mut s.active_mut().unwrap().doc).name = "Changed while worker ran".into();
    assert!(s.wait_job(job).unwrap_err().to_string().contains("changed while it ran"));
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.extensions.installed[&id].state, completed_state);
    let model = std::env::var("PHOTOCRAFT_BACKGROUND_MODEL").unwrap_or_else(|_| "isnet_quint8".into());
    let output = std::env::var("PHOTOCRAFT_BACKGROUND_OUTPUT").unwrap_or_else(|_| "mask".into());
    let crate::jobs::Started::Job(job) = s.start("plugin.processImage", json!({"id":id,"options":{"model":model,"output":output},"preview":false})).unwrap()
    else {
        panic!("background job expected")
    };
    s.wait_job(job).unwrap();
    assert_eq!(s.documents().len(), 3);
    assert_eq!(s.extensions.installed[&id].state, completed_state);
}
