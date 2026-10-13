//! Scoped stock Cubic/Linear compatibility for current implicit RGB/Gray nonlinear rasters.
use crate::{EngineError, Result};
use photocraft_algo::{transform::Homography, wide_geometry::GeometryLimits};
use photocraft_cms::{
    Intent, Transform,
    xcf_color::{Encoding, effective_profile_from_definition},
};
use photocraft_doc::{Document, PixelFormat, SampleType, WorkingProfileInterpretation};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface, checked_region_byte_len};
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
fn resident(surface: &Surface) -> Result<u64> {
    surface
        .resident_payload_bytes()
        .map_err(|e| bad(e.to_string()))?
        .checked_add((surface.tile_count() as u64).checked_mul(256).ok_or_else(|| bad("cubic memory overflow"))?)
        .ok_or_else(|| bad("cubic memory overflow"))
}
fn check_memory(source: &Surface, output: &Surface, retained: u64, scratch: u64) -> Result<()> {
    let bytes = resident(source)?
        .checked_add(resident(output)?)
        .and_then(|v| v.checked_add(retained))
        .and_then(|v| v.checked_add(scratch))
        .ok_or_else(|| bad("cubic memory overflow"))?;
    if bytes > GeometryLimits::default().resident_bytes {
        return Err(bad("cubic staging exceeds resident budget"));
    }
    Ok(())
}
fn convert(
    source: &Surface,
    bounds: Rect,
    target: PixelFormat,
    transform: &Transform,
    options: (u64, bool, bool, bool),
    control: &Interrupt<'_>,
) -> Result<Surface> {
    let (retained, round_generated, finite_normal, half_target) = options;
    let mut output = Surface::new(target);
    let colors = target.mode.color_channels();
    let channels = target.channels();
    for tile in bounds.tiles() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let area = tile.rect().intersect(&bounds);
        let len = checked_region_byte_len(area, target).map_err(|e| bad(e.to_string()))?;
        let tile_bytes = (photocraft_geom::TILE_SIZE as u64)
            .checked_pow(2)
            .and_then(|v| v.checked_mul(target.bytes_per_pixel() as u64))
            .ok_or_else(|| bad("cubic tile budget overflow"))?;
        check_memory(
            source,
            &output,
            retained,
            (len as u64)
                .checked_add(tile_bytes.checked_mul(2).ok_or_else(|| bad("cubic tile budget overflow"))?)
                .ok_or_else(|| bad("cubic tile budget overflow"))?,
        )?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| bad("cubic staging allocation failed"))?;
        bytes.resize(len, 0);
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if cursor.is_multiple_of(4096) {
                    control.check().map_err(|_| EngineError::Cancelled)?;
                }
                let mut input = [0.; 4];
                source.try_read_pixel_f64(x, y, input.get_mut(..channels).ok_or_else(|| bad("cubic channel layout"))?).map_err(|e| bad(e.to_string()))?;
                if !round_generated
                    && input.get(..channels).ok_or_else(|| bad("cubic channel layout"))?.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(bad("GIMP interpolation compatibility requires finite unit source samples"));
                }
                if round_generated
                    && matches!(target.sample, SampleType::F32 | SampleType::F64)
                    && !finite_normal
                    && !(0.0..=1.0).contains(input.get(colors).ok_or_else(|| bad("cubic alpha layout"))?)
                {
                    return Err(bad("Cubic generated alpha outside the qualified finite unit coverage domain"));
                }
                let mut values = [0.; 4];
                transform
                    .try_eval_f64(
                        input.get(..colors).ok_or_else(|| bad("cubic channel layout"))?,
                        values.get_mut(..colors).ok_or_else(|| bad("cubic channel layout"))?,
                    )
                    .map_err(|e| bad(e.to_string()))?;
                *values.get_mut(colors).ok_or_else(|| bad("cubic alpha layout"))? = *input.get(colors).ok_or_else(|| bad("cubic alpha layout"))?;
                if round_generated
                    && matches!(target.sample, SampleType::F32 | SampleType::F64)
                    && values.get(..colors).ok_or_else(|| bad("cubic color layout"))?.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(bad("GIMP generated color outside the qualified finite unit domain"));
                }
                let end = cursor.checked_add(target.bytes_per_pixel()).ok_or_else(|| bad("cubic offset overflow"))?;
                let pixel = bytes.get_mut(cursor..end).ok_or_else(|| bad("cubic buffer layout"))?;
                for (channel, value) in values.get(..channels).ok_or_else(|| bad("cubic channel layout"))?.iter().enumerate() {
                    // The independent stock edit stores f32-generated encoded values, including
                    // F64 storage. This explicit compatibility stage never rewrites source bits.
                    let value = if round_generated {
                        let generated = *value as f32;
                        if !generated.is_finite() {
                            return Err(bad("Cubic generated sample exceeds observed finite precision"));
                        }
                        if half_target {
                            let bits = photocraft_color::try_round_f32_to_f16_bits(generated).map_err(|e| bad(e.to_string()))?;
                            f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(bits)))
                        } else {
                            f64::from(generated)
                        }
                    } else {
                        *value
                    };
                    photocraft_color::try_write_sample_f64(pixel, target.sample, channel, value).map_err(|e| bad(e.to_string()))?;
                }
                cursor = end;
            }
        }
        output.try_write_interleaved_with(area, &bytes, control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    }
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(output)
}
pub(super) fn warp(doc: &Document, source: &Surface, bounds: Rect, h: &Homography, linear_sampling: bool, control: &Interrupt<'_>) -> Result<Surface> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    if source.format().mode != doc.mode
        || bounds.is_empty()
        || bounds.width() > 32
        || bounds.height() > 32
        || !source.format().alpha
        || !(matches!(source.format().sample, SampleType::U32 | SampleType::F64)
            || (linear_sampling && matches!(source.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32)))
    {
        return Err(bad("GIMP interpolation compatibility requires a qualified bounded alpha raster"));
    }
    if !h.0[8].is_finite() || h.0[8].abs() < 1e-12 {
        return Err(bad("GIMP interpolation homography normalization is unsupported"));
    }
    let normalized = Homography(h.0.map(|value| value / h.0[8]));
    if normalized.0.iter().any(|value| !value.is_finite()) {
        return Err(bad("GIMP interpolation homography is not finite"));
    }
    let h = &normalized;
    let translation = h.0[0] == 1. && h.0[1] == 0. && h.0[3] == 0. && h.0[4] == 1. && h.0[6] == 0. && h.0[7] == 0. && h.0[8] == 1.;
    // Only these independently saved/reopened nontranslation matrices are admitted.
    // Rect-to-quad reconstruction differs by rounding ulps; 1e-12 is coefficient
    // reconstruction tolerance, not a broader geometry or display error budget.
    let matrices = [
        [1.25, 0., 0.25, 0., 0.75, 0.375, 0., 0., 1.],
        [0.8660254037844386, -0.5, 3., 0.5, 0.8660254037844386, 1., 0., 0., 1.],
        [1., 0.1, 1.25, -0.05, 1., 0.75, 0.02, 0.015, 1.],
    ];
    if !translation && (bounds != Rect::new(-2, 1, 6, 8) || !matrices.iter().any(|m| m.iter().zip(h.0.iter()).all(|(a, b)| (a - b).abs() <= 1e-12))) {
        return Err(bad("GIMP interpolation nontranslation geometry is not independently qualified"));
    }
    if translation && h.0[2].is_finite() && h.0[5].is_finite() && h.0[2] == h.0[2].round() && h.0[5] == h.0[5].round() {
        return photocraft_algo::transform::try_warp_nearest_surface_with_limits(source, bounds, h, Default::default(), control)
            .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) });
    }
    let fractional_member = linear_sampling
        && (doc.sample_encoding == Some(photocraft_doc::SampleEncoding::Perceptual)
            || (source.format().sample == SampleType::F32
                && doc.source_color_context.as_ref().is_some_and(|context| matches!(context.source_precision, 500 | 550))));
    let phases: &[(f64, f64)] = if fractional_member {
        &[(0.25, 0.75), (0.8, 0.2), (-0.375, -0.125)]
    } else if linear_sampling {
        &[(0.25, 0.75), (0.8, 0.2)]
    } else {
        &[(0.5, 0.375), (0.125, 0.625), (0.8, 0.2), (0.25, 0.75), (0.375, 0.125)]
    };
    if translation && !phases.contains(&(h.0[2], h.0[5])) {
        return Err(bad("GIMP interpolation translation phase is not independently qualified"));
    }
    if linear_sampling && (bounds.width() != 8 || bounds.height() != 7) {
        return Err(bad("Linear compatibility requires the independently observed 8x7 owner extent"));
    }
    let legal_linear_depth = linear_sampling && matches!(source.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32);
    if linear_sampling && source.format().sample != doc.depth {
        return Err(bad("GIMP Linear mixed document/raster precision is not independently qualified"));
    }
    if legal_linear_depth && translation && (h.0[2], h.0[5]) != (0.25, 0.75) && !fractional_member {
        return Err(bad("GIMP Linear translation at this depth is not independently qualified"));
    }
    let perceptual_target = linear_sampling && doc.sample_encoding == Some(photocraft_doc::SampleEncoding::Perceptual);
    if perceptual_target {
        let precision_matches = doc.source_color_context.as_ref().is_some_and(|context| {
            matches!(
                (source.format().sample, context.source_precision),
                (SampleType::U8, 175) | (SampleType::U16, 275) | (SampleType::U32, 375) | (SampleType::F32, 575 | 675) | (SampleType::F64, 775)
            )
        });
        if !precision_matches {
            return Err(bad("GIMP Linear Perceptual precision or translation is not independently qualified"));
        }
    }
    let half_target = legal_linear_depth
        && source.format().sample == SampleType::F32
        && doc
            .source_color_context
            .as_ref()
            .is_some_and(|context| matches!(context.source_precision, 500 | 550) || (perceptual_target && context.source_precision == 575));
    if legal_linear_depth
        && source.format().sample == SampleType::F32
        && doc.source_color_context.as_ref().is_some_and(|context| context.source_precision == 575)
        && !perceptual_target
    {
        return Err(bad("GIMP Linear Half perceptual source is not independently qualified"));
    }
    if half_target {
        // Immutable source precision cannot justify rounding current F32 edits.
        // Admit only exact current Half-representable native samples.
        for y in bounds.y0..bounds.y1 {
            control.check().map_err(|_| EngineError::Cancelled)?;
            for x in bounds.x0..bounds.x1 {
                let mut pixel = [0.; 4];
                source
                    .try_read_pixel_f64(x, y, pixel.get_mut(..source.channels()).ok_or_else(|| bad("Half channel layout"))?)
                    .map_err(|e| bad(e.to_string()))?;
                for value in pixel.get(..source.channels()).ok_or_else(|| bad("Half channel layout"))? {
                    let native = *value as f32;
                    let bits =
                        photocraft_color::try_round_f32_to_f16_bits(native).map_err(|_| bad("current half-float source is not exactly representable"))?;
                    if photocraft_color::widen_f16_to_f32_bits(bits) != native.to_bits() {
                        return Err(bad("current half-float source is not exactly representable"));
                    }
                }
            }
        }
    }
    if translation && fractional_member && bounds != Rect::new(-2, 1, 6, 8) {
        return Err(bad("GIMP Linear Half/Perceptual translation source extent is not independently qualified"));
    }
    if !(doc.sample_encoding == Some(photocraft_doc::SampleEncoding::NonlinearProfile)
        || (linear_sampling && doc.sample_encoding == Some(photocraft_doc::SampleEncoding::Linear))
        || perceptual_target)
        || !matches!(
            (&doc.working_profile_interpretation, doc.mode),
            (WorkingProfileInterpretation::ImplicitSrgb { .. }, photocraft_doc::ColorMode::Rgb)
                | (WorkingProfileInterpretation::ImplicitSGray { .. }, photocraft_doc::ColorMode::Grayscale)
        )
    {
        return Err(bad("GIMP interpolation compatibility requires a current implicit sRGB/sGray nonlinear profile"));
    }
    let profile = crate::color_cmds::try_document_profile(doc)?;
    let linear = effective_profile_from_definition(&profile, Encoding::Linear).map_err(|e| bad(e.to_string()))?;
    let decode = Transform::new(&profile, &linear, Intent::RelativeColorimetric, false).map_err(|e| bad(e.to_string()))?;
    let encode = Transform::new(&linear, &profile, Intent::RelativeColorimetric, false).map_err(|e| bad(e.to_string()))?;
    source.try_validate_tile_origins_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    let format = PixelFormat::new(source.format().mode, SampleType::F64, true);
    let original_bytes = resident(source)?;
    let working = convert(source, bounds, format, &decode, (0, false, false, false), control)?;
    let remaining = GeometryLimits::default().resident_bytes.checked_sub(original_bytes).ok_or_else(|| bad("cubic retained source exceeds resident budget"))?;
    let limits = GeometryLimits { resident_bytes: remaining, ..Default::default() };
    let output = if linear_sampling {
        photocraft_algo::transform::try_warp_bilinear_hidden_surface_with_limits(&working, bounds, h, limits, control)
    } else {
        photocraft_algo::transform::try_warp_cubic_surface_with_limits(&working, bounds, h, limits, control)
    }
    .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    drop(working);
    let extent = photocraft_algo::transform::try_warp_bounds(bounds, h).map_err(|e| bad(e.to_string()))?;
    convert(&output, extent, source.format(), &encode, (original_bytes, true, qualified_finite_output(doc, extent), half_target), control)
}

fn qualified_finite_output(doc: &Document, bounds: Rect) -> bool {
    use photocraft_doc::{LayerContent, SampleEncoding, import_compositing::*};
    doc.depth == SampleType::F64
        && doc.sample_encoding == Some(SampleEncoding::NonlinearProfile)
        && matches!(
            (&doc.working_profile_interpretation, doc.mode),
            (WorkingProfileInterpretation::ImplicitSrgb { .. }, photocraft_doc::ColorMode::Rgb)
                | (WorkingProfileInterpretation::ImplicitSGray { .. }, photocraft_doc::ColorMode::Grayscale)
        )
        && (doc.size.width, doc.size.height) == (16, 14)
        && [Rect::new(-2, 1, 7, 9), Rect::new(-3, 0, 8, 11), Rect::new(-1, 1, 7, 9)].contains(&bounds)
        && doc.layers.len() == 1
        && doc.channels.is_empty()
        && doc.selection.is_none()
        && doc.quick_mask.is_none()
        && doc.xcf_state.as_ref().is_some_and(|state| {
            state.filter_stacks.is_empty()
                && state.source_items.is_empty()
                && state.floating.is_none()
                && doc.layers.first().is_some_and(|layer| {
                    layer.visible
                        && layer.opacity.to_bits() == 1_f32.to_bits()
                        && layer.fill_opacity.to_bits() == 1_f32.to_bits()
                        && layer.mask.is_none()
                        && layer.vector_mask.is_none()
                        && matches!(layer.content, LayerContent::Raster(_))
                        && layer.surface().is_some_and(|s| s.format().sample == SampleType::F64 && s.format().alpha)
                        && state
                            .items
                            .iter()
                            .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id))
                            .and_then(|item| item.composition.as_ref())
                            .is_some_and(|policy| {
                                policy.source_mode == 28
                                    && !policy.pass_through
                                    && (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) == (0, -1, -1)
                                    && policy.resolved.as_ref().is_some_and(|r| {
                                        r.blend_space == GimpSpace::LinearRgb && r.composite_space == GimpSpace::LinearRgb && r.operator == GimpOperator::Union
                                    })
                            })
                })
        })
}

pub(super) fn stamp_finite_normal_if_needed(doc: &mut Document, id: photocraft_doc::LayerId, bounds: Rect, control: &Interrupt<'_>) -> Result<()> {
    let surface = doc.layer(id).and_then(|layer| layer.surface()).ok_or_else(|| bad("missing generated raster"))?;
    if surface.format().sample != SampleType::F64 {
        return Ok(());
    }
    let channels = surface.channels();
    let mut nonunit = false;
    for y in bounds.y0..bounds.y1 {
        control.check().map_err(|_| EngineError::Cancelled)?;
        for x in bounds.x0..bounds.x1 {
            let mut pixel = [0.; 4];
            surface.try_read_pixel_f64(x, y, pixel.get_mut(..channels).ok_or_else(|| bad("generated channels"))?).map_err(|e| bad(e.to_string()))?;
            let alpha = *pixel.get(channels.checked_sub(1).ok_or_else(|| bad("generated alpha"))?).ok_or_else(|| bad("generated alpha"))?;
            if !alpha.is_finite() {
                return Err(bad("generated alpha is nonfinite"));
            }
            nonunit |= !(0.0..=1.0).contains(&alpha);
        }
    }
    if !nonunit {
        return Ok(());
    }
    if !qualified_finite_output(doc, bounds) {
        return Err(bad("Cubic generated alpha outside the qualified finite Normal context"));
    }
    let policy = doc
        .xcf_state
        .as_mut()
        .and_then(|state| state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(id)))
        .and_then(|item| item.composition.as_mut())
        .ok_or_else(|| bad("missing finite Normal policy"))?;
    policy.compatibility_epoch = photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH.into();
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    Ok(())
}
