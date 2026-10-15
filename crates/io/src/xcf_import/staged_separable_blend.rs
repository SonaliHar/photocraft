//! One sealed original-source intersection for flat modern separable blends.
use super::*;
#[path = "staged_legacy_separable_blend.rs"]
mod legacy;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Domain {
    Basic,
    MultiplyAuto,
    MultiplySpaces,
}
pub(super) fn mode(mode: i32) -> bool {
    matches!(mode,23|30..=36|41..=53)
}
pub(super) fn hint<P>(f: &XcfFile<P>) -> bool {
    f.layers.len() == 2 && f.layers.first().is_some_and(|d| !d.group && (mode(d.mode) || legacy::mode(d.mode)))
}
pub(super) fn domain<P: ExactParsedPlane>(f: &XcfFile<P>) -> Option<Domain> {
    if f.model != Model::Rgb
        || f.sample != Sample::F32
        || f.encoding != Encoding::NonlinearProfile
        || !f.icc.is_empty()
        || f.width != 16
        || f.height != 12
        || f.layers.len() != 2
        || !f.channels.is_empty()
        || !f.paths.is_empty()
    {
        return None;
    }
    if !f.layers.iter().all(|d| {
        !d.group
            && d.path.len() <= 1
            && d.effects.is_empty()
            && d.visible
            && d.floating_attachment.is_none()
            && d.mask.is_none()
            && d.model == Model::Rgb
            && d.alpha
            && d.text.is_none()
            && d.vector.is_none()
            && d.link.is_none()
            && d.transform.is_none()
            && d.plane.as_ref().is_some_and(|p| p.descriptor().sample == Sample::F32 && p.descriptor().channels == 4)
    }) {
        return None;
    }
    let top = f.layers.first()?;
    let base = f.layers.get(1)?;
    if !mode(top.mode)
        || top.width != 10
        || top.height != 9
        || !((top.offset == (2, -1) && top.opacity.to_bits() == 0.7125_f32.to_bits())
            || (top.offset == (-1, 2) && top.opacity.to_bits() == 0.4375_f32.to_bits()))
        || base.mode != 28
        || base.width != 16
        || base.height != 12
        || base.offset != (0, 0)
        || base.opacity != 1.0
        || base.blend_space != 0
        || base.composite_space != -1
        || base.composite_mode != -1
    {
        return None;
    }
    if top.mode == 30 {
        if f.version == 12 && top.blend_space == -1 && top.composite_space == -1 && top.composite_mode == -2 {
            return Some(Domain::MultiplyAuto);
        }
        if top.composite_mode == -2 && matches!((f.version, top.blend_space, top.composite_space), (12, 2, -1) | (23, 4, -1) | (12, -1, 2) | (23, -1, 4)) {
            return Some(Domain::MultiplySpaces);
        }
        return None;
    }
    if top.composite_space != -1 || top.composite_mode != -2 {
        return None;
    }
    let basic = (f.version == 23 && matches!(top.mode, 23 | 31 | 32 | 42..=53) && top.blend_space == -4)
        || (f.version == 12 && matches!(top.mode, 33 | 34 | 41) && top.blend_space == -1)
        || (f.version == 12 && matches!(top.mode, 35 | 36) && top.blend_space == 0);
    basic.then_some(Domain::Basic)
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<Option<Domain>, IoError> {
    let Some(domain) = domain(f) else { return Ok(None) };
    let mode = f.layers.first().ok_or_else(|| IoError::Unsupported("separable owner".into()))?.mode;
    let unit = domain == Domain::Basic && matches!(mode, 41..=49 | 51 | 53);
    let mut valid = true;
    for (owner, d) in f.layers.iter().enumerate() {
        let p = d.plane.as_ref().ok_or_else(|| IoError::Unsupported("separable pixels".into()))?;
        let desc = p.descriptor();
        let expected = u64::from(d.width)
            .checked_mul(u64::from(d.height))
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| IoError::Unsupported("separable length".into()))?;
        if desc.width != d.width || desc.height != d.height || desc.channels != 4 || desc.sample != Sample::F32 || p.native_byte_len() != Some(expected) {
            return Ok(None);
        }
        let mut seen = 0u64;
        p.visit_native(ctl, &mut |block| {
            budget.consume_work(block.len() as u64, "separable original native predicates").map_err(accounting_error)?;
            seen = seen.checked_add(block.len() as u64).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("separable excess native".into()))?;
            if block.is_empty() || block.len() > 64 * 16 || block.len() % 16 != 0 {
                return Err(IoError::Unsupported("separable native block".into()));
            }
            for pixel in block.chunks_exact(16) {
                for (channel, raw) in pixel.chunks_exact(4).enumerate() {
                    let bytes = <[u8; 4]>::try_from(raw).map_err(|_| IoError::Unsupported("separable word".into()))?;
                    let value = f32::from_ne_bytes(bytes);
                    valid &= value.is_finite() && (!(unit || channel == 3) || (0.0..=1.0).contains(&value));
                    if owner == 0 && channel != 3 {
                        valid &= match mode {
                            42 => value < 1.0,
                            48 => value > 0.0 && value < 1.0,
                            41 | 43 => value > 0.0,
                            _ => true,
                        };
                    }
                }
            }
            Ok(())
        })?;
        if seen != expected {
            return Err(IoError::Unsupported("separable incomplete native".into()));
        }
    }
    Ok(valid.then_some(domain))
}

// Selection is metadata-first. A selected strict native failure never retries
// the general policy (whose per-entry numeric domain can differ).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceKind {
    Strict(Domain),
    LegacyStrict,
    General,
}
fn pay(ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>, units: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    budget.consume_work(units, "general flat source metadata/native validation").map_err(accounting_error)
}
/// Paid route hint only. In particular every all-Normal flat file remains Plain;
/// an unusual all-Normal Plain refusal must not retry the separable validator.
pub(super) fn general_hint<P>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    pay(ctl, budget, 1)?;
    if f.model != Model::Rgb
        || f.sample != Sample::F32
        || f.precision != 650
        || f.encoding != Encoding::NonlinearProfile
        || !f.icc.is_empty()
        || f.layers.is_empty()
        || f.layers.len() > 4096
    {
        return Ok(false);
    }
    let mut non_normal = false;
    for d in &f.layers {
        pay(ctl, budget, 1)?;
        non_normal |= mode(d.mode);
    }
    Ok(non_normal)
}
pub(super) fn qualify_selected<P: ExactParsedPlane>(
    f: &XcfFile<P>,
    ctl: &Interrupt<'_>,
    budget: &photocraft_xcf::Budget<'_>,
) -> Result<Option<SourceKind>, IoError> {
    if domain(f).is_some() {
        return qualify(f, ctl, budget).map(|kind| kind.map(SourceKind::Strict));
    }
    if legacy::domain(f) { return legacy::qualify(f, ctl, budget).map(|ok| ok.then_some(SourceKind::LegacyStrict)); }
    qualify_general(f, ctl, budget).map(|ok| ok.then_some(SourceKind::General))
}
fn qualify_general<P: ExactParsedPlane>(f: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    use photocraft_doc::import_compositing::flat_modern_source_rule;
    pay(ctl, budget, 1)?;
    if f.model != Model::Rgb
        || f.sample != Sample::F32
        || f.precision != 650
        || f.encoding != Encoding::NonlinearProfile
        || !f.icc.is_empty()
        || f.layers.is_empty()
        || f.layers.len() > 4096
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
        || !f.palette.is_empty()
        || !f.channels.is_empty()
        || !f.paths.is_empty()
        || !f.item_sets.is_empty()
    {
        return Ok(false);
    }
    let mut non_normal = false;
    let mut valid = true;
    for d in &f.layers {
        pay(ctl, budget, 1)?;
        non_normal |= mode(d.mode);
        if d.group
            || d.group_flags != 0
            || d.path.len() > 1
            || d.model != Model::Rgb
            || !d.alpha
            || d.mask.is_some()
            || !d.effects.is_empty()
            || d.text.is_some()
            || d.text_flags != 0
            || d.vector.is_some()
            || d.link.is_some()
            || d.legacy_linked
            || d.transform.is_some()
            || d.floating_attachment.is_some()
            || d.selection
            || !d.item_sets.is_empty()
            || !d.unsupported.is_empty()
            || !d.opacity.is_finite()
            || !(0.0..=1.0).contains(&d.opacity)
            || d.width == 0
            || d.height == 0
            || d.width > 262144
            || d.height > 262144
        {
            return Ok(false);
        }
        let Some(rule) = flat_modern_source_rule(f.version, d.mode, d.blend_space, d.composite_space, d.composite_mode) else { return Ok(false) };
        bounds(d)?;
        let Some(p) = &d.plane else { return Ok(false) };
        let descriptor = p.descriptor();
        let expected = u64::from(d.width)
            .checked_mul(u64::from(d.height))
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| IoError::Unsupported("general flat native length".into()))?;
        if descriptor.width != d.width
            || descriptor.height != d.height
            || descriptor.channels != 4
            || descriptor.sample != Sample::F32
            || p.native_byte_len() != Some(expected)
        {
            return Ok(false);
        }
        let mut seen = 0u64;
        p.visit_native(ctl, &mut |block| {
            if block.is_empty() || block.len() > 64 * 16 || block.len() % 16 != 0 {
                return Err(IoError::Unsupported("general flat native block".into()));
            }
            seen = seen.checked_add(block.len() as u64).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("general flat excess native".into()))?;
            pay(ctl, budget, block.len() as u64)?;
            for pixel in block.chunks_exact(16) {
                for (channel, word) in pixel.chunks_exact(4).enumerate() {
                    let raw = <[u8; 4]>::try_from(word).map_err(|_| IoError::Unsupported("general flat native word".into()))?;
                    let value = f64::from(f32::from_ne_bytes(raw));
                    valid &= if channel == 3 { value.is_finite() && (0.0..=1.0).contains(&value) } else { rule.source_rgb_domain.accepts(value) };
                }
            }
            Ok(())
        })?;
        if seen != expected {
            return Err(IoError::Unsupported("general flat incomplete native".into()));
        }
    }
    pay(ctl, budget, 1)?;
    Ok(non_normal && valid)
}

#[cfg(all(test, feature = "corpus"))]
#[path = "staged_separable_blend_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "staged_general_separable_blend_tests.rs"]
mod general_tests;

pub(super) fn legacy_domain<P:ExactParsedPlane>(f:&XcfFile<P>)->bool{legacy::domain(f)}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "staged_legacy_separable_blend_tests.rs"]
mod legacy_tests;
