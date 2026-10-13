//! Import-time notices only: never a persisted claim about later edits.
use photocraft_io::xcf_assessment::{Fidelity, ImportAssessment};
use serde_json::Value;

fn push(warnings: &mut Vec<String>, message: &str) {
    if !warnings.iter().any(|warning| warning == message) {
        warnings.push(message.to_owned());
    }
}

fn fidelity(warnings: &mut Vec<String>, value: &str) {
    match value {
        "reduced" => push(warnings, "This XCF import has reduced fidelity."),
        "dependency_limited" => push(warnings, "This XCF import has fidelity limited by unavailable or inactive dependencies."),
        _ => {}
    }
}

pub(crate) fn append(warnings: &mut Vec<String>, assessment: Option<&ImportAssessment>) {
    let Some(assessment) = assessment else { return };
    fidelity(
        warnings,
        match assessment.fidelity {
            Fidelity::Full => "full",
            Fidelity::Reduced => "reduced",
            Fidelity::DependencyLimited => "dependency_limited",
        },
    );
    for diagnostic in &assessment.diagnostics {
        push(warnings, diagnostic.message);
    }
}

/// Background job reports are trusted engine output. Only known diagnostic
/// codes are displayed, so resource locators and payloads never become notices.
pub(crate) fn append_json(warnings: &mut Vec<String>, assessment: Option<&Value>) {
    let Some(assessment) = assessment else { return };
    fidelity(warnings, assessment.get("fidelity").and_then(Value::as_str).unwrap_or_default());
    if let Some(diagnostics) = assessment.get("diagnostics").and_then(Value::as_array) {
        for diagnostic in diagnostics.iter().take(1000) {
            let message = match diagnostic.get("code").and_then(Value::as_str) {
                Some("missing_resource") => "A retained dependency is unavailable; its stored locator grants no read permission.",
                Some("resource_not_activated") => "Supplied bytes have not established an exact active resource consumer.",
                Some("substituted_resource") => "An explicitly substituted dependency lowers this project's fidelity.",
                _ => continue,
            };
            push(warnings, message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn import_notices_report_limits_without_storing_full_or_resource_authority() {
        let mut warnings = Vec::new();
        append_json(&mut warnings, Some(&serde_json::json!({"fidelity":"full"})));
        assert!(warnings.is_empty());
        let report = serde_json::json!({"fidelity":"dependency_limited", "diagnostics":[
            {"code":"missing_resource", "message":"private path", "locator":"private path"},
            {"code":"unknown", "message":"private path"}]});
        append_json(&mut warnings, Some(&report));
        append_json(&mut warnings, Some(&report));
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|warning| !warning.contains("private path")));
        assert!(warnings[0].contains("fidelity limited"));
    }

    #[test]
    fn synchronous_and_automation_opens_publish_material_import_notices() {
        for automation in [false, true] {
            let services = crate::Services {
                import: Some(Box::new(|name, _, _depth| {
                    Ok(photocraft_io::ImportResult { source_read_only: false, preview_only: false,
                        document: photocraft_doc::Document::new(
                            name,
                            photocraft_geom::Size::new(4, 4),
                            photocraft_color::ColorMode::Rgb,
                            photocraft_color::SampleType::U8,
                        ),
                        warnings: Vec::new(),
                        origin: Default::default(),
                        assessment: Some(ImportAssessment {
                            fidelity: Fidelity::Reduced,
                            source_format: "xcf",
                            source_version: 22,
                            container: "xcf".into(),
                            dependencies: Vec::new(),
                            diagnostics: vec![photocraft_io::xcf_assessment::SuccessDiagnostic {
                                code: "substituted_resource",
                                severity: "warning",
                                resource_id: "1".into(),
                                message: "An explicitly substituted dependency lowers this project's fidelity.",
                                affected_capability: "render",
                                recovery_actions: Vec::new(),
                            }],
                        }),
                    })
                })),
                ..Default::default()
            };
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
            let warnings = if automation { app.open_automation_bytes("example.xcf", b"xcf") } else { app.open_bytes("example.xcf", b"xcf") }.unwrap();
            assert_eq!(warnings.len(), 2);
            assert!(app.ui.status_error);
            assert!(app.ui.notices.iter().any(|notice| notice.lines.iter().any(|line| line.contains("substituted dependency"))));
            assert_eq!(app.session.documents().len(), 1);
        }
    }

    #[test]
    #[ignore = "real independently authored nearest linked XCF, bounded offscreen Vulkan notice/image"]
    fn original_linked_import_missing_dependency_notice_and_raw_appearance() {
        let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
            .join("transform-interpolation");
        let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
        let bytes = std::fs::read(root.join("link-none.xcf")).unwrap();
        assert_eq!(hash(&bytes), "b40f7d42962b18ce1a7fff73fdc48354b6a32efed3fc559966acc21f5544f674");
        let reference = std::fs::read(root.join("link-none.ref.png")).unwrap();
        assert_eq!(hash(&reference), "ad613286b5d98299feaaf659016ef69e77db512a8915f98421832c850c1b62ef");
        let rgba = photocraft_codecs::decode(&reference).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
        let expected = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.data());
        let services = crate::Services {
            import: Some(Box::new(|name, bytes, _depth| photocraft_io::import(name, bytes).map_err(|error| error.to_string()))),
            ..Default::default()
        };
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let warnings = app.open_bytes("link-none.xcf", &bytes).unwrap();
        assert!(warnings.iter().any(|warning| warning.contains("fidelity limited")));
        assert!(warnings.iter().any(|warning| warning.contains("stored locator grants no read permission")));
        let source = app.session.active().unwrap().doc.clone();
        let ctx = egui::Context::default();
        let (texture, _) = crate::canvas::ensure_texture(&mut app, &ctx, 0, None).unwrap();
        let mut delta = ctx.tex_manager().write().take_delta();
        let image = delta.set.get(&texture).unwrap().last().unwrap().image.clone();
        delta.clear();
        let egui::ImageData::Color(image) = image;
        assert_eq!(image.size, expected.size);
        for (actual, expected) in image.pixels.iter().zip(&expected.pixels) {
            for (a, e) in actual.to_array().into_iter().zip(expected.to_array()) {
                assert!(a.abs_diff(e) <= 2);
            }
        }
        assert_eq!(*app.session.active().unwrap().doc, *source);
        let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_LINKED_UI_VISUALS").expect("explicit private visual directory"));
        std::fs::create_dir_all(&directory).unwrap();
        let mut setup = egui_kittest::wgpu::default_wgpu_setup();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
            create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
        }
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(720., 520.)).wgpu_setup(setup).build_ui_state(
            |ui, app| {
                ui.label("link-none.xcf");
                if let Some((texture, _)) = crate::canvas::ensure_texture(app, ui.ctx(), 0, None) {
                    ui.image((texture, egui::vec2(280., 240.)));
                }
                crate::notices::show(app, ui.ctx());
            },
            app,
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
        harness.run_steps(2);
        assert_eq!(*harness.state().session.active().unwrap().doc, *source);
        assert!(!harness.state().ui.notices.is_empty());
        harness.render().unwrap().save(directory.join("linked-missing-dependency.png")).unwrap();
        assert_eq!(std::fs::read(root.join("link-none.xcf")).unwrap(), bytes);
    }
}
