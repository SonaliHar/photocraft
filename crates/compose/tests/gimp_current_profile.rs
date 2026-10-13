//! Current profile interpretation must be independent of immutable XCF provenance.
use photocraft_compose::gimp_render::PreparedRenderer;
use photocraft_doc::{ColorMode, Document, Layer, SampleEncoding, SampleType, Size, SourceColorContext, WorkingProfileInterpretation};
#[test]
fn linear_and_perceptual_validate_current_assigned_profile_even_with_valid_original() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
            let source = if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb.profile() } else { photocraft_cms::Builtin::SGray.profile() };
            let mut doc = Document::new("current-profile", Size::new(1, 1), mode, SampleType::F64);
            doc.sample_encoding = Some(encoding);
            doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
            doc.source_color_context = Some(SourceColorContext { original_icc: Some(source.to_bytes()), source_precision: 775 });
            doc.icc_profile = Some(
                photocraft_cms::xcf_color::effective_profile_from_definition(
                    source,
                    if encoding == SampleEncoding::Linear {
                        photocraft_cms::xcf_color::Encoding::Linear
                    } else {
                        photocraft_cms::xcf_color::Encoding::Perceptual
                    },
                )
                .unwrap()
                .to_bytes(),
            );
            doc.layers.push(Layer::raster("current raster", doc.pixel_format()));
            assert!(PreparedRenderer::new(&doc).is_ok());
            let mut malformed_provenance = doc.clone();
            malformed_provenance.source_color_context.as_mut().unwrap().original_icc = Some(std::sync::Arc::new(vec![1, 2, 3]));
            let unchanged = malformed_provenance.clone();
            assert!(
                PreparedRenderer::new(&malformed_provenance).is_ok(),
                "immutable invalid original bytes are inert under valid explicit current interpretation"
            );
            assert_eq!(malformed_provenance, unchanged);
            let mut wrong_original = doc.clone();
            wrong_original.source_color_context.as_mut().unwrap().original_icc = Some(if mode == ColorMode::Rgb {
                photocraft_cms::Builtin::SGray.profile().to_bytes()
            } else {
                photocraft_cms::Builtin::Srgb.profile().to_bytes()
            });
            assert!(PreparedRenderer::new(&wrong_original).is_ok(), "immutable original model never replaces valid current model");
            let before = doc.clone();
            let mut corrupt = doc.clone();
            corrupt.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3]));
            assert!(PreparedRenderer::new(&corrupt).is_err(), "valid original cannot replace corrupt current profile");
            let mut wrong = doc.clone();
            wrong.icc_profile = Some(if mode == ColorMode::Rgb {
                photocraft_cms::Builtin::SGray.profile().to_bytes()
            } else {
                photocraft_cms::Builtin::Srgb.profile().to_bytes()
            });
            assert!(PreparedRenderer::new(&wrong).is_err(), "valid original cannot replace incompatible current color model");
            let mut absent = doc.clone();
            absent.icc_profile = None;
            assert!(PreparedRenderer::new(&absent).is_err(), "explicit current interpretation requires a current profile");
            assert_eq!(doc, before);
        }
    }
}
