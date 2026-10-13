//! Explicitly accounted ordinary Normal-mask source geometry.
use crate::IoError;
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, Drawable, Encoding, Model, Sample, XcfFile};

fn plain(item: &Drawable) -> bool {
    !item.group
        && item.path.len() <= 1
        && item.effects.is_empty()
        && item.text.is_none()
        && item.vector.is_none()
        && item.link.is_none()
        && item.transform.is_none()
        && item.visible
        && item.floating_attachment.is_none()
}
fn pixels(width: u32, height: u32) -> Option<u64> {
    (width > 0 && height > 0 && width <= 262144 && height <= 262144).then(|| u64::from(width).checked_mul(u64::from(height))).flatten()
}
fn plane(item: &Drawable, sample: Sample, channels: usize, ctl: &Interrupt<'_>, budget: &Budget<'_>) -> Result<bool, IoError> {
    let Some(plane) = &item.plane else { return Ok(false) };
    let bytes = match sample {
        Sample::U16 => 2,
        Sample::U32 | Sample::F16 | Sample::F32 => 4,
        Sample::F64 => 8,
        _ => return Ok(false),
    };
    let expected = pixels(item.width, item.height).and_then(|n| n.checked_mul(channels as u64)).and_then(|n| n.checked_mul(bytes as u64));
    let decoded_sample = if sample == Sample::F16 { Sample::F32 } else { sample };
    if plane.sample != decoded_sample
        || plane.channels != channels
        || plane.width != item.width
        || plane.height != item.height
        || expected != Some(plane.bytes.len() as u64)
    {
        return Ok(false);
    }
    // Charge and poll before each <=64-pixel validation chunk. Integer values
    // retain their exact bits; floating source values must be finite unit data.
    for chunk in plane.bytes.chunks(bytes * channels * 64) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        budget.consume_work((chunk.len() / bytes) as u64, "ordinary source plane validation")?;
        if matches!(sample, Sample::F16 | Sample::F32 | Sample::F64) {
            for raw in chunk.chunks_exact(bytes) {
                let value = if sample == Sample::F64 {
                    let Ok(raw) = raw.try_into() else { return Ok(false) };
                    f64::from_ne_bytes(raw)
                } else {
                    let Ok(raw) = raw.try_into() else { return Ok(false) };
                    f64::from(f32::from_ne_bytes(raw))
                };
                if !value.is_finite() || !(0.0..=1.0).contains(&value) || sample == Sample::F16 && !super::xcf_import::exact_half_unit_f32(value as f32) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
pub(crate) fn qualifies(file: &XcfFile, ctl: &Interrupt<'_>, budget: &Budget<'_>) -> Result<bool, IoError> {
    let Some(canvas) = pixels(file.width, file.height) else { return Ok(false) };
    if canvas > 1_u64 << 30
        || file.version != 12
        || !matches!(file.model, Model::Rgb | Model::Gray)
        || !matches!(
            (file.precision, file.sample, file.encoding),
            (250, Sample::U16, Encoding::NonlinearProfile)
                | (350, Sample::U32, Encoding::NonlinearProfile)
                | (550, Sample::F16, Encoding::NonlinearProfile)
                | (650, Sample::F32, Encoding::NonlinearProfile)
                | (750, Sample::F64, Encoding::NonlinearProfile)
        )
        || !file.channels.is_empty()
        || !file.paths.is_empty()
        || file.layers.len() != 2
        || !(132..=65536).contains(&file.icc.len())
    {
        return Ok(false);
    }
    let profile_bound = (file.icc.len() as u64)
        .checked_mul(128)
        .and_then(|n| n.checked_add(65536))
        .ok_or_else(|| IoError::Unsupported("ordinary ICC preparation overflow".into()))?;
    let _profile_lease = budget.reserve(photocraft_xcf::BudgetKind::Resident, profile_bound, "ordinary source ICC preparation")?;
    for chunk in file.icc.chunks(4096) {
        budget.consume_work(chunk.len() as u64, "ordinary source ICC qualification")?;
    }
    if !super::xcf_import::qualified_analytic_normal_icc(file) {
        return Ok(false);
    }
    let Some(top) = file.layers.first() else { return Ok(false) };
    let Some(base) = file.layers.get(1) else { return Ok(false) };
    let Some(mask) = &top.mask else { return Ok(false) };
    let Some(owner_pixels) = pixels(top.width, top.height) else { return Ok(false) };
    if canvas.checked_add(owner_pixels.checked_mul(2).ok_or_else(|| IoError::Unsupported("ordinary source size overflow".into()))?).is_none()
        || !file.layers.iter().all(|item| {
            plain(item) && item.model == file.model && item.alpha && item.mode == 28 && item.blend_space == 0 && super::xcf_import::bounds(item).is_ok()
        })
        || top.composite_space != 1
        || !matches!(top.composite_mode, 1..=4)
        || !top.opacity.is_finite()
        || !(0.0..=1.0).contains(&top.opacity)
        || !top.mask_enabled
        || top.mask_shown
        || top.mask_editing
        || !plain(mask)
        || (mask.width, mask.height) != (top.width, top.height)
        || mask.offset != (0, 0)
        || mask.model != Model::Gray
        || mask.alpha
        || (base.width, base.height) != (file.width, file.height)
        || base.offset != (0, 0)
        || base.opacity.to_bits() != 1_f32.to_bits()
        || base.mask.is_some()
        || (base.composite_space, base.composite_mode) != (-1, -1)
    {
        return Ok(false);
    }
    let channels = if file.model == Model::Gray { 2 } else { 4 };
    Ok(plane(top, file.sample, channels, ctl, budget)? && plane(base, file.sample, channels, ctl, budget)? && plane(mask, file.sample, 1, ctl, budget)?)
}
