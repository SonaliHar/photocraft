//! Duplicated typed XCF state must suggest a lossless native destination without source grants.
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services};
use serde_json::json;
use std::sync::{Arc, Mutex};
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn duplicate_save(mut doc: photocraft_doc::Document, command: &str) -> TestResult {
    doc.name = "typed-source.xcf".into();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let delivered = writes.clone();
    let services = Services {
        file_dialog: Some(Box::new(move |request, _, reply| {
            captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request.clone());
            let answer = match request {
                FileDialogRequest::Save { suggested } => Some(FileDialogAnswer::SaveTo(suggested)),
                _ => None,
            };
            reply.send(answer);
        })),
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |path, bytes| {
            delivered.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, Some("typed-source.pcraft".into()))?;
    session.edit("Own source redo seed", |doc, _| {
        doc.resolution_dpi += 1.0;
        Ok(())
    })?;
    assert!(session.undo());
    assert!(session.active().ok_or("missing source")?.history.can_redo());
    let mut app = PhotocraftApp::new(session, services);
    assert!(!app.background_jobs);
    let source = app.session.active().ok_or("missing source")?;
    let source_doc = source.doc.clone();
    let source_identity = source.source.clone();
    let source_revision = source.revision;
    let source_saved = source.saved_revision;
    let source_past: Vec<_> = source.history.entry_labels().map(str::to_owned).collect();
    let source_redo: Vec<_> = source.history.redo_labels().map(str::to_owned).collect();
    let result = app.run("image.duplicate", json!({}))?;
    assert_ne!(result.get("pending").and_then(serde_json::Value::as_bool), Some(true));
    assert_eq!(app.session.documents().len(), 2);
    let before = app.session.active().ok_or("missing duplicate")?.doc.clone();
    assert!(before.xcf_state.is_some());
    assert!(app.session.active().ok_or("missing duplicate")?.source.is_none());
    assert!(app.session.active().ok_or("missing duplicate")?.path.is_none());
    let ctx = egui::Context::default();
    photocraft_ui_egui::menus::invoke(&mut app, &ctx, command, json!({}))?;
    app.poll_file_dialog(&egui::Context::default(), None);
    let requests = requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests.first(), Some(&FileDialogRequest::Save { suggested: "typed-source.xcf copy.pcraft".into() }));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let writes = writes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(writes.len(), 1);
    let (name, bytes) = writes.first().ok_or("missing native delivery")?;
    assert_eq!(name, "typed-source.xcf copy.pcraft");
    let reopened = photocraft_format::load_from_bytes(bytes)?;
    let mut named = (*before).clone();
    named.name = name.clone();
    assert_eq!(reopened, named);
    assert_eq!(&*app.session.active().ok_or("missing duplicate")?.doc, &named);
    let current = app.session.active().ok_or("missing duplicate")?;
    assert_eq!(current.path.as_deref(), Some("typed-source.xcf copy.pcraft"));
    assert_eq!(current.saved_revision, current.revision);
    let original = app.session.documents().first().ok_or("missing source")?;
    assert_eq!(original.doc, source_doc);
    assert_eq!(original.source, source_identity);
    assert_eq!(original.revision, source_revision);
    assert_eq!(original.saved_revision, source_saved);
    assert_eq!(original.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), source_past);
    assert_eq!(original.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), source_redo);
    assert_eq!(original.path.as_deref(), Some("typed-source.pcraft"));
    assert_ne!(current.path, original.path);
    Ok(())
}

#[test]
fn duplicate_typed_state_save_and_save_as_keep_native_format() -> TestResult {
    for command in ["file.save", "file.saveAs"] {
        let mut doc =
            photocraft_doc::Document::new("typed", photocraft_geom::Size::new(4, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        doc.xcf_state = Some(Default::default());
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        duplicate_save(doc, command)?;
    }
    Ok(())
}

#[test]
#[ignore = "requires the hash-bound actual browser XCF fixture; run explicitly with PHOTOCRAFT_DUPLICATE_SAVE_XCF"]
fn actual_live_filter_duplicate_save_and_save_as_keep_native_format() -> TestResult {
    let document = pinned_live_document()?;
    for command in ["file.save", "file.saveAs"] {
        duplicate_save(document.clone(), command)?;
    }
    Ok(())
}

fn pinned_live_document() -> Result<photocraft_doc::Document, Box<dyn std::error::Error>> {
    let path = std::env::var("PHOTOCRAFT_DUPLICATE_SAVE_XCF")?;
    let path = std::path::PathBuf::from(path);
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)?;
        if metadata.file_type().is_symlink() {
            return Err("fixture symlink refused".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("fixture reparse point refused".into());
            }
        }
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() != 4239 {
        return Err("fixture exact length mismatch".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(4240)?;
    std::fs::File::open(&path)?.take(4240).read_to_end(&mut bytes)?;
    if bytes.len() != 4239 {
        return Err("fixture changed or exceeds cap".into());
    }
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(&bytes));
    if hash != "2223d32264557606ae4d85dae0b605e9eb5ef406e7ee3bd7c9ed99b8fd0a6535" {
        return Err("fixture SHA256 mismatch".into());
    }
    let imported = photocraft_io::import("negative-owner.source.xcf", &bytes)?;
    assert!(imported.document.xcf_state.as_ref().is_some_and(|s| s.filter_stacks.iter().any(|stack| stack.nodes.iter().any(|node| node.enabled))));
    Ok(imported.document)
}

#[test]
#[ignore = "requires the pinned actual browser live-filter fixture"]
fn explicit_psd_refusal_preserves_duplicate_and_never_delivers() -> TestResult {
    let writes = Arc::new(Mutex::new(0usize));
    let captured = writes.clone();
    let services = Services {
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |_, _| {
            *captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            Ok(())
        })),
        ..Default::default()
    };
    let mut session = photocraft_engine::Session::new();
    session.add_document(pinned_live_document()?, None)?;
    let mut app = PhotocraftApp::new(session, services);
    app.run("image.duplicate", json!({}))?;
    let before = app.session.active().ok_or("missing duplicate")?;
    let native = photocraft_format::save_to_bytes(&before.doc, &Default::default())?;
    let revision = before.revision;
    let saved = before.saved_revision;
    let past: Vec<_> = before.history.entry_labels().map(str::to_owned).collect();
    let redo: Vec<_> = before.history.redo_labels().map(str::to_owned).collect();
    for path in ["explicit.psd", "explicit.xcf"] {
        let error = app.save_as(Some(path.into())).expect_err("unsupported explicit destination must refuse");
        assert!(
            if path.ends_with(".xcf") {
                error.contains("XCF is an import-only format")
            } else {
                error.contains("Layered export cannot yet preserve live GIMP filters")
            },
            "{error}"
        );
        let after = app.session.active().ok_or("missing duplicate")?;
        assert_eq!(photocraft_format::save_to_bytes(&after.doc, &Default::default())?, native);
        assert_eq!(after.revision, revision);
        assert_eq!(after.saved_revision, saved);
        assert!(after.path.is_none() && after.source.is_none());
        assert_eq!(after.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), past);
        assert_eq!(after.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
    }
    assert_eq!(*writes.lock().unwrap_or_else(std::sync::PoisonError::into_inner), 0);
    Ok(())
}

#[test]
fn supplied_multidot_duplicate_display_name_is_preserved_completely() -> TestResult {
    for name in ["Project.v2 title", "Project.pcraft", "Unicode .α release", ".hidden title"] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |request, _, reply| {
                captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request);
                reply.send(None);
            })),
            ..Default::default()
        };
        let mut doc =
            photocraft_doc::Document::new("source", photocraft_geom::Size::new(4, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        doc.xcf_state = Some(Default::default());
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut session = photocraft_engine::Session::new();
        session.add_document(doc, Some("source.pcraft".into()))?;
        let mut app = PhotocraftApp::new(session, services);
        app.run("image.duplicate", json!({"name": name}))?;
        app.save_as(None)?;
        app.poll_file_dialog(&egui::Context::default(), None);
        let requests = requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(
            requests.first(),
            Some(&FileDialogRequest::Save { suggested: if name.ends_with(".pcraft") { name.to_string() } else { format!("{name}.pcraft") } })
        );
        assert!(app.session.active().ok_or("missing duplicate")?.path.is_none());
        assert_eq!(app.session.documents().first().ok_or("missing source")?.path.as_deref(), Some("source.pcraft"));
    }
    Ok(())
}

#[test]
fn native_memory_reopen_preserves_copy_multidot_and_native_suffix() -> TestResult {
    for (title, suggested, expected, lowercase_extension) in [
        ("typed-source.xcf copy", "typed-source.xcf copy.pcraft", "typed-source.xcf copy.pcraft", true),
        ("Project.v2 title", "Project.v2 title.pcraft", "Project.v2 title.pcraft", true),
        ("Already.pcraft", "Already.pcraft", "Already.pcraft", true),
        ("Upper.PCRAFT", "Upper.PCRAFT", "Upper.PCRAFT", false),
        ("Upper.PCRAFT", "Upper.PCRAFT", "Upper.pcraft", true),
    ] {
        for command in ["file.save", "file.saveAs"] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let writes = Arc::new(Mutex::new(Vec::new()));
            let captured = requests.clone();
            let delivered = writes.clone();
            let services = Services {
                import: Some(Box::new(|name, bytes, depth| photocraft_io::import_with_svg_group_depth(name, bytes, depth).map_err(|e| e.to_string()))),
                export: Some(Box::new(|doc, path, _| {
                    photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
                })),
                file_dialog: Some(Box::new(move |request, _, reply| {
                    captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request.clone());
                    reply.send(match request {
                        FileDialogRequest::Save { suggested } => Some(FileDialogAnswer::SaveTo(suggested)),
                        _ => None,
                    });
                })),
                write: Some(Box::new(move |path, bytes| {
                    delivered.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((path.to_string(), bytes.to_vec()));
                    Ok(())
                })),
                ..Default::default()
            };
            let mut doc =
                photocraft_doc::Document::new(title, photocraft_geom::Size::new(4, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
            doc.xcf_state = Some(Default::default());
            doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
            doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
            let input = photocraft_format::save_to_bytes(&doc, &Default::default())?;
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            // Exercise the real default-on policy and explicit case-preserving opt-out.
            assert!(app.session.prefs().file_handling.lowercase_extension);
            if !lowercase_extension {
                app.run("prefs.set", json!({"path":"fileHandling.lowercaseExtension", "value":false}))?;
            }
            assert_eq!(app.session.prefs().file_handling.lowercase_extension, lowercase_extension);
            app.open_bytes("saved-source.pcraft", &input)?;
            let before = app.session.active().ok_or("missing reopened native")?.doc.clone();
            let source = app.session.active().ok_or("missing reopened native")?.source.clone();
            assert_eq!(before.name, title);
            assert!(source.as_ref().is_some_and(|source| matches!(source.origin.format, photocraft_io::ImportFormat::Pcraft)));
            assert!(app.session.active().ok_or("missing reopened native")?.path.is_none());
            let ctx = egui::Context::default();
            photocraft_ui_egui::menus::invoke(&mut app, &ctx, command, json!({}))?;
            app.poll_file_dialog(&ctx, None);
            let requests = requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(requests.first(), Some(&FileDialogRequest::Save { suggested: suggested.into() }));
            assert!(!app.ui.status_error, "{}", app.ui.status);
            let writes = writes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(writes.len(), 1);
            let (name, bytes) = writes.first().ok_or("missing native delivery")?;
            assert_eq!(name, expected);
            let mut named = (*before).clone();
            named.name = expected.into();
            assert_eq!(photocraft_format::load_from_bytes(bytes)?, named);
            let after = app.session.active().ok_or("missing reopened native")?;
            assert_eq!(&*after.doc, &named);
            assert_eq!(after.source, source);
            assert_eq!(after.path.as_deref(), Some(expected));
            assert_eq!(after.saved_revision, after.revision);
        }
    }
    Ok(())
}
