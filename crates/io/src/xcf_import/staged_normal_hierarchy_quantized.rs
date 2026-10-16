//! Explicit plain integer Source purpose. No integer filter/mask programme.
use super::*;
use crate::xcf_import::staged_filtered_flat::work;
pub(super) fn exact_depth<P>(f: &XcfFile<P>) -> bool {
    matches!((f.sample, f.precision), (Sample::U8, 150) | (Sample::U16, 250))
}
/// Metadata planning miss is permanently old-only, not a new Source failure.
pub(super) fn plan_eligible<P>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    if f.layers.len() > 4096 {
        return Ok(false);
    }
    for d in &f.layers {
        work(i, b, 8)?;
        if !d.effects.is_empty() || d.mask.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn preflight<P>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
    if !plan_eligible(f, i, b)? {
        return Err(IoError::Unsupported("integer hierarchy requires globally empty effects and masks".into()));
    }
    Ok(())
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
    if !exact_depth(f) {
        return Err(IoError::Unsupported("plain integer hierarchy source precision".into()));
    }
    preflight(f, i, b)?;
    super::staged_normal_hierarchy::validate_source_body(f, i, b, false, true)
}
/// Integer codes are intrinsically finite/unit; storage shape and every actual
/// byte are nevertheless checked under the original visitor/work controls.
pub(super) fn native<P: ExactParsedPlane>(
    p: &P,
    sample: Sample,
    width: u32,
    height: u32,
    i: &Interrupt<'_>,
    b: &photocraft_xcf::Budget<'_>,
) -> Result<bool, IoError> {
    let word = match sample {
        Sample::U8 => 1usize,
        Sample::U16 => 2,
        _ => return Ok(false),
    };
    let pixel = 4usize.checked_mul(word).ok_or_else(|| IoError::Unsupported("integer hierarchy pixel stride".into()))?;
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(pixel as u64))
        .ok_or_else(|| IoError::Unsupported("integer hierarchy native bytes".into()))?;
    work(i, b, 16)?;
    let d = p.descriptor();
    if d.sample != sample || (d.width, d.height, d.channels) != (width, height, 4) || p.native_byte_len() != Some(expected) {
        return Ok(false);
    }
    let mut seen = 0u64;
    let mut valid = true;
    p.visit_native(i, &mut |chunk| {
        work(i, b, (chunk.len() as u64).checked_add((chunk.len() / word) as u64).ok_or_else(|| IoError::Unsupported("integer hierarchy scan work".into()))?)?;
        seen = seen.checked_add(chunk.len() as u64).ok_or_else(|| IoError::Unsupported("integer hierarchy native count".into()))?;
        valid &= !chunk.is_empty() && chunk.len() <= 64 * pixel && chunk.len() % pixel == 0 && seen <= expected;
        Ok(())
    })?;
    work(i, b, 1)?;
    Ok(valid && seen == expected)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_normal_hierarchy_quantized_tests.rs"]
mod tests;
