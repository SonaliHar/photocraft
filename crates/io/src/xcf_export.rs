//! Map imported sample interpretation to the PSD ICC equivalent without changing source pixels.
use photocraft_doc::{ColorMode, Document, SampleEncoding};

/// Exact active analytical interpretation; immutable source provenance does not
/// select it after the current profile has changed.
pub(crate) fn active_implicit_profile(doc: &Document) -> Result<Option<photocraft_cms::Profile>, photocraft_cms::CmsError> {
    use photocraft_cms::{
        CmsError, ColorSpace,
        xcf_color::{Encoding, effective_implicit_profile},
    };
    if matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc) {
        return Ok(None);
    }
    doc.working_profile_interpretation.validate_with(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode).map_err(|e| CmsError::Invalid(e.to_string()))?;
    let bytes = doc.icc_profile.as_ref().ok_or_else(|| CmsError::Invalid("missing implicit working ICC".into()))?;
    let space = match doc.mode {
        ColorMode::Rgb | ColorMode::Indexed => ColorSpace::Rgb,
        ColorMode::Grayscale => ColorSpace::Gray,
        _ => return Err(CmsError::Unsupported("implicit working color model".into())),
    };
    let encoding = match doc.sample_encoding.unwrap_or(SampleEncoding::NonlinearProfile) {
        SampleEncoding::Linear => Encoding::Linear,
        SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
        SampleEncoding::Perceptual => Encoding::Perceptual,
    };
    effective_implicit_profile(bytes, space, encoding).map(Some)
}

pub(crate) fn effective_icc(doc: &Document) -> Result<Option<Vec<u8>>, photocraft_cms::CmsError> {
    use photocraft_cms::xcf_color::{Encoding, effective_profile};
    if let Some(profile) = active_implicit_profile(doc)? {
        return Ok(Some(profile.to_bytes().to_vec()));
    }
    let Some(encoding) = doc.sample_encoding else { return Ok(doc.icc_profile.as_ref().map(|p| p.to_vec())) };
    // Nonlinear-profile explicitly delegates transfer to the effective document ICC, including
    // profiles/models resulting from a later user conversion outside XCF's original RGB/Gray.
    if encoding == SampleEncoding::NonlinearProfile {
        return Ok(doc.icc_profile.as_ref().map(|p| p.to_vec()));
    }
    let space = match doc.mode {
        ColorMode::Rgb | ColorMode::Indexed => photocraft_cms::ColorSpace::Rgb,
        ColorMode::Grayscale => photocraft_cms::ColorSpace::Gray,
        _ => return Err(photocraft_cms::CmsError::Unsupported("explicit imported sample encoding is incompatible with the export color model".into())),
    };
    let encoding = if encoding == SampleEncoding::Linear { Encoding::Linear } else { Encoding::Perceptual };
    Ok(Some(effective_profile(doc.icc_profile.as_ref().map(|p| p.as_slice()), space, encoding)?.to_bytes().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_cms::{Builtin, Profile};
    use photocraft_doc::{Layer, SampleType, Size, SourceColorContext};
    #[test]
    fn psd_preserves_sample_bytes_and_embeds_the_actual_transfer_at_every_depth() {
        for (depth, encoding) in
            [(SampleType::U16, SampleEncoding::Linear), (SampleType::F32, SampleEncoding::NonlinearProfile), (SampleType::U8, SampleEncoding::Perceptual)]
        {
            let mut doc = Document::new("imported", Size::new(2, 2), ColorMode::Rgb, depth);
            doc.sample_encoding = Some(encoding);
            doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
            doc.source_color_context = Some(SourceColorContext { original_icc: doc.icc_profile.clone(), source_precision: 500 });
            let mut layer = Layer::raster("samples", doc.pixel_format());
            layer.surface_mut().unwrap().write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
            doc.layers.push(layer);
            let before = doc.clone();
            let original = doc.layers[0].surface().unwrap().pixel(0, 0);
            let (psd, _) = crate::document_to_psd_with(&doc, &Default::default()).unwrap();
            let resource = psd.resources.iter().find(|r| r.id == photocraft_psd::resources::ids::ICC_PROFILE).unwrap();
            let profile = Profile::parse(&resource.data).unwrap();
            let curve = &profile.trc.as_ref().unwrap()[0];
            let expected = if encoding == SampleEncoding::Linear { 0.5 } else { photocraft_color::convert::srgb_to_linear(0.5) };
            assert!((curve.eval(0.5) - expected).abs() < 0.0001);
            let serialized = psd.to_bytes().unwrap();
            let reopened = crate::import("export.psd", &serialized).unwrap().document;
            assert_eq!(reopened.layers[0].surface().unwrap().pixel(0, 0), original);
            assert_eq!(Profile::parse(reopened.icc_profile.as_ref().unwrap()).unwrap().trc.as_ref().unwrap()[0].eval(0.5), curve.eval(0.5));
            assert_eq!(doc, before);
        }
    }
    #[test]
    fn ordinary_exports_keep_original_icc_bytes_and_invalid_override_is_reported() {
        let mut doc = Document::new("ordinary", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        assert_eq!(effective_icc(&doc).unwrap().unwrap(), doc.icc_profile.as_ref().unwrap().as_slice());
        doc.sample_encoding = Some(SampleEncoding::Linear);
        doc.icc_profile = Some(std::sync::Arc::new(vec![0; 16]));
        assert!(effective_icc(&doc).is_err());
        assert!(matches!(crate::export(&doc, "psd", &Default::default()), Err(crate::IoError::Unsupported(_))));
    }
}
