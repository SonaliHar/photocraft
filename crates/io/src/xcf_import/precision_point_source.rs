//! Shared existing precision-point source predicate over exact borrowed planes.
//! This neither maps filters nor admits a new source/default renderer endpoint.
use super::{ArgumentValue, Encoding, ErrorKind, ExactParsedPlane, Interrupt, IoError, Model, Sample, XcfFile, exact_half_unit_f32};

fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("precision point source visitor: {message}"))
}
fn poll(control: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>, units: u64) -> Result<(), IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    if let Some(budget) = budget {
        budget
            .consume_work(units, "precision point source qualification")
            .map_err(|error| if error.kind == ErrorKind::Cancelled { IoError::Cancelled } else { IoError::Xcf(error) })?;
    }
    control.check().map_err(|_| IoError::Cancelled)
}

/// Exactly the previous eager predicate's metadata, precision and sample domain.
/// The fallible visitor adds no positive eligibility. It reports malformed
/// descriptor/stream lengths and supplied cancellation/work failures explicitly.
/// The compatibility eager Boolean wrapper keeps its existing false-on-malformed
/// contract; future controlled callers must propagate this Result.
pub(super) fn qualify<P: ExactParsedPlane>(
    file: &XcfFile<P>,
    operation: &str,
    balance_depth: bool,
    exposure_geometry: bool,
    control: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    poll(control, budget, 32)?;
    let canvas = if exposure_geometry { (295, 160) } else { (8, 6) };
    let drawable = if exposure_geometry { (257, 129) } else { (8, 6) };
    let wide = matches!(
        operation,
        "gegl:brightness-contrast"
            | "gegl:levels"
            | "gegl:exposure"
            | "gimp:color-balance"
            | "gegl:color-temperature"
            | "gimp:desaturate"
            | "gegl:saturation"
            | "gegl:sepia"
            | "gegl:alien-map"
            | "gimp:threshold"
            | "gimp:threshold-alpha"
    );
    let width = match (file.precision, file.sample, file.encoding) {
        (100, Sample::U8, Encoding::Linear) | (150, Sample::U8, Encoding::NonlinearProfile) | (175, Sample::U8, Encoding::Perceptual) if balance_depth => 1,
        (300, Sample::U32, Encoding::Linear) | (350, Sample::U32, Encoding::NonlinearProfile) | (375, Sample::U32, Encoding::Perceptual) if balance_depth => 4,
        (500, Sample::F16, Encoding::Linear) | (550, Sample::F16, Encoding::NonlinearProfile) | (575, Sample::F16, Encoding::Perceptual) if balance_depth => 4,
        (275, Sample::U16, Encoding::Perceptual) if balance_depth => 2,
        (675, Sample::F32, Encoding::Perceptual) if balance_depth => 4,
        (775, Sample::F64, Encoding::Perceptual) if balance_depth => 8,
        (250, Sample::U16, Encoding::NonlinearProfile) if !balance_depth => 2,
        (600, Sample::F32, Encoding::Linear) if !balance_depth => 4,
        (750, Sample::F64, Encoding::NonlinearProfile) if !balance_depth => 8,
        (200, Sample::U16, Encoding::Linear) if wide && !balance_depth => 2,
        (650, Sample::F32, Encoding::NonlinearProfile) if wide && !balance_depth => 4,
        (700, Sample::F64, Encoding::Linear) if wide && !balance_depth => 8,
        _ => return Ok(false),
    };
    if !(file.version == 22
        && (file.model == Model::Rgb
            || wide
                && !matches!(operation, "gimp:color-balance" | "gimp:desaturate" | "gimp:threshold" | "gegl:sepia" | "gegl:alien-map")
                && file.model == Model::Gray)
        && file.icc.is_empty()
        && (file.width, file.height) == canvas
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1)
    {
        return Ok(false);
    }
    let Some(d) = file.layers.first() else { return Ok(false) };
    if !(!d.group
        && d.path.len() <= 1
        && d.visible
        && d.alpha
        && d.model == file.model
        && (d.width, d.height) == drawable
        && (if exposure_geometry { matches!(d.offset, (-13, 13) | (17, -11)) } else { d.offset == (0, 0) })
        && d.mode == 28
        && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
        && d.opacity.to_bits() == 1.0_f32.to_bits()
        && d.mask.is_none()
        && d.text.is_none()
        && d.vector.is_none()
        && d.link.is_none()
        && d.transform.is_none()
        && d.floating_attachment.is_none()
        && d.effects.len() == 1)
    {
        return Ok(false);
    }
    let Some(effect) = d.effects.first() else { return Ok(false) };
    if effect.operation != operation {
        return Ok(false);
    }
    if operation == "gegl:saturation" {
        let mut colorspace = false;
        for argument in &effect.arguments {
            poll(control, budget, 1)?;
            if argument.name == "colorspace"
                && (matches!(argument.value, ArgumentValue::Enum(0 | 1))
                    || argument.value == ArgumentValue::Enum(2) && (file.model == Model::Gray || matches!(file.sample, Sample::U16 | Sample::F32)))
            {
                colorspace = true;
                break;
            }
        }
        if !colorspace {
            return Ok(false);
        }
    }
    let Some(plane) = d.plane.as_ref() else { return Ok(false) };
    poll(control, budget, 8)?;
    let descriptor = plane.descriptor();
    if !(descriptor.sample == if file.sample == Sample::F16 { Sample::F32 } else { file.sample }
        && descriptor.channels == if file.model == Model::Gray { 2 } else { 4 }
        && descriptor.width == drawable.0
        && descriptor.height == drawable.1)
    {
        return Ok(false);
    }
    let stride = descriptor.channels.checked_mul(width).ok_or_else(|| bad("pixel stride overflow"))?;
    let expected = usize::try_from(drawable.0)
        .ok()
        .and_then(|w| w.checked_mul(drawable.1 as usize))
        .and_then(|n| n.checked_mul(stride))
        .ok_or_else(|| bad("plane byte count overflow"))?;
    let maximum_block = stride.checked_mul(64).ok_or_else(|| bad("visitor block overflow"))?;
    let mut seen = 0usize;
    let mut valid = true;
    plane.visit_native(control, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > maximum_block || bytes.len() % stride != 0 {
            return Err(bad("non-pixel-aligned, empty or oversized block"));
        }
        let next = seen.checked_add(bytes.len()).filter(|next| *next <= expected).ok_or_else(|| bad("extra plane bytes"))?;
        let work = bytes.len().checked_add(bytes.len() / width).ok_or_else(|| bad("native validation work overflow"))?;
        poll(control, budget, u64::try_from(work).map_err(|_| bad("native work address space"))?)?;
        seen = next;
        if valid {
            valid = bytes.chunks_exact(width).all(|bytes| match file.sample {
                Sample::U8 | Sample::U16 | Sample::U32 => true,
                Sample::F16 | Sample::F32 => {
                    let Ok(raw) = <[u8; 4]>::try_from(bytes) else { return false };
                    let value = f32::from_ne_bytes(raw);
                    value.is_finite() && (0.0..=1.0).contains(&value) && (file.sample != Sample::F16 || exact_half_unit_f32(value))
                }
                Sample::F64 => {
                    let Ok(raw) = <[u8; 8]>::try_from(bytes) else { return false };
                    let value = f64::from_ne_bytes(raw);
                    value.is_finite() && (0.0..=1.0).contains(&value)
                }
            });
        }
        poll(control, budget, 0)
    })?;
    if seen != expected {
        return Err(bad("short plane byte count"));
    }
    poll(control, budget, 0)?;
    Ok(valid)
}

#[cfg(test)]
#[path = "precision_point_source_tests.rs"]
mod tests;
