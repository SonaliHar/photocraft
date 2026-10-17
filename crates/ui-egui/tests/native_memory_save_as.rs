//! Save As retains the format of a native document opened from memory.
use std::sync::{Arc, Mutex};

use photocraft_io::{ImportFormat, ImportOrigin};
use photocraft_ui_egui::{FileDialogRequest, PhotocraftApp, Services};

fn suggestion(origin: ImportOrigin, path: Option<&str>) -> Result<String, String> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let services = Services {
        import: Some(Box::new(move |_, _, _| {
            Ok(photocraft_io::ImportResult {
                document: photocraft_doc::Document::new(
                    "internal-source.xcf",
                    photocraft_geom::Size::new(4, 4),
                    photocraft_color::ColorMode::Rgb,
                    photocraft_color::SampleType::U8,
                ),
                warnings: Vec::new(),
                origin,
                assessment: None,
                source_read_only: origin.is_read_only(),
                preview_only: false,
            })
        })),
        file_dialog: Some(Box::new(move |request, _, reply| {
            captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(request);
            reply.send(None);
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.open_bytes("browser-rgb-edited.pcraft", b"synthetic importer bytes")?;
    let state = app.session.active_mut().ok_or("missing imported document")?;
    if state.doc.name != "internal-source.xcf" || state.path.is_some() {
        return Err("memory import did not preserve the internal name and absent path".into());
    }
    if let Some(path) = path {
        state.set_import_origin(origin, "source", Some(path.into()));
    }
    app.save_as(None)?;
    app.poll_file_dialog(&egui::Context::default(), None);
    let requests = requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if requests.len() != 1 {
        return Err("expected exactly one save request".into());
    }
    match requests.first() {
        Some(FileDialogRequest::Save { suggested }) => Ok(suggested.clone()),
        _ => Err("expected a save dialog".into()),
    }
}

#[test]
fn native_memory_contents_keep_native_suffix_despite_internal_xcf_name() {
    assert_eq!(suggestion(ImportOrigin::writable(ImportFormat::Pcraft), None).expect("native save suggestion"), "internal-source.pcraft");
}

#[test]
fn psd_and_flat_memory_contents_keep_existing_psd_default() {
    for format in [ImportFormat::Psd, ImportFormat::Flat(photocraft_codecs::Format::Png)] {
        assert_eq!(suggestion(ImportOrigin::writable(format), None).expect("existing save suggestion"), "internal-source.psd");
    }
}

#[test]
fn writable_destinations_and_readonly_xcf_suggestions_remain_unchanged() {
    for (format, path) in [
        (ImportFormat::Pcraft, "/pics/project.pcraft"),
        (ImportFormat::Psd, "/pics/source.psd"),
        (ImportFormat::Flat(photocraft_codecs::Format::Png), "/pics/picture.png"),
    ] {
        assert_eq!(suggestion(ImportOrigin::writable(format), Some(path)).expect("existing writable path"), path);
    }
    assert_eq!(suggestion(ImportOrigin::xcf(), None).expect("protected XCF save suggestion"), "internal-source.pcraft");
}
