//! Typed byte-owned linked-image refresh. No source locator or source raster is read.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    Document, LayerId, PixelFormat, SampleEncoding,
    item_identity::ItemRef,
    xcf_items::{EditableSource, SourceItem},
};
use photocraft_raster::{Interrupt, Surface};

fn unsupported() -> CompositionError {
    CompositionError::Unsupported("unqualified linked image profile, precision or transform")
}
/// Only independently qualified linked interpolation families share placement checks.
pub fn qualify_linked_geometry(width: u32, height: u32, transform: &photocraft_doc::xcf_items::TransformState, bounds: photocraft_doc::Rect) -> Result<()> {
    match transform.interpolation {
        0 => crate::gimp_link_nearest::qualify_geometry(width, height, transform, bounds),
        1 => crate::gimp_link_linear::qualify_geometry(width, height, transform, bounds),
        2 => crate::gimp_link_cubic::qualify_geometry(width, height, transform, bounds),
        3 => crate::gimp_link_nohalo::qualify_geometry(width, height, transform, bounds),
        _ => Err(unsupported()),
    }
}
fn qualify_png_metadata(bytes: &[u8]) -> Result<()> {
    let mut at = 8usize;
    while at < bytes.len() {
        let header_end = at.checked_add(8).ok_or_else(unsupported)?;
        let header = bytes.get(at..header_end).ok_or_else(unsupported)?;
        let length = header.get(..4).and_then(|value| value.try_into().ok()).map(u32::from_be_bytes).ok_or_else(unsupported)?;
        let tag = header.get(4..8).ok_or_else(unsupported)?;
        // Separate transfer/chromaticity and orientation policies need their own
        // source interpretation proof; exact ICC matching alone does not resolve them.
        if [b"gAMA".as_slice(), b"cHRM", b"sRGB", b"eXIf"].contains(&tag) {
            return Err(unsupported());
        }
        at = header_end.checked_add(usize::try_from(length).map_err(|_| unsupported())?).and_then(|end| end.checked_add(4)).ok_or_else(unsupported)?;
        if at > bytes.len() {
            return Err(unsupported());
        }
        if tag == b"IEND" {
            return if at == bytes.len() { Ok(()) } else { Err(unsupported()) };
        }
    }
    Err(unsupported())
}

// Owned resource pixels have their own ICC identity. Convert explicitly into the
// active document interpretation; neither provenance nor approximate profile equality
// supplies authority or substitutes for a color transform.
fn convert_owned_pixels(
    doc: &Document,
    bytes: &[u8],
    source_icc: &[u8],
    format: PixelFormat,
    ctl: &Interrupt<'_>,
) -> Result<(Vec<u8>, photocraft_cms::Profile)> {
    use photocraft_cms::{ColorSpace, Intent, Transform};
    let space = if doc.mode == photocraft_doc::ColorMode::Rgb { ColorSpace::Rgb } else { ColorSpace::Gray };
    let active_icc = doc.icc_profile.as_deref().ok_or_else(unsupported)?;
    doc.working_profile_interpretation
        .validate_with_interrupt(Some(active_icc), doc.mode, ctl)
        .map_err(|error| if error.code == "cancelled" { photocraft_raster::RasterInputError::Cancelled.into() } else { unsupported() })?;
    let source = photocraft_cms::xcf_color::parse_bounded_profile(source_icc, space)?;
    let working = if matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc) {
        photocraft_cms::xcf_color::parse_bounded_profile(active_icc, space)?
    } else {
        photocraft_cms::xcf_color::implicit_source_profile(active_icc, space, photocraft_cms::xcf_color::Encoding::NonlinearProfile)?
    };
    photocraft_cms::xcf_color::qualify_analytic_profile(&source)?;
    photocraft_cms::xcf_color::qualify_analytic_profile(&working)?;
    let mut output = Vec::new();
    output.try_reserve_exact(bytes.len()).map_err(|_| photocraft_raster::RasterInputError::AllocationFailed)?;
    output.extend_from_slice(bytes);
    if source_icc == active_icc && matches!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc) {
        return Ok((output, working));
    }
    let transform = Transform::new(&source, &working, Intent::RelativeColorimetric, false)?;
    let channels = if space == ColorSpace::Rgb { 3 } else { 1 };
    let width = match doc.depth {
        photocraft_doc::SampleType::U8 => 1,
        photocraft_doc::SampleType::U16 => 2,
        _ => return Err(unsupported()),
    };
    let maximum = if width == 1 { 255.0 } else { 65535.0 };
    let stride = format.bytes_per_pixel();
    if !bytes.len().is_multiple_of(stride) {
        return Err(unsupported());
    }
    for (index, pixel) in output.chunks_exact_mut(stride).enumerate() {
        if index.is_multiple_of(256) {
            ctl.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        }
        let mut input = [0.0; 3];
        let mut converted = [0.0; 3];
        for channel in 0..channels {
            let offset = channel * width;
            let value = if width == 1 {
                f64::from(*pixel.get(offset).ok_or_else(unsupported)?)
            } else {
                f64::from(u16::from_ne_bytes(pixel.get(offset..offset + 2).and_then(|v| v.try_into().ok()).ok_or_else(unsupported)?))
            };
            *input.get_mut(channel).ok_or_else(unsupported)? = value / maximum;
        }
        transform.try_eval_f64(input.get(..channels).ok_or_else(unsupported)?, converted.get_mut(..channels).ok_or_else(unsupported)?)?;
        for channel in 0..channels {
            let value = *converted.get(channel).ok_or_else(unsupported)?;
            if !value.is_finite() {
                return Err(unsupported());
            }
            let value = (value.clamp(0.0, 1.0) * maximum).round();
            let offset = channel * width;
            if width == 1 {
                *pixel.get_mut(offset).ok_or_else(unsupported)? = value as u8;
            } else {
                pixel.get_mut(offset..offset + 2).ok_or_else(unsupported)?.copy_from_slice(&(value as u16).to_ne_bytes());
            }
        }
    }
    Ok((output, working))
}

/// Decode an explicitly resolved, owned PNG under its exact current color interpretation.
/// ICC conversion preserves declared integer precision and alpha; transform families
/// are admitted only by their separately qualified consumers.
pub fn prepare_current_source(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>) -> Result<(LayerId, Surface)> {
    ctl.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
    let EditableSource::Linked { resource, configuration, current, .. } = &source.source else {
        return Err(unsupported());
    };
    let ItemRef::Layer(owner) = source.owner else {
        return Err(unsupported());
    };
    if doc.sample_encoding != Some(SampleEncoding::NonlinearProfile) {
        return Err(unsupported());
    }
    let state = doc.xcf_state.as_ref().ok_or_else(unsupported)?;
    let retained = state.resources.iter().find(|entry| entry.id == *resource).ok_or_else(unsupported)?;
    let resolution = retained.resolution.as_ref().ok_or_else(unsupported)?;
    let bytes = retained.owned_bytes.as_ref().map(|bytes| bytes.as_slice()).ok_or_else(unsupported)?;
    if bytes.len() > 8 << 20 || photocraft_codecs::detect(bytes) != Some(photocraft_codecs::Format::Png) {
        return Err(unsupported());
    }
    qualify_png_metadata(bytes)?;
    // Shared validation checks requested vs current hashes and substitution identity.
    doc.validate_xcf_state_with(ctl).map_err(|e| {
        if e.code == "cancelled" { photocraft_raster::RasterInputError::Cancelled.into() } else { CompositionError::Invalid("invalid linked resource state") }
    })?;
    if resolution.payload_hash.is_empty() {
        return Err(unsupported());
    }
    let image = photocraft_codecs::decode_with(
        bytes,
        &photocraft_codecs::DecodeOptions {
            limits: photocraft_codecs::Limits { max_width: 8192, max_height: 8192, max_pixels: 4_000_000, max_alloc: 16 << 20 },
            ..Default::default()
        },
    )
    .map_err(|_| CompositionError::Invalid("invalid or oversized linked image"))?;
    let mut fields = std::collections::BTreeSet::new();
    if configuration.len() > 3 {
        return Err(unsupported());
    }
    for argument in configuration {
        if !fields.insert(argument.name.as_str()) {
            return Err(unsupported());
        }
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }) = argument.value else {
            return Err(unsupported());
        };
        match argument.name.as_str() {
            "flags" if value <= 3 => {}
            "source-width" if value == u64::from(image.dimensions().0) => {}
            "source-height" if value == u64::from(image.dimensions().1) => {}
            _ => return Err(unsupported()),
        }
    }
    ctl.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
    if !image.warnings.is_empty() || doc.icc_profile.is_none() || image.icc.is_none() {
        return Err(unsupported());
    }
    let mode = match image.layout() {
        photocraft_codecs::ChannelLayout::Rgba => photocraft_doc::ColorMode::Rgb,
        photocraft_codecs::ChannelLayout::GrayA => photocraft_doc::ColorMode::Grayscale,
        _ => return Err(unsupported()),
    };
    let sample = match image.sample_type() {
        photocraft_codecs::SampleType::U8 => photocraft_doc::SampleType::U8,
        photocraft_codecs::SampleType::U16 => photocraft_doc::SampleType::U16,
        photocraft_codecs::SampleType::F16 => return Err(unsupported()),
        photocraft_codecs::SampleType::F32 => photocraft_doc::SampleType::F32,
    };
    if mode != doc.mode || sample != doc.depth {
        return Err(unsupported());
    }
    let (pixels, working_profile) =
        convert_owned_pixels(doc, image.data(), image.icc.as_deref().ok_or_else(unsupported)?, PixelFormat::new(mode, sample, true), ctl)?;
    let bounds = state.items.iter().find(|item| item.item == source.owner).and_then(|item| item.raster_bounds).ok_or_else(unsupported)?;
    let format = PixelFormat::new(mode, sample, true);
    let transform = current.as_ref().map_or(source.transform.as_ref(), |state| state.transform.as_ref());
    if let Some(transform) = transform {
        if transform.interpolation == 3 {
            return Ok((
                owner,
                crate::gimp_link_nohalo::transform_with_profile(
                    &pixels,
                    image.dimensions().0,
                    image.dimensions().1,
                    format,
                    transform,
                    bounds,
                    &working_profile,
                    ctl,
                )?,
            ));
        }
        if transform.interpolation == 2 {
            return Ok((
                owner,
                crate::gimp_link_cubic::transform_with_profile(
                    &pixels,
                    image.dimensions().0,
                    image.dimensions().1,
                    format,
                    transform,
                    bounds,
                    &working_profile,
                    ctl,
                )?,
            ));
        }
        if transform.interpolation == 1 {
            return Ok((
                owner,
                crate::gimp_link_linear::transform_with_profile(
                    &pixels,
                    image.dimensions().0,
                    image.dimensions().1,
                    format,
                    transform,
                    bounds,
                    &working_profile,
                    ctl,
                )?,
            ));
        }
        return Ok((owner, crate::gimp_link_nearest::transform(&pixels, image.dimensions().0, image.dimensions().1, format, transform, bounds, ctl)?));
    }
    if (bounds.width(), bounds.height()) != image.dimensions() {
        return Err(unsupported());
    }
    let mut surface = Surface::try_with_default_bytes(format, [0u8; 32].get(..format.bytes_per_pixel()).ok_or_else(unsupported)?)?;
    surface.try_write_interleaved_with(bounds, &pixels, ctl)?;
    surface.prune();
    Ok((owner, surface))
}

/// A retained source cache never establishes byte authority. The current raster must
/// agree exactly with the independently decoded, explicitly owned resource material.
pub fn qualify_current_source(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>) -> Result<()> {
    let EditableSource::Linked { resource, last_verified_hash, .. } = &source.source else {
        return Err(unsupported());
    };
    let digest = doc
        .xcf_state
        .as_ref()
        .and_then(|state| state.resources.iter().find(|entry| entry.id == *resource))
        .and_then(|entry| entry.resolution.as_ref())
        .map(|resolution| resolution.payload_hash.as_str());
    if digest.is_none() || digest != last_verified_hash.as_deref() {
        return Err(unsupported());
    }
    let (owner, expected) = prepare_current_source(doc, source, ctl)?;
    let current = doc.layer(owner).and_then(|layer| layer.surface()).ok_or_else(unsupported)?;
    if current != &expected {
        return Err(CompositionError::Invalid("linked raster is stale; refresh the resolved resource"));
    }
    Ok(())
}

/// Available original linked appearance, distinct from regenerated dependency identity.
/// Only the qualified RGB/U8 nonlinear nearest/linear/cubic families are admitted. A resource locator
/// is never read; a newly recovered payload remains pending until explicit refresh.
pub fn qualify_original_cache(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>) -> Result<()> {
    let EditableSource::Linked { resource, last_verified_hash: None, configuration, current: None } = &source.source else {
        return Err(unsupported());
    };
    if doc.mode != photocraft_doc::ColorMode::Rgb
        || doc.depth != photocraft_doc::SampleType::U8
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || configuration.len() != 3
    {
        return Err(unsupported());
    }
    let icc = doc.icc_profile.as_deref().map(Vec::as_slice).ok_or_else(unsupported)?;
    photocraft_cms::xcf_color::parse_bounded_profile(icc, photocraft_cms::ColorSpace::Rgb).map_err(|_| unsupported())?;
    doc.validate_xcf_state_with(ctl).map_err(|error| {
        if error.code == "cancelled" {
            photocraft_raster::RasterInputError::Cancelled.into()
        } else {
            CompositionError::Invalid("invalid original linked state")
        }
    })?;
    let state = doc.xcf_state.as_ref().ok_or_else(unsupported)?;
    if !state.resources.iter().any(|entry| entry.id == *resource && entry.kind == photocraft_doc::xcf_resources::ResourceKind::LinkedImage) {
        return Err(unsupported());
    }
    let mut width = None;
    let mut height = None;
    let mut flags = None;
    for argument in configuration {
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }) = argument.value else {
            return Err(unsupported());
        };
        let value = u32::try_from(value).map_err(|_| unsupported())?;
        match argument.name.as_str() {
            "flags" if flags.is_none() => flags = Some(value),
            "source-width" if width.is_none() => width = Some(value),
            "source-height" if height.is_none() => height = Some(value),
            _ => return Err(unsupported()),
        }
    }
    // Official XCF PROP_LINK_LAYER bits concern naming and file monitoring.
    // They never authorize a read and do not select the stored pixel sampler.
    if flags.is_none_or(|value| value > 3) {
        return Err(unsupported());
    }
    let bounds = state.items.iter().find(|item| item.item == source.owner).and_then(|item| item.raster_bounds).ok_or_else(unsupported)?;
    qualify_linked_geometry(width.ok_or_else(unsupported)?, height.ok_or_else(unsupported)?, source.transform.as_ref().ok_or_else(unsupported)?, bounds)?;
    let cache = source.cache.as_ref().ok_or_else(unsupported)?;
    if cache.format != doc.pixel_format() || (cache.width, cache.height) != (bounds.width(), bounds.height()) {
        return Err(unsupported());
    }
    let ItemRef::Layer(id) = source.owner else {
        return Err(unsupported());
    };
    let current = doc.layer(id).and_then(|layer| layer.surface()).ok_or_else(unsupported)?;
    let mut expected = Surface::try_with_default_bytes(cache.format, [0u8; 40].get(..cache.format.bytes_per_pixel()).ok_or_else(unsupported)?)?;
    expected.try_write_interleaved_with(bounds, &cache.bytes, ctl)?;
    expected.prune();
    if current != &expected {
        return Err(CompositionError::Invalid("original linked drawable cache is stale"));
    }
    Ok(())
}

/// Select original available appearance or explicitly regenerated current appearance.
/// Successful original selection does not make its dependency exact or grant a read.
pub fn qualify_display_source(doc: &Document, source: &SourceItem, ctl: &Interrupt<'_>) -> Result<()> {
    match &source.source {
        EditableSource::Linked { last_verified_hash: None, .. } => qualify_original_cache(doc, source, ctl),
        EditableSource::Linked { .. } => qualify_current_source(doc, source, ctl),
        _ => Err(unsupported()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, SampleType, Size, XcfState, item_identity::ResourceId, xcf_items::SourceCache, xcf_resources::*};
    fn project(mode: ColorMode, depth: SampleType) -> (Document, Vec<u8>) {
        let mut doc = Document::with_background("Own linked identity", Size::new(2, 1), mode, depth, Color::WHITE);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(if mode == ColorMode::Rgb {
            photocraft_cms::builtin::srgb().to_bytes()
        } else {
            photocraft_cms::builtin::Builtin::GrayGamma22.profile().to_bytes()
        });
        let format = doc.pixel_format();
        let raw: Vec<u8> = if depth == SampleType::U8 {
            (0..format.bytes_per_pixel() * 2).map(|i| (i * 17 + 3) as u8).collect()
        } else {
            (0..format.channels() * 2).flat_map(|i| ((i * 3131 + 17) as u16).to_ne_bytes()).collect()
        };
        let mut image = photocraft_codecs::Image::from_raw(
            2,
            1,
            if mode == ColorMode::Rgb { photocraft_codecs::ChannelLayout::Rgba } else { photocraft_codecs::ChannelLayout::GrayA },
            if depth == SampleType::U8 { photocraft_codecs::SampleType::U8 } else { photocraft_codecs::SampleType::U16 },
            raw.clone(),
        )
        .unwrap();
        image.icc = doc.icc_profile.as_deref().cloned();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let digest = blake3::hash(&png).to_hex().to_string();
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items[0].raster_bounds = Some(doc.bounds());
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own material".into(),
            collection: None,
            internal: false,
            locator: Some("https://invalid.example/never-read".into()),
            content_hash: Some(digest.clone()),
            owned_bytes: Some(png.into()),
            resolution: Some(ResourceResolution { fidelity: ResourceFidelity::Exact, payload_hash: digest.clone() }),
        });
        let original = vec![211; raw.len()];
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(doc.layers[0].id),
            source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: Some(digest), configuration: vec![], current: None },
            transform: None,
            cache: Some(SourceCache { blake3: blake3::hash(&original).to_hex().to_string(), bytes: original.into(), width: 2, height: 1, format }),
        });
        doc.xcf_state = Some(state);
        (doc, raw)
    }
    #[test]
    fn identity_decode_preserves_rgb_gray_u8_u16_and_uses_owned_material_not_source_cache() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16] {
                let (mut doc, raw) = project(mode, depth);
                let source = &doc.xcf_state.as_ref().unwrap().source_items[0];
                let (id, surface) = prepare_current_source(&doc, source, &Interrupt::NONE).unwrap();
                assert_eq!(surface.try_read_interleaved(doc.bounds()).unwrap(), raw);
                assert!(qualify_current_source(&doc, source, &Interrupt::NONE).is_err());
                doc.layer_mut(id).unwrap().content = photocraft_doc::LayerContent::Raster(surface);
                qualify_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).unwrap();
            }
        }
    }
    #[test]
    fn explicit_profile_conversion_preserves_alpha_and_rejects_stale_current_binding() {
        let source = photocraft_cms::Builtin::Srgb.profile().to_bytes();
        let mut doc = Document::new("Owned color conversion", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        let (pixels, _) = convert_owned_pixels(&doc, &[192, 32, 96, 41], &source, PixelFormat::RGBA8, &Interrupt::NONE).unwrap();
        assert_eq!(pixels[3], 41);
        assert_ne!(pixels[..3], [192, 32, 96]);
        assert!(convert_owned_pixels(&doc, &[192, 32, 96, 41], &source, PixelFormat::RGBA8, &Interrupt::cancel_only(&|| true)).is_err());
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(&source);
        assert!(convert_owned_pixels(&doc, &[192, 32, 96, 41], &source, PixelFormat::RGBA8, &Interrupt::NONE).is_err());
    }
    #[test]
    fn source_cache_no_grant_cancel_and_missing_owned_bytes_never_establish_authority() {
        let (mut doc, _) = project(ColorMode::Rgb, SampleType::U8);
        assert!(prepare_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::cancel_only(&|| true)).is_err());
        doc.xcf_state.as_mut().unwrap().resources[0].owned_bytes = None;
        assert!(prepare_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).is_err());
    }

    #[cfg(feature = "corpus")]
    #[test]
    fn frozen_link_none_projective_native_oracle_matches_every_byte_and_placement() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation");
        let png = std::fs::read(root.join("own-linked-resource.png")).expect("frozen XCF resource corpus is required");
        let oracle = std::fs::read(root.join("link-none.source.0.native.bin")).expect("independent GIMP native oracle is required");
        use sha2::Digest;
        assert_eq!(
            sha2::Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "fec97c32d4d01dfe813191f1e37eadd6d409556d6ad98dd8e02fd4bbea5260a8"
        );
        let source_xcf = std::fs::read(root.join("link-none.xcf")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&source_xcf).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "b40f7d42962b18ce1a7fff73fdc48354b6a32efed3fc559966acc21f5544f674"
        );
        assert_eq!(
            sha2::Sha256::digest(&oracle).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "ea6660ef7a5968a36bac5cad9ac9025f0ce7dce424c1f2763bd353464462e0e4"
        );
        let image = photocraft_codecs::decode(&png).unwrap();
        assert_eq!(image.dimensions(), (96, 80));
        let mut doc = Document::with_background("Frozen own link nearest", Size::new(96, 80), ColorMode::Rgb, SampleType::U8, Color::TRANSPARENT);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = image.icc.map(std::sync::Arc::new);
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        let bounds = photocraft_doc::Rect::new(2, -6, 90, 73);
        state.items[0].raster_bounds = Some(bounds);
        let digest = blake3::hash(&png).to_hex().to_string();
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own frozen resource".into(),
            collection: None,
            internal: false,
            locator: Some("own-linked-resource.png".into()),
            content_hash: None,
            owned_bytes: Some(png.into()),
            resolution: Some(ResourceResolution { fidelity: ResourceFidelity::Substituted, payload_hash: digest }),
        });
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(doc.layers[0].id),
            source: EditableSource::Linked {
                resource: ResourceId(1),
                last_verified_hash: None,
                current: None,
                configuration: [("flags", 1), ("source-width", 96), ("source-height", 80)]
                    .into_iter()
                    .map(|(name, value)| photocraft_doc::xcf_filters::FilterArgument {
                        name: name.into(),
                        value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }),
                    })
                    .collect(),
            },
            transform: Some(photocraft_doc::xcf_items::TransformState {
                pre_offset: [0, 0],
                interpolation: 0,
                matrix_f64_bits: [1.0f32, 0.1, 2.0, -0.1, 1.0, 4.0, 0.001, 0.002, 1.0].map(|v| f64::from(v).to_bits()),
            }),
            cache: Some(SourceCache {
                width: 88,
                height: 79,
                format: doc.pixel_format(),
                blake3: blake3::hash(&oracle).to_hex().to_string(),
                bytes: oracle.clone().into(),
            }),
        });
        doc.xcf_state = Some(state);
        let (_, output) = prepare_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(bounds).unwrap(), oracle);
        let mut original = doc.clone();
        original.layers[0].content = photocraft_doc::LayerContent::Raster(output);
        original.xcf_state.as_mut().unwrap().resources[0].owned_bytes = None;
        original.xcf_state.as_mut().unwrap().resources[0].resolution = None;
        let source = &original.xcf_state.as_ref().unwrap().source_items[0];
        qualify_original_cache(&original, source, &Interrupt::NONE).unwrap();
        qualify_display_source(&original, source, &Interrupt::NONE).unwrap();
        let mut escaped = original.clone();
        let photocraft_doc::LayerContent::Raster(surface) = &mut escaped.layers[0].content else { panic!("expected raw linked raster") };
        surface.try_write_interleaved(photocraft_doc::Rect::new(95, 75, 96, 76), &[255, 12, 64, 255]).unwrap();
        assert!(qualify_original_cache(&escaped, &escaped.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).is_err());
        assert!(qualify_current_source(&original, source, &Interrupt::NONE).is_err());
        assert!(qualify_original_cache(&original, source, &Interrupt::cancel_only(&|| true)).is_err());
        original.layers[0].content = doc.layers[0].content.clone();
        assert!(qualify_original_cache(&original, &original.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).is_err());
    }
    #[cfg(feature = "corpus")]
    #[test]
    fn frozen_link_linear_projective_native_oracle_meets_raw_channel_floor_and_placement() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation");
        let png = std::fs::read(root.join("own-linked-resource.png")).expect("frozen XCF resource corpus is required");
        let oracle = std::fs::read(root.join("link-linear.source.0.native.bin")).expect("independent GIMP native oracle is required");
        use sha2::Digest;
        assert_eq!(
            sha2::Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "fec97c32d4d01dfe813191f1e37eadd6d409556d6ad98dd8e02fd4bbea5260a8"
        );
        let source_xcf = std::fs::read(root.join("link-linear.xcf")).unwrap();
        assert_eq!(
            sha2::Sha256::digest(&source_xcf).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "d6c6a815a52f5d8c1501829756efb4dc5baed476d72f43de8767a50b147de4c0"
        );
        assert_eq!(
            sha2::Sha256::digest(&oracle).iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "d064490959e917bbb4f6d21e525afdc8adcb899d3b8b421d039467d46f88b518"
        );
        let image = photocraft_codecs::decode(&png).unwrap();
        assert_eq!(image.dimensions(), (96, 80));
        let mut doc = Document::with_background("Frozen own link nearest", Size::new(96, 80), ColorMode::Rgb, SampleType::U8, Color::TRANSPARENT);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = image.icc.map(std::sync::Arc::new);
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        let bounds = photocraft_doc::Rect::new(2, -6, 90, 73);
        state.items[0].raster_bounds = Some(bounds);
        let digest = blake3::hash(&png).to_hex().to_string();
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own frozen resource".into(),
            collection: None,
            internal: false,
            locator: Some("own-linked-resource.png".into()),
            content_hash: None,
            owned_bytes: Some(png.into()),
            resolution: Some(ResourceResolution { fidelity: ResourceFidelity::Substituted, payload_hash: digest }),
        });
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(doc.layers[0].id),
            source: EditableSource::Linked {
                resource: ResourceId(1),
                last_verified_hash: None,
                current: None,
                configuration: [("flags", 1), ("source-width", 96), ("source-height", 80)]
                    .into_iter()
                    .map(|(name, value)| photocraft_doc::xcf_filters::FilterArgument {
                        name: name.into(),
                        value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Unsigned { width: 32, value }),
                    })
                    .collect(),
            },
            transform: Some(photocraft_doc::xcf_items::TransformState {
                pre_offset: [0, 0],
                interpolation: 1,
                matrix_f64_bits: [1.0f32, 0.1, 2.0, -0.1, 1.0, 4.0, 0.001, 0.002, 1.0].map(|v| f64::from(v).to_bits()),
            }),
            cache: Some(SourceCache {
                width: 88,
                height: 79,
                format: doc.pixel_format(),
                blake3: blake3::hash(&oracle).to_hex().to_string(),
                bytes: oracle.clone().into(),
            }),
        });
        doc.xcf_state = Some(state);
        let (_, output) = prepare_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).unwrap();
        let actual = output.try_read_interleaved(bounds).unwrap();
        assert_eq!(actual.len(), oracle.len());
        let max = actual.iter().zip(&oracle).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(max <= 2, "native linked linear max channel error {max} exceeds frozen 2/255 floor");
        let translated = photocraft_doc::Rect::new(5, -4, 93, 75);
        doc.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(translated);
        let edited_oracle = std::fs::read(root.join("link-linear.edited.0.native.bin")).unwrap();
        let (_, translated_output) = prepare_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::NONE).unwrap();
        let translated_raw = translated_output.try_read_interleaved(translated).unwrap();
        assert_eq!(translated_raw.len(), edited_oracle.len());
        assert!(translated_raw.iter().zip(&edited_oracle).all(|(a, b)| a.abs_diff(*b) <= 2));
        assert!(prepare_current_source(&doc, &doc.xcf_state.as_ref().unwrap().source_items[0], &Interrupt::cancel_only(&|| true)).is_err());
    }
}
