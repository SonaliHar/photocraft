#[path = "common/xcf.rs"]
mod common;
use photocraft_doc::{LayerContent, Rect, SampleType};

#[test]
fn native_import_and_export_reject_forged_implicit_profile_semantics() {
    use photocraft_doc::{ColorMode, Document, Size, WorkingProfileInterpretation};
    let mut doc = Document::new("forged profile", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    let assigned = photocraft_cms::Builtin::DisplayP3.profile().to_bytes();
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&assigned);
    doc.icc_profile = Some(assigned);
    doc.validate_xcf_state().unwrap();
    let before = doc.clone();
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert!(photocraft_io::import("forged.pcraft", &native).is_err());
    for extension in ["pcraft", "png", "psd"] {
        assert!(photocraft_io::export(&doc, extension, &Default::default()).is_err());
    }
    assert_eq!(doc, before);
}

#[test]
fn normal_auto_policy_is_resolved_once_and_preserved_with_stored_values() {
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    for (mode, expected) in [(0_i32, GimpSpace::ProfileRgb), (28_i32, GimpSpace::LinearRgb)] {
        let options = common::FixtureOptions {
            layer_properties: vec![
                (7, mode.to_be_bytes().to_vec()),
                (35, 0_i32.to_be_bytes().to_vec()),
                (36, 0_i32.to_be_bytes().to_vec()),
                (37, 0_i32.to_be_bytes().to_vec()),
            ],
            ..Default::default()
        };
        let imported = photocraft_io::import("auto.xcf", &common::image_custom(12, 150, 0, 1, 1, &[128, 64, 32, 255], &options)).unwrap();
        let policy = imported.document.xcf_state.as_ref().unwrap().items[0].composition.as_ref().unwrap();
        assert_eq!((policy.source_mode, policy.stored_operator, policy.stored_composite_space, policy.stored_blend_space), (mode, 0, 0, 0));
        let resolved = policy.resolved.as_ref().unwrap();
        assert_eq!((resolved.composite_space, resolved.blend_space, resolved.operator), (expected, expected, GimpOperator::Union));
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.xcf_state, imported.document.xcf_state);
    }
}

#[test]
fn wide_psd_export_is_explicitly_converted_and_source_is_unchanged() {
    let bytes: Vec<_> = [1u32, u32::MAX / 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect();
    let imported = photocraft_io::import("wide.xcf", &common::image(12, 350, 0, 1, 1, &bytes)).unwrap();
    let before = imported.document.clone();
    let output = photocraft_io::export(&before, "wide.psd", &Default::default()).unwrap();
    assert!(output.warnings.iter().any(|w| w.contains("U32/F64 samples to F32")));
    let reopened = photocraft_io::import("wide.psd", &output.bytes).unwrap();
    assert_eq!(reopened.document.depth, SampleType::F32);
    let expected = before.layers[0].surface().unwrap().try_convert(before.pixel_format().with_sample(SampleType::F32)).unwrap();
    assert_eq!(
        reopened.document.layers[0].surface().unwrap().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(),
        expected.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap()
    );
    assert_eq!(imported.document, before);
}

#[test]
fn wide_psd_export_rejects_unrepresentable_samples_without_mutation() {
    let bytes: Vec<_> = [f64::MAX, 0.5, 0.25, 1.0].into_iter().flat_map(f64::to_be_bytes).collect();
    let imported = photocraft_io::import("wide.xcf", &common::image(12, 750, 0, 1, 1, &bytes)).unwrap();
    let before = imported.document.clone();
    assert!(photocraft_io::export(&before, "wide.psd", &Default::default()).is_err());
    assert_eq!(imported.document, before);
}
#[test]
fn layered_tiff_rejects_wide_samples_without_mutation() {
    for precision in [350, 750] {
        let bytes = if precision == 350 {
            [1_u32, u32::MAX / 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect::<Vec<_>>()
        } else {
            [0.5 + 2.0_f64.powi(-40), -0.125, 2.0, 1.0].into_iter().flat_map(f64::to_be_bytes).collect::<Vec<_>>()
        };
        let doc = photocraft_io::import("wide.xcf", &common::image(12, precision, 0, 1, 1, &bytes)).unwrap().document;
        let before = doc.clone();
        let options = photocraft_io::ExportOptions { tiff_layers: true, ..Default::default() };
        let error = photocraft_io::export(&doc, "layers.tiff", &options).unwrap_err();
        assert!(error.to_string().contains("Layered TIFF cannot retain U32/F64"));
        assert_eq!(doc, before);
    }
}

#[test]
fn native_save_retains_wide_source_when_optional_display_preview_is_unrepresentable() {
    let bytes: Vec<_> = [f64::MAX, 0.5, 0.25, 1.0].into_iter().flat_map(f64::to_be_bytes).collect();
    let imported = photocraft_io::import("wide.xcf", &common::image(12, 750, 0, 1, 1, &bytes)).unwrap();
    let before = imported.document.clone();
    let native = photocraft_io::export(&before, "wide.pcraft", &Default::default()).unwrap();
    assert!(native.warnings.iter().any(|warning| warning.contains("without optional previews")));
    let reopened = photocraft_format::load_from_bytes(&native.bytes).unwrap();
    assert_eq!(reopened.layers[0].surface(), before.layers[0].surface());
    assert_eq!(reopened.xcf_state, before.xcf_state);
    assert_eq!(imported.document, before);
}

#[test]
fn wide_flat_export_composes_before_explicit_codec_narrowing() {
    use photocraft_doc::{ColorMode, Document, Layer, PixelFormat, SampleEncoding, Size};
    let mut doc = Document::new("cancellation", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
    doc.sample_encoding = Some(SampleEncoding::Linear);
    doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    for rgba in [[-1e12, 0.25, 0.5, 1.0], [1e12 + 0.25, 0.75, 0.5, 0.5]] {
        let raw: Vec<_> = rgba.into_iter().flat_map(f64::to_ne_bytes).collect();
        doc.layers.push(Layer::new("pixels", LayerContent::Raster(photocraft_raster::Surface::try_from_interleaved(fmt, doc.bounds(), &raw).unwrap())));
    }
    let before = doc.clone();
    let mut warnings = Vec::new();
    let image = photocraft_io::document_to_image(&doc, &mut warnings).unwrap();
    let first = f32::from_ne_bytes(image.data()[..4].try_into().unwrap());
    assert_eq!(first, 0.125);
    assert_eq!(doc, before);
    assert!(warnings.iter().any(|w| w.contains("U32/F64")));
}

#[test]
fn wide_u32_and_f64_samples_import_and_migrate_without_narrowing() {
    for (precision, sample, bytes) in [
        (350, SampleType::U32, [0u32, 1, u32::MAX - 1, u32::MAX].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>()),
        (750, SampleType::F64, [-0.0f64, 1.0 + f64::EPSILON, -3.25, 1.0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>()),
    ] {
        let input = common::image(12, precision, 0, 1, 1, &bytes);
        let imported = photocraft_io::import("renamed.bin", &input).unwrap();
        assert_eq!(imported.document.depth, sample);
        let LayerContent::Raster(surface) = &imported.document.layers[0].content else { panic!("raster") };
        let source = surface.to_interleaved(Rect::new(0, 0, 1, 1));
        let expected = bytes
            .chunks_exact(sample.bytes())
            .flat_map(|chunk| {
                let mut native = chunk.to_vec();
                if cfg!(target_endian = "little") {
                    native.reverse();
                }
                native
            })
            .collect::<Vec<_>>();
        assert_eq!(source, expected);
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.layers, imported.document.layers);
        assert_eq!(reopened.source_color_context, imported.document.source_color_context);
    }
}

#[test]
fn unequal_xcf_axes_survive_native_reopen_and_psd_export() {
    let axes = [137.5f32, 92.25];
    let options = common::FixtureOptions { image_properties: vec![(19, axes.iter().flat_map(|v| v.to_be_bytes()).collect())], ..Default::default() };
    let input = common::image_custom(12, 150, 0, 1, 1, &[40, 60, 80, 255], &options);
    let imported = photocraft_io::import("axes.xcf", &input).unwrap();
    let resolution = imported.document.effective_resolution();
    assert_eq!((resolution.x_dpi, resolution.y_dpi), (137.5, 92.25));
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.effective_resolution(), resolution);
    let exported = photocraft_io::export(&reopened, "out.psd", &Default::default()).unwrap();
    let psd = photocraft_psd::PsdFile::from_bytes(&exported.bytes).unwrap();
    let record = psd.resources.iter().find(|r| r.id == photocraft_psd::resources::ids::RESOLUTION_INFO).unwrap();
    let resolution = photocraft_psd::ResolutionInfo::from_bytes(&record.data).unwrap();
    assert_eq!((resolution.h_res(), resolution.v_res()), (137.5, 92.25));
}
