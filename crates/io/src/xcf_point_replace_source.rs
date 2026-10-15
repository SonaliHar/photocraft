//! Measured singleton Replace compatibility with opaque stored fields preserved.
use crate::xcf_import::plane::ExactParsedPlane;
use crate::{
    IoError,
    xcf_filters::{NeighborhoodDomain, PointReplaceFamily},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{ArgumentValue, Encoding, Model, Sample, XcfFile};

#[cfg(all(test, feature = "corpus"))]
#[path = "xcf_alpha_clip_finite_source_tests.rs"]
mod finite_source_tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "xcf_component_geometry_source_tests.rs"]
mod component_geometry_source_tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "xcf_color_overlay_source_tests.rs"]
mod color_overlay_source_tests;

#[path = "xcf_absolute_source.rs"]
pub(crate) mod absolute_source;
#[path = "xcf_channel_mixer_source.rs"]
pub(crate) mod channel_mixer_source;
#[path = "xcf_color_rotate_source.rs"]
pub(crate) mod color_rotate_source;
#[path = "xcf_mono_mixer_source.rs"]
pub(crate) mod mono_mixer_source;
#[path = "xcf_red_eye_source.rs"]
pub(crate) mod red_eye_source;
#[path = "xcf_video_degradation_source.rs"]
pub(crate) mod video_degradation_source;

/// Exact source argument validation without allocating a mapped filter stack.
fn finite_alpha_operation<P>(effect: &photocraft_xcf::Effect<P>) -> Option<photocraft_algo::gimp::alpha_clip_finite::AlphaClipFinite> {
    if effect.version.as_deref() != Some("0:0") || effect.arguments.len() != 4 {
        return None;
    }
    let (mut clip_low, mut clip_high, mut low_limit, mut high_limit) = (None, None, None, None);
    for argument in &effect.arguments {
        if argument.configuration.is_some() || argument.graph.is_some() {
            return None;
        }
        match (argument.name.as_str(), &argument.value) {
            ("clip-low", ArgumentValue::Boolean(v)) if clip_low.is_none() => clip_low = Some(*v),
            ("clip-high", ArgumentValue::Boolean(v)) if clip_high.is_none() => clip_high = Some(*v),
            ("low-limit", ArgumentValue::Float32Bits(v)) if low_limit.is_none() => low_limit = Some(f32::from_bits(*v)),
            ("high-limit", ArgumentValue::Float32Bits(v)) if high_limit.is_none() => high_limit = Some(f32::from_bits(*v)),
            _ => return None,
        }
    }
    let operation = photocraft_algo::gimp::alpha_clip_finite::AlphaClipFinite {
        clip_low: clip_low?,
        low_limit: low_limit?,
        clip_high: clip_high?,
        high_limit: high_limit?,
    };
    operation.validate().ok()?;
    Some(operation)
}

fn poll(control: &Interrupt<'_>, shared: Option<&photocraft_xcf::Budget<'_>>, bytes: usize) -> Result<(), IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    if let Some(shared) = shared {
        shared
            .consume_work(bytes as u64, "point Replace source qualification")
            .map_err(|error| if error.kind == photocraft_xcf::ErrorKind::Cancelled { IoError::Cancelled } else { IoError::Xcf(error) })?;
    }
    Ok(())
}

/// Bounded route hint only; it never supplies whole-file source authority.
/// The selector and classifier both cap the flat original argument list before
/// invoking the exact existing family discriminator. Full classify + canonical
/// qualified_stack remain mandatory before any successful selected completion.
pub(crate) fn classify_family_hint<P>(file: &XcfFile<P>) -> Option<PointReplaceFamily> {
    let owner = file.layers.first()?;
    let effect = owner.effects.first()?;
    if effect.arguments.len() > 10 {
        return None;
    }
    Some(match effect.operation.as_str() {
        "gegl:video-degradation" if video_degradation_source::observed_epoch(effect).is_some() => PointReplaceFamily::VideoDegradation,
        "gegl:absolute" if absolute_source::observed_envelope(effect) && file.encoding == Encoding::Linear => PointReplaceFamily::AbsoluteLinear,
        "gegl:invert-linear" => PointReplaceFamily::InvertLinear,
        "gegl:invert-gamma" => PointReplaceFamily::InvertGamma,
        "gegl:color-exchange" => PointReplaceFamily::ColorExchange,
        "gegl:channel-mixer" if channel_mixer_source::observed_envelope(effect) => PointReplaceFamily::ChannelMixer,
        "gegl:mono-mixer" if mono_mixer_source::observed_envelope(effect) => PointReplaceFamily::MonoMixerSigned,
        "gegl:color-rotate" if color_rotate_source::observed_envelope(effect) => PointReplaceFamily::ColorRotate,
        "gegl:red-eye-removal" if red_eye_source::observed_epoch(effect).is_some() => PointReplaceFamily::RedEye,
        "gegl:color-overlay" => PointReplaceFamily::ColorOverlay,
        "gegl:color-to-alpha" => PointReplaceFamily::ColorToAlpha,
        "gegl:component-extract" => match effect.arguments.iter().find(|argument| argument.name == "component").map(|argument| &argument.value) {
            Some(ArgumentValue::Enum(0 | 1 | 2 | 8..=14 | 20))
                if (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode)
                    == (Some(87327313), Some(87327313), Some(87327313)) =>
            {
                PointReplaceFamily::ComponentExtractGeometry
            }
            Some(ArgumentValue::Enum(0 | 1 | 2 | 20)) => PointReplaceFamily::ComponentExtract,
            Some(ArgumentValue::Enum(8..=11)) => PointReplaceFamily::ComponentExtractCmyk,
            Some(ArgumentValue::Enum(12..=14)) => PointReplaceFamily::ComponentExtractYcbcr,
            _ => return None,
        },
        "gegl:alpha-clip" => {
            match (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode) {
                (Some(-305954223), Some(-305954223), Some(-305954223)) if matches!(file.sample, Sample::F32 | Sample::F64) => {
                    PointReplaceFamily::AlphaClipFinite
                }
                (Some(-1710980527), Some(-1710980527), Some(-1710980527)) => PointReplaceFamily::AlphaClip,
                // A failed newer envelope must never gain another epoch's authority.
                _ => return None,
            }
        }
        "gegl:brightness-contrast" => PointReplaceFamily::Brightness,
        "gegl:levels" => PointReplaceFamily::Levels,
        "gegl:exposure" => PointReplaceFamily::Exposure,
        "gegl:color-temperature" => PointReplaceFamily::Temperature,
        "gegl:sepia" => PointReplaceFamily::Sepia,
        "gegl:alien-map" => match effect.arguments.iter().find(|argument| argument.name == "color-model").map(|argument| &argument.value) {
            Some(ArgumentValue::Enum(0)) => PointReplaceFamily::AlienRgb,
            Some(ArgumentValue::Enum(1)) => PointReplaceFamily::AlienHsl,
            _ => return None,
        },
        _ => return None,
    })
}

/// No allocation, profile substitution, enum normalization or source recoding.
/// Typed parameters are subsequently checked by the unchanged base mapper.
pub(crate) fn classify<P: ExactParsedPlane>(
    file: &XcfFile<P>,
    control: &Interrupt<'_>,
    shared: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<Option<NeighborhoodDomain>, IoError> {
    poll(control, shared, 0)?;
    let Some(owner) = file.layers.first() else { return Ok(None) };
    let Some(effect) = owner.effects.first() else { return Ok(None) };
    let Some(family) = classify_family_hint(file) else { return Ok(None) };
    if matches!(family, PointReplaceFamily::ComponentExtractCmyk | PointReplaceFamily::ComponentExtractYcbcr)
        && (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode) != (Some(-305954223), Some(-305954223), Some(-305954223))
    {
        return Ok(None);
    }
    let color_rotate = family == PointReplaceFamily::ColorRotate;
    let red_eye_epoch = if family == PointReplaceFamily::RedEye { red_eye_source::observed_epoch(effect) } else { None };
    let red_eye_geometry = red_eye_epoch == Some(photocraft_compose::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH);
    if red_eye_geometry && (file.model != Model::Rgb || file.precision != 650 || file.sample != Sample::F32 || file.encoding != Encoding::NonlinearProfile) {
        return Ok(None);
    }
    let video = family == PointReplaceFamily::VideoDegradation;
    if video && (file.model != Model::Rgb || file.precision != 650 || file.sample != Sample::F32 || file.encoding != Encoding::NonlinearProfile) {
        return Ok(None);
    }
    if color_rotate && (file.precision != 675 || file.sample != Sample::F32 || file.encoding != Encoding::Perceptual) {
        return Ok(None);
    }
    let absolute = family == PointReplaceFamily::AbsoluteLinear;
    let channel_mixer = family == PointReplaceFamily::ChannelMixer;
    let mono_signed = family == PointReplaceFamily::MonoMixerSigned;
    let component_geometry = family == PointReplaceFamily::ComponentExtractGeometry;
    let overlay = if family == PointReplaceFamily::ColorOverlay {
        let Some(parameters) = crate::xcf_filters::color_overlay_parameters(effect) else { return Ok(None) };
        if (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode) != (Some(1435337297), Some(1435337297), Some(1435337297)) {
            return Ok(None);
        }
        Some(parameters)
    } else {
        None
    };
    if component_geometry && (file.sample != Sample::F32 || file.precision != 650 || file.encoding != Encoding::NonlinearProfile) {
        return Ok(None);
    }
    // Current canonical Half storage is exact F32 widening. A newly generated
    // Half coverage buffer needs a mutable quantization authority; immutable
    // source precision cannot supply it after edits. Keep this family closed.
    let width = match (file.precision, file.sample, file.encoding) {
        (100, Sample::U8, Encoding::Linear) | (150, Sample::U8, Encoding::NonlinearProfile) | (175, Sample::U8, Encoding::Perceptual) => 1,
        (200, Sample::U16, Encoding::Linear) | (250, Sample::U16, Encoding::NonlinearProfile) | (275, Sample::U16, Encoding::Perceptual) => 2,
        (300, Sample::U32, Encoding::Linear) | (350, Sample::U32, Encoding::NonlinearProfile) | (375, Sample::U32, Encoding::Perceptual) => 4,
        // The decoder widens original Half samples exactly; canonical storage
        // is F32 and must not be reinterpreted as two-byte normalized integers.
        (500, Sample::F16, Encoding::Linear) | (550, Sample::F16, Encoding::NonlinearProfile) | (575, Sample::F16, Encoding::Perceptual) => 4,
        (600, Sample::F32, Encoding::Linear) | (650, Sample::F32, Encoding::NonlinearProfile) | (675, Sample::F32, Encoding::Perceptual) => 4,
        (700, Sample::F64, Encoding::Linear) | (750, Sample::F64, Encoding::NonlinearProfile) | (775, Sample::F64, Encoding::Perceptual) => 8,
        _ => return Ok(None),
    };
    let canonical_sample = if file.sample == Sample::F16 { Sample::F32 } else { file.sample };
    let channels = match file.model {
        Model::Rgb => 4,
        Model::Gray if !matches!(family, PointReplaceFamily::Sepia | PointReplaceFamily::AlienRgb | PointReplaceFamily::AlienHsl) => 2,
        _ => return Ok(None),
    };
    let bounds_valid = |width: u32, height: u32, offset: (i32, i32)| {
        width > 0
            && height > 0
            && width <= 262144
            && height <= 262144
            && [i64::from(offset.0), i64::from(offset.1), i64::from(offset.0) + i64::from(width), i64::from(offset.1) + i64::from(height)]
                .iter()
                .all(|value| (-(1i64 << 29)..=(1i64 << 29)).contains(value))
    };
    let source = &effect.state;
    // Public Auto getters report zero, but original stored properties remain
    // 0/-1/-1. Preserve those bytes; never alias the public and stored tuples.
    let component = matches!(
        family,
        PointReplaceFamily::ComponentExtract
            | PointReplaceFamily::ComponentExtractCmyk
            | PointReplaceFamily::ComponentExtractYcbcr
            | PointReplaceFamily::ComponentExtractGeometry
    );
    let alpha_clip = matches!(family, PointReplaceFamily::AlphaClip | PointReplaceFamily::AlphaClipFinite);
    let finite_alpha = if family == PointReplaceFamily::AlphaClipFinite {
        poll(control, shared, 4)?;
        let Some(operation) = finite_alpha_operation(effect) else { return Ok(None) };
        Some(operation)
    } else {
        None
    };
    let owner_tuple = (owner.blend_space, owner.composite_space, owner.composite_mode);
    let owner_policy = if video {
        file.width == 16 && file.height == 16 && owner.width == 16 && owner.height == 16 && owner.offset == (0, 0) && owner_tuple == (0, -1, -1)
    } else if color_rotate {
        file.width == 32 && file.height == 16 && owner.width == 32 && owner.height == 16 && owner.offset == (0, 0) && owner_tuple == (0, -1, -1)
    } else if component_geometry {
        matches!(((file.width, file.height), (owner.width, owner.height), owner.offset), ((37, 29), (19, 13), (-7, 5)) | ((259, 17), (262, 9), (-3, -4)))
            && owner_tuple == (0, -1, -1)
    } else if let Some(epoch) = red_eye_epoch {
        photocraft_compose::gimp_red_eye::qualified_owner_geometry(epoch, (file.width, file.height), (owner.width, owner.height), owner.offset)
            && owner_tuple == (0, -1, -1)
    } else if absolute || channel_mixer || mono_signed || component || alpha_clip || overlay.is_some() {
        file.width == 8 && file.height == 6 && owner.width == 8 && owner.height == 6 && owner.offset == (0, 0) && owner_tuple == (0, -1, -1)
    } else {
        owner_tuple == (0, -1, -1)
    };
    if file.version != 22
        || !file.icc.is_empty()
        || file.layers.len() != 1
        || !file.channels.is_empty()
        || !file.paths.is_empty()
        || !bounds_valid(file.width, file.height, (0, 0))
        || !bounds_valid(owner.width, owner.height, owner.offset)
        || owner.group
        || owner.path.len() > 1
        || !owner.visible
        || !owner.alpha
        || owner.model != file.model
        || owner.mode != 28
        || !owner_policy
        || owner.opacity.to_bits() != 1f32.to_bits()
        || owner.mask.is_some()
        || owner.text.is_some()
        || owner.vector.is_some()
        || owner.link.is_some()
        || owner.transform.is_some()
        || owner.floating_attachment.is_some()
        || owner.effects.len() != 1
        || source.mode != Some(62)
        || source.opacity_bits != Some(1f32.to_bits())
        || source.opacity_u8.is_some()
        || source.clipping != Some(0)
        || source.region != Some(0)
        || source.enabled.is_none()
        || source.blend_space.is_none()
        || source.composite_space.is_none()
        || source.composite_mode.is_none()
    {
        return Ok(None);
    }
    let Some(plane) = owner.plane.as_ref() else { return Ok(None) };
    let owner_bytes = u64::from(owner.width).checked_mul(u64::from(owner.height)).and_then(|v| v.checked_mul(channels as u64 * width as u64));
    let descriptor = plane.descriptor();
    if descriptor.sample != canonical_sample
        || descriptor.width != owner.width
        || descriptor.height != owner.height
        || descriptor.channels != channels
        || owner_bytes.is_none()
        || owner_bytes != plane.native_byte_len()
    {
        return Ok(None);
    }
    let typed_color = matches!(
        family,
        PointReplaceFamily::ColorExchange
            | PointReplaceFamily::AbsoluteLinear
            | PointReplaceFamily::RedEye
            | PointReplaceFamily::ChannelMixer
            | PointReplaceFamily::MonoMixerSigned
            | PointReplaceFamily::ColorOverlay
            | PointReplaceFamily::ColorToAlpha
            | PointReplaceFamily::ComponentExtract
            | PointReplaceFamily::ComponentExtractCmyk
            | PointReplaceFamily::ComponentExtractYcbcr
            | PointReplaceFamily::ComponentExtractGeometry
            | PointReplaceFamily::AlphaClip
            | PointReplaceFamily::AlphaClipFinite
    );
    let mut admitted = true;
    let mut visited = 0u64;
    plane.visit_native(control, &mut |chunk| {
        visited = visited.checked_add(chunk.len() as u64).ok_or_else(|| IoError::Unsupported("point Replace native length overflow".into()))?;
        // The visitor may finish its bounded traversal after a predicate has
        // refused a previous block. Every actual read still pays shared work.
        poll(control, shared, chunk.len())?;
        if chunk.len() % (channels * width) != 0 || chunk.len() > 64 * channels * width {
            admitted = false;
        }
        if !admitted {
            return Ok(());
        }
        let validate = || -> Result<bool, IoError> {
            if typed_color {
                let profile = if file.model == Model::Gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
                let encoding = match file.encoding {
                    Encoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
                    Encoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    Encoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
                };
                for pixel in chunk.chunks_exact(channels * width) {
                    let mut values = [0f64; 4];
                    for (target, value) in values.iter_mut().zip(pixel.chunks_exact(width)) {
                        *target = match canonical_sample {
                            Sample::U8 => f64::from(*value.first().ok_or_else(|| IoError::Unsupported("Color Exchange native sample".into()))?) / 255.,
                            Sample::U16 => {
                                f64::from(u16::from_ne_bytes(value.try_into().map_err(|_| IoError::Unsupported("Color Exchange native sample".into()))?))
                                    / 65535.
                            }
                            Sample::U32 => {
                                f64::from(u32::from_ne_bytes(value.try_into().map_err(|_| IoError::Unsupported("Color Exchange native sample".into()))?))
                                    / 4294967295.
                            }
                            Sample::F32 => {
                                f64::from(f32::from_ne_bytes(value.try_into().map_err(|_| IoError::Unsupported("Color Exchange native sample".into()))?))
                            }
                            Sample::F64 => f64::from_ne_bytes(value.try_into().map_err(|_| IoError::Unsupported("Color Exchange native sample".into()))?),
                            Sample::F16 => return Ok(false),
                        };
                    }
                    let color_count = channels - 1;
                    if values.iter().take(channels).any(|value| !value.is_finite()) || !values.get(color_count).is_some_and(|alpha| (0.0..=1.0).contains(alpha))
                    {
                        return Ok(false);
                    }
                    if file.sample == Sample::F16
                        && values.iter().take(channels).any(|value| {
                            let value = *value as f32;
                            !photocraft_color::try_round_f32_to_f16_bits(value)
                                .is_ok_and(|bits| photocraft_color::widen_f16_to_f32_bits(bits) == value.to_bits())
                        })
                    {
                        return Ok(false);
                    }
                    let mut working = [0f64; 3];
                    poll(control, shared, 128)?;
                    let working_encoding = if absolute
                        || channel_mixer
                        || mono_signed
                        || overlay.is_some_and(|(_, srgb)| !srgb)
                        || alpha_clip && (file.model == Model::Gray || file.encoding != Encoding::NonlinearProfile)
                    {
                        photocraft_cms::xcf_color::Encoding::Linear
                    } else {
                        photocraft_cms::xcf_color::Encoding::NonlinearProfile
                    };
                    photocraft_cms::xcf_color::convert_encoding_f64(
                        profile,
                        encoding,
                        working_encoding,
                        values.get(..color_count).ok_or_else(|| IoError::Unsupported("Color Exchange source channels".into()))?,
                        working.get_mut(..color_count).ok_or_else(|| IoError::Unsupported("Color Exchange working channels".into()))?,
                    )
                    .map_err(|_| IoError::Unsupported("Color Exchange source transfer".into()))?;
                    if working.iter().take(color_count).any(|value| {
                        let value = *value as f32;
                        !value.is_finite() || (!absolute && !(0.0..=1.0).contains(&value))
                    }) {
                        return Ok(false);
                    }
                    if channel_mixer && source.enabled == Some(true) {
                        let Some(operation) = channel_mixer_source::parameters(effect) else { return Ok(false) };
                        let red = *working.first().ok_or_else(|| IoError::Unsupported("Channel Mixer working color".into()))?;
                        let alpha = *values.get(color_count).ok_or_else(|| IoError::Unsupported("Channel Mixer alpha".into()))?;
                        let input = if file.model == Model::Gray {
                            [red as f32, red as f32, red as f32, alpha as f32]
                        } else {
                            [
                                red as f32,
                                *working.get(1).ok_or_else(|| IoError::Unsupported("Channel Mixer green".into()))? as f32,
                                *working.get(2).ok_or_else(|| IoError::Unsupported("Channel Mixer blue".into()))? as f32,
                                alpha as f32,
                            ]
                        };
                        poll(control, shared, 96)?;
                        if operation.try_eval_linear_float(&input, &mut [0.; 4]).is_err() {
                            return Ok(false);
                        }
                    }
                    if mono_signed {
                        let Some(operation) = mono_mixer_source::parameters(effect) else { return Ok(false) };
                        let red = *working.first().ok_or_else(|| IoError::Unsupported("Mono Mixer working red".into()))?;
                        let input =
                            if file.model == Model::Gray { [red, red, red, values[color_count]] } else { [red, working[1], working[2], values[color_count]] };
                        let mut output = [0.; 4];
                        poll(control, shared, 96)?;
                        if let Err(error) = operation.try_eval_pixel_with(&input, &mut output, source.enabled == Some(true), control, &|_| Ok(())) {
                            return if error == photocraft_raster::RasterInputError::Cancelled { Err(IoError::Cancelled) } else { Ok(false) };
                        }
                    }
                    if let Some(operation) = finite_alpha {
                        // Admission includes the current Normal owner's unit-alpha
                        // postcondition. Finite thresholds may otherwise create
                        // coverage which this ordinary document domain cannot retain.
                        let red = *working.first().ok_or_else(|| IoError::Unsupported("Alpha Clip working color".into()))?;
                        let input = if file.model == Model::Gray {
                            [red, red, red, *values.get(color_count).ok_or_else(|| IoError::Unsupported("Alpha Clip source alpha".into()))?]
                        } else {
                            [
                                red,
                                *working.get(1).ok_or_else(|| IoError::Unsupported("Alpha Clip working green".into()))?,
                                *working.get(2).ok_or_else(|| IoError::Unsupported("Alpha Clip working blue".into()))?,
                                *values.get(color_count).ok_or_else(|| IoError::Unsupported("Alpha Clip source alpha".into()))?,
                            ]
                        };
                        let mut output = [0.; 4];
                        // Prepay the kernel's eight scalar sample units on the shared
                        // source ledger, retaining the caller's cancellation checks.
                        poll(control, shared, 64)?;
                        if let Err(error) = operation.try_eval_pixel_with(&input, &mut output, source.enabled == Some(true), control, &|_| Ok(())) {
                            return if error == photocraft_raster::RasterInputError::Cancelled { Err(IoError::Cancelled) } else { Ok(false) };
                        }
                        if !output.get(3).is_some_and(|alpha| alpha.is_finite() && (0.0..=1.0).contains(alpha)) {
                            return Ok(false);
                        }
                    }
                }
                return Ok(true);
            }
            for value in chunk.chunks_exact(width) {
                let finite_unit = match file.sample {
                    Sample::U8 => true,
                    Sample::U16 => true,
                    Sample::U32 => true,
                    Sample::F16 => <[u8; 4]>::try_from(value).is_ok_and(|raw| {
                        let v = f32::from_ne_bytes(raw);
                        v.is_finite()
                            && (0.0..=1.0).contains(&v)
                            && photocraft_color::try_round_f32_to_f16_bits(v).is_ok_and(|bits| photocraft_color::widen_f16_to_f32_bits(bits) == v.to_bits())
                    }),
                    Sample::F32 => <[u8; 4]>::try_from(value).is_ok_and(|raw| {
                        let v = f32::from_ne_bytes(raw);
                        v.is_finite() && (0.0..=1.0).contains(&v)
                    }),
                    Sample::F64 => <[u8; 8]>::try_from(value).is_ok_and(|raw| {
                        let v = f64::from_ne_bytes(raw);
                        v.is_finite() && (0.0..=1.0).contains(&v)
                    }),
                };
                if !finite_unit {
                    return Ok(false);
                }
            }

            Ok(true)
        };
        admitted = validate()?;
        Ok(())
    })?;
    if !admitted || Some(visited) != owner_bytes {
        return Ok(None);
    }
    let Some(mask) = effect.mask.as_ref() else { return Ok(None) };
    if red_eye_geometry
        && (owner.text_flags != 0
            || owner.group_flags != 0
            || mask.group_flags != 0
            || owner.unsupported.iter().any(|reason| reason != "GEGL layer effects")
            || !mask.path.is_empty()
            || mask.mask.is_some()
            || mask.text.is_some()
            || mask.text_flags != 0
            || mask.vector.is_some()
            || mask.link.is_some()
            || mask.transform.is_some()
            || mask.floating_attachment.is_some()
            || !matches!(mask.opacity.to_bits(), 0x3f000000 | 0x3f008081))
    {
        return Ok(None);
    }
    if video
        && video_degradation_source::observed_epoch(effect) == Some(photocraft_compose::gimp_video_degradation::VIDEO_DEGRADATION_OPTIONS_EPOCH)
        && (!mask.path.is_empty()
            || mask.mask.is_some()
            || mask.text.is_some()
            || mask.text_flags != 0
            || mask.vector.is_some()
            || mask.link.is_some()
            || mask.transform.is_some()
            || mask.floating_attachment.is_some())
    {
        return Ok(None);
    }
    let Some(plane) = mask.plane.as_ref() else { return Ok(None) };
    let expected = u64::from(file.width).checked_mul(u64::from(file.height)).and_then(|v| v.checked_mul(width as u64));
    let descriptor = plane.descriptor();
    if mask.group
        || mask.alpha
        || mask.model != Model::Gray
        || !mask.visible
        || mask.offset != (0, 0)
        || !mask.mask_enabled
        || mask.mask_shown
        || mask.mask_editing
        || mask.legacy_linked
        || mask.mode != 0
        || (mask.blend_space, mask.composite_space, mask.composite_mode) != (2, 2, 1)
        || (mask.width, mask.height) != (file.width, file.height)
        || !mask.effects.is_empty()
        || !mask.unsupported.is_empty()
        || descriptor.sample != canonical_sample
        || descriptor.channels != 1
        || (descriptor.width, descriptor.height) != (file.width, file.height)
        || expected.is_none()
        || expected != plane.native_byte_len()
    {
        return Ok(None);
    }
    let mut zero = true;
    let mut visited = 0u64;
    plane.visit_native(control, &mut |chunk| {
        poll(control, shared, chunk.len())?;
        visited = visited.checked_add(chunk.len() as u64).ok_or_else(|| IoError::Unsupported("point Replace mask length overflow".into()))?;
        zero &= chunk.len() % width == 0 && chunk.len() <= 64 * width && chunk.iter().all(|byte| *byte == 0);
        Ok(())
    })?;
    if !zero || Some(visited) != expected {
        return Ok(None);
    }
    Ok(Some(NeighborhoodDomain::PointReplace(family)))
}

#[cfg(all(test, feature = "corpus"))]
mod tests {
    #[test]
    #[ignore = "local unpublished 1260 independently saved CMYK and YCbCr source classifications"]
    fn expanded_component_source_families_are_distinct_from_rgb_alpha() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let mut count = 0;
        for (folder, expected_count, expected_manifest) in [
            ("point-component-extract-cmyk-evidence", 720, "ed5791616c890aeed174d57b086a104e79179b098a62e4335c7b86236bd9d163"),
            ("point-component-extract-ycbcr-evidence", 540, "6bb9b502dd09dcffa69d717f871bc91678acfa26fea1e3c345176722112d4623"),
        ] {
            let packet = root.join(folder);
            let manifest = std::fs::read(packet.join("batch-manifest.json")).unwrap();
            let hex = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            assert_eq!(hex(&manifest), expected_manifest);
            let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
            let cases = manifest["cases"].as_array().unwrap();
            assert_eq!(cases.len(), expected_count);
            for row in cases {
                let bytes = std::fs::read(packet.join(row["source"].as_str().unwrap())).unwrap();
                assert_eq!(hex(&bytes), row["sha256"].as_str().unwrap());
                let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
                let domain = super::classify(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
                let family = if folder == "point-component-extract-cmyk-evidence" {
                    crate::xcf_filters::PointReplaceFamily::ComponentExtractCmyk
                } else {
                    crate::xcf_filters::PointReplaceFamily::ComponentExtractYcbcr
                };
                assert!(domain == Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(family)), "{}", row["id"]);
                count += 1;
            }
        }
        assert_eq!(count, 1260);
    }

    #[test]
    #[ignore = "local unpublished expanded Component source envelope failure controls"]
    fn expanded_component_source_envelope_rejects_unqualified_contexts() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        for (folder, components) in [
            ("point-component-extract-cmyk-evidence", &["cmyk-c", "cmyk-m", "cmyk-y", "cmyk-k"][..]),
            ("point-component-extract-ycbcr-evidence", &["ycbcr-y", "ycbcr-cb", "ycbcr-cr"][..]),
        ] {
            for component in components {
                let selector = component;
                let source = root.join(folder).join(format!("states/rgb.float_non_linear.{selector}.source.xcf"));
                let bytes = std::fs::read(&source).unwrap();
                let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
                assert!(classify(&file, &Interrupt::NONE, None).unwrap().is_some());
                for mutation in 0..19 {
                    let mut hostile = file.clone();
                    match mutation {
                        0 => hostile.width = 9,
                        1 => hostile.layers[0].width = 9,
                        2 => hostile.layers[0].offset = (1, 0),
                        3 => hostile.layers[0].blend_space = 2,
                        4 => hostile.layers[0].opacity = 0.5,
                        5 => hostile.layers[0].mode = 30,
                        6 => hostile.layers[0].group = true,
                        7 => hostile.icc = vec![1],
                        8 => hostile.layers[0].effects[0].state.region = Some(1),
                        9 => hostile.layers[0].effects[0].state.enabled = None,
                        10 => hostile.layers[0].plane.as_mut().unwrap().sample = Sample::U16,
                        11 => hostile.precision = 750,
                        12 => hostile.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes()),
                        13 => hostile.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.5f32.to_ne_bytes()),
                        14 => hostile.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                        15 => hostile.layers[0].effects[0].arguments.iter_mut().find(|a| a.name == "component").unwrap().value = ArgumentValue::Enum(15),
                        16 => hostile.layers[0].effects[0].state.blend_space = Some(0),
                        17 => hostile.layers[0].effects[0].state.composite_space = Some(2),
                        _ => hostile.layers[0].effects[0].state.composite_mode = Some(-466517423),
                    }
                    assert!(classify(&hostile, &Interrupt::NONE, None).unwrap().is_none(), "{selector} mutation{mutation}");
                }
                let cancelled = || true;
                assert!(matches!(classify(&file, &Interrupt::cancel_only(&cancelled), None), Err(IoError::Cancelled)));
                let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
                let budget = photocraft_xcf::Budget::new(limits, None);
                assert!(matches!(classify(&file, &Interrupt::NONE, Some(&budget)), Err(IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit));
                assert_eq!(std::fs::read(source).unwrap(), bytes);
            }
        }
    }

    #[test]
    #[ignore = "requires reviewed unpublished finite Alpha Clip original"]
    fn alpha_source_tuple_rejection_cannot_downgrade_to_unit_epoch() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let path = root.join("point-alpha-clip-value-domain-nonlinear-controls/rgb-float_non_linear.unit.ordinary.xcf");
        let bytes = std::fs::read(&path).unwrap();
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(classify(&original, &Interrupt::NONE, None).unwrap() == Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::AlphaClipFinite)));
        for field in 0..3 {
            for value in [-1710980527, 0, 1, 2, 3, 4, i32::MAX] {
                let mut changed = original.clone();
                let state = &mut changed.layers[0].effects[0].state;
                match field {
                    0 => state.blend_space = Some(value),
                    1 => state.composite_space = Some(value),
                    _ => state.composite_mode = Some(value),
                }
                assert!(classify(&changed, &Interrupt::NONE, None).unwrap().is_none(), "field{field}={value} must not downgrade to unit epoch");
                assert_eq!(changed.layers[0].plane.as_ref().unwrap().bytes, original.layers[0].plane.as_ref().unwrap().bytes);
            }
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    #[ignore = "local unpublished Component Extract original source admission controls"]
    fn component_source_classification_matches_originals() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/component-extract-precision-private");
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        for row in observations["records"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let effect = &file.layers[0].effects[0];
            let mask = effect.mask.as_ref().unwrap();
            let plane = mask.plane.as_ref().unwrap();
            println!(
                "{} owner={:?} mask={}x{}@{:?} mode={} tuple={}/{}/{} flags={}/{}/{}/{} sample={:?} bytes={} zero={} unsupported={:?}",
                row["id"],
                (file.layers[0].blend_space, file.layers[0].composite_space, file.layers[0].composite_mode),
                mask.width,
                mask.height,
                mask.offset,
                mask.mode,
                mask.blend_space,
                mask.composite_space,
                mask.composite_mode,
                mask.visible,
                mask.mask_enabled,
                mask.mask_shown,
                mask.mask_editing,
                plane.sample,
                plane.bytes.len(),
                plane.bytes.iter().all(|byte| *byte == 0),
                mask.unsupported
            );
            assert!(
                matches!(
                    super::classify(&file, &photocraft_raster::Interrupt::NONE, None).unwrap(),
                    Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::ComponentExtract))
                ),
                "{}",
                row["id"]
            );
        }
    }
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    #[ignore = "local unpublished 144 independently saved typed literal Color Exchange originals"]
    fn color_exchange_source_working_boundary_preserves_native_and_refuses_hostile_samples() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-exchange-evidence");
        let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        assert_eq!(observations["records"].as_array().unwrap().len(), 144);
        for row in observations["records"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let before = file.layers[0].plane.as_ref().unwrap().bytes.clone();
            assert!(
                matches!(classify(&file, &Interrupt::NONE, None).unwrap(), Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::ColorExchange))),
                "{}",
                row["id"]
            );
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, before);
            assert!(matches!(classify(&file, &Interrupt::cancel_only(&|| true), None), Err(IoError::Cancelled)));
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
            let budget = photocraft_xcf::Budget::new(limits, None);
            assert!(matches!(classify(&file, &Interrupt::NONE, Some(&budget)), Err(IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit));
            assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, before);
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e30, -1e30] {
                if file.sample != Sample::F64 {
                    continue;
                }
                let mut altered = file.clone();
                altered.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&bad.to_ne_bytes());
                assert!(classify(&altered, &Interrupt::NONE, None).unwrap().is_none());
                assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, before);
            }
            if file.sample == Sample::F64 && file.encoding == Encoding::NonlinearProfile {
                for (color, accepted) in [(1. + 2f64.powi(-25), true), (1. + 2f64.powi(-23), false), (-2f64.powi(-150), true), (-2f64.powi(-149), false)] {
                    let mut altered = file.clone();
                    altered.layers[0].plane.as_mut().unwrap().bytes[..8].copy_from_slice(&color.to_ne_bytes());
                    assert_eq!(classify(&altered, &Interrupt::NONE, None).unwrap().is_some(), accepted, "working Float32 unit boundary {color}");
                    assert_eq!(altered.layers[0].plane.as_ref().unwrap().bytes[..8], color.to_ne_bytes());
                }
                let mut altered = file.clone();
                let alpha_offset = if file.model == Model::Gray { 8 } else { 24 };
                altered.layers[0].plane.as_mut().unwrap().bytes[alpha_offset..alpha_offset + 8].copy_from_slice(&(1. + 2f64.powi(-25)).to_ne_bytes());
                assert!(classify(&altered, &Interrupt::NONE, None).unwrap().is_none(), "alpha never inherits color working Float32 narrowing");
            }
        }
    }
    #[test]
    #[ignore = "local unpublished invert-linear six-precision three-transfer source qualification"]
    fn invert_linear_source_contract_cancel_budget_and_structural_controls() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-invert-linear-evidence");
        for precision in ["u8", "u16", "u32", "half", "float", "double"] {
            for encoding in ["linear", "non_linear", "perceptual"] {
                let bytes = std::fs::read(root.join(format!("invert-linear-rgb-{precision}_{encoding}.small.source.xcf"))).unwrap();
                let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
                assert!(matches!(classify(&file, &Interrupt::NONE, None).unwrap(), Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::InvertLinear))));
                for mutation in 0..6 {
                    let mut changed = file.clone();
                    match mutation {
                        0 => changed.precision = changed.precision.saturating_add(1),
                        1 => changed.layers[0].plane.as_mut().unwrap().bytes.truncate(1),
                        2 => changed.icc = vec![1],
                        3 => changed.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                        4 => changed.layers[0].effects[0].state.region = Some(1),
                        _ => changed.layers[0].effects[0].operation = "gegl:unqualified-zero-argument".into(),
                    }
                    assert!(classify(&changed, &Interrupt::NONE, None).unwrap().is_none());
                }
                assert!(matches!(classify(&file, &Interrupt::cancel_only(&|| true), None), Err(IoError::Cancelled)));
                let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
                let budget = photocraft_xcf::Budget::new(limits, None);
                assert!(matches!(classify(&file,&Interrupt::NONE,Some(&budget)),Err(IoError::Xcf(error)) if error.kind==photocraft_xcf::ErrorKind::Limit));
            }
        }
    }
    #[test]
    #[ignore = "local unpublished invert-gamma six-precision three-transfer source qualification"]
    fn invert_gamma_source_contract_cancel_budget_and_structural_controls() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-invert-gamma-evidence");
        for precision in ["u8", "u16", "u32", "half", "float", "double"] {
            for encoding in ["linear", "non_linear", "perceptual"] {
                let bytes = std::fs::read(root.join(format!("invert-gamma-rgb-{precision}_{encoding}.small.source.xcf"))).unwrap();
                let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
                assert!(matches!(classify(&file, &Interrupt::NONE, None).unwrap(), Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::InvertGamma))));
                for mutation in 0..6 {
                    let mut changed = file.clone();
                    match mutation {
                        0 => changed.precision = changed.precision.saturating_add(1),
                        1 => changed.layers[0].plane.as_mut().unwrap().bytes.truncate(1),
                        2 => changed.icc = vec![1],
                        3 => changed.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                        4 => changed.layers[0].effects[0].state.region = Some(1),
                        _ => changed.layers[0].effects[0].operation = "gegl:unqualified-zero-argument".into(),
                    }
                    assert!(classify(&changed, &Interrupt::NONE, None).unwrap().is_none());
                }
                assert!(matches!(classify(&file, &Interrupt::cancel_only(&|| true), None), Err(IoError::Cancelled)));
                let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
                let budget = photocraft_xcf::Budget::new(limits, None);
                assert!(matches!(classify(&file,&Interrupt::NONE,Some(&budget)),Err(IoError::Xcf(error)) if error.kind==photocraft_xcf::ErrorKind::Limit));
            }
        }
    }
    #[test]
    #[ignore = "local unpublished six-precision Perceptual source qualification"]
    fn perceptual_precisions_require_exact_plane_encoding_and_shared_budget() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-perceptual-evidence");
        for precision in ["u8", "u16", "u32", "half", "float", "double"] {
            let bytes = std::fs::read(root.join(format!("brightness-rgb-{precision}_perceptual.small.source.xcf"))).unwrap();
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert_eq!(file.encoding, Encoding::Perceptual);
            assert!(matches!(classify(&file, &Interrupt::NONE, None).unwrap(), Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::Brightness))));
            for mutation in 0..6 {
                let mut changed = file.clone();
                match mutation {
                    0 => changed.encoding = Encoding::NonlinearProfile,
                    1 => changed.precision = changed.precision.saturating_add(1),
                    2 => changed.layers[0].plane.as_mut().unwrap().bytes.pop().map(|_| ()).unwrap(),
                    3 => changed.icc = vec![1],
                    4 => changed.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                    _ => changed.layers[0].effects[0].state.region = Some(1),
                }
                assert!(classify(&changed, &Interrupt::NONE, None).unwrap().is_none(), "{precision}/{mutation}");
            }
            let cancel = || true;
            assert!(matches!(classify(&file, &Interrupt::cancel_only(&cancel), None), Err(IoError::Cancelled)));
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
            let budget = photocraft_xcf::Budget::new(limits, None);
            assert!(matches!(classify(&file, &Interrupt::NONE, Some(&budget)), Err(IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit));
            assert_eq!(std::fs::read(root.join(format!("brightness-rgb-{precision}_perceptual.small.source.xcf"))).unwrap(), bytes);
        }
    }
    #[test]
    #[ignore = "local unpublished exact Half/U32 source and shared-budget witnesses"]
    fn half_u32_source_precision_planes_and_budget_are_checked() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-half-u32-evidence");
        for name in ["brightness-rgb-half_linear.small.source.xcf", "brightness-rgb-u32_linear.small.source.xcf"] {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let source = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert!(classify(&source, &Interrupt::NONE, None).unwrap().is_some());
            for mutation in 0..7 {
                let mut file = source.clone();
                match mutation {
                    0 => file.layers[0].plane.as_mut().unwrap().sample = Sample::U16,
                    1 => file.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = Sample::U16,
                    2 => file.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                    3 => file.precision = 400,
                    4 => {
                        file.layers[0].plane.as_mut().unwrap().bytes.pop();
                    }
                    5 => file.layers[0].effects[0].state.enabled = None,
                    _ => file.layers[0].effects[0].mask.as_mut().unwrap().offset = (1, 0),
                }
                assert!(classify(&file, &Interrupt::NONE, None).unwrap().is_none(), "{name}:{mutation}");
            }
            if source.sample == Sample::F16 {
                for sample in [0.1_f32, f32::NAN, -0.25, 1.25] {
                    let mut file = source.clone();
                    file.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&sample.to_ne_bytes());
                    assert!(classify(&file, &Interrupt::NONE, None).unwrap().is_none());
                }
            }
            assert!(matches!(classify(&source, &Interrupt::cancel_only(&|| true), None), Err(IoError::Cancelled)));
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
            let budget = photocraft_xcf::Budget::new(limits, None);
            assert!(matches!(classify(&source, &Interrupt::NONE, Some(&budget)), Err(IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit));
            assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
        }
    }
    #[test]
    #[ignore = "local unpublished point Replace source geometry and cancellation witness"]
    fn source_qualifier_rejects_changed_context_and_obeys_shared_work_and_cancellation() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-geometry-evidence");
        let bytes = std::fs::read(root.join("brightness-rgb-float_non_linear.negative-x.source.xcf")).unwrap();
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(matches!(classify(&file, &Interrupt::NONE, None).unwrap(), Some(NeighborhoodDomain::PointReplace(PointReplaceFamily::Brightness))));
        for mutation in 0..15 {
            let mut hostile = file.clone();
            match mutation {
                0 => hostile.width = 0,
                1 => hostile.width = 262145,
                2 => hostile.layers[0].offset.0 = i32::MAX,
                3 => hostile.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                4 => hostile.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes()),
                5 => hostile.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                6 => hostile.layers[0].effects[0].mask.as_mut().unwrap().offset = (-13, 11),
                7 => hostile.layers[0].effects[0].mask.as_mut().unwrap().mask_editing = true,
                8 => hostile.layers[0].effects[0].mask.as_mut().unwrap().mask_enabled = false,
                9 => hostile.layers[0].effects[0].state.region = Some(1),
                10 => hostile.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                11 => hostile.layers[0].effects[0].state.blend_space = None,
                12 => hostile.layers[0].mode = 30,
                13 => hostile.icc = vec![1],
                _ => hostile.precision = 675,
            }
            assert!(classify(&hostile, &Interrupt::NONE, None).unwrap().is_none(), "{mutation}");
        }
        let calls = AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 4;
        assert!(matches!(classify(&file, &Interrupt::cancel_only(&cancelled), None), Err(IoError::Cancelled)));
        assert!(calls.load(Ordering::Relaxed) <= 6);
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
        let budget = photocraft_xcf::Budget::new(limits, None);
        assert!(matches!(classify(&file, &Interrupt::NONE, Some(&budget)), Err(IoError::Xcf(error)) if error.kind == photocraft_xcf::ErrorKind::Limit));
        assert_eq!(std::fs::read(root.join("brightness-rgb-float_non_linear.negative-x.source.xcf")).unwrap(), bytes);
    }
}
