//! Existing legacy scalar source identities, kept separate from PointReplace.
//! One successful admission triggers paid preparation without changing old
//! per-domain mask mapping flags or manufacturing source/renderer authority.
use super::*;
use crate::xcf_filters::NeighborhoodDomain;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::xcf_import) enum LegacyScalarFamily {
    ExposureGeometry,
    RgbClipHdr,
    Unpremultiply,
    MonoOriginal,
    MonoEdits,
    ValueOriginal,
}
impl LegacyScalarFamily {
    pub(super) fn hint<P: ExactParsedPlane>(file: &XcfFile<P>) -> Option<Self> {
        let effect = file.layers.first()?.effects.first()?;
        if effect.arguments.len() > 10 {
            return None;
        }
        let raw = (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode);
        Some(match effect.operation.as_str() {
            "gegl:exposure" if raw == (Some(-901479855), Some(-901479855), Some(-901479855)) => Self::ExposureGeometry,
            "gegl:rgb-clip" => Self::RgbClipHdr,
            "gegl:unpremultiply" => Self::Unpremultiply,
            "gegl:mono-mixer" if raw == (Some(107971153), Some(107971153), Some(107971153)) => Self::MonoEdits,
            "gegl:mono-mixer" if raw == (Some(-261651887), Some(-261651887), Some(-261651887)) => Self::MonoOriginal,
            "gegl:value-invert" if file.precision == 150 && (file.width, file.height) == (32, 32) => Self::ValueOriginal,
            _ => return None,
        })
    }
    pub(super) fn domain(self) -> NeighborhoodDomain {
        match self {
            Self::ExposureGeometry => NeighborhoodDomain::ExposureGeometry,
            Self::RgbClipHdr => NeighborhoodDomain::RgbClipHdr,
            Self::Unpremultiply => NeighborhoodDomain::UnpremultiplyRgbFloatLinear,
            Self::MonoOriginal => NeighborhoodDomain::MonoMixerOriginal,
            Self::MonoEdits => NeighborhoodDomain::MonoMixerEditsOriginal,
            Self::ValueOriginal => NeighborhoodDomain::ValueInvertOriginal,
        }
    }
    pub(super) fn set_late(self, domains: &mut admission::LateAdmissionDomains) {
        match self {
            Self::ExposureGeometry => domains.exposure_geometry_domain = true,
            Self::RgbClipHdr => domains.hdr_clip_domain = true,
            Self::Unpremultiply => {}
            Self::MonoOriginal => domains.mono_mixer_domain = true,
            Self::MonoEdits => {
                domains.mono_mixer_domain = true;
                domains.mono_mixer_edits_domain = true
            }
            Self::ValueOriginal => domains.value_invert_domain = true,
        }
    }
    pub(super) fn qualify<P: ExactParsedPlane>(
        self,
        file: &XcfFile<P>,
        ctl: &Interrupt<'_>,
        budget: Option<&photocraft_xcf::Budget<'_>>,
    ) -> Result<bool, IoError> {
        if Self::hint(file) != Some(self) {
            return Ok(false);
        }
        match self {
            Self::ExposureGeometry => precision_point_source::qualify(file, "gegl:exposure", false, true, ctl, budget),
            Self::RgbClipHdr => hdr_clip(file, ctl, budget),
            Self::Unpremultiply => unpremultiply(file, ctl, budget),
            Self::MonoOriginal | Self::MonoEdits => residual::opaque_original(file, "gegl:mono-mixer", ctl, budget),
            Self::ValueOriginal => residual::opaque_original(file, "gegl:value-invert", ctl, budget),
        }
    }
}
fn poll(ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>, units: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Some(budget) = budget {
        budget.consume_work(units, "legacy scalar original source").map_err(accounting_error)?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}
fn visit<P: ExactParsedPlane>(
    plane: &P,
    expected: usize,
    stride: usize,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
    mut valid: impl FnMut(&[u8]) -> Result<bool, IoError>,
) -> Result<bool, IoError> {
    let mut seen = 0usize;
    let mut accepted = true;
    plane.visit_native(ctl, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > stride * 64 || bytes.len() % stride != 0 {
            return Err(IoError::Unsupported("legacy scalar native block".into()));
        }
        seen = seen.checked_add(bytes.len()).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("legacy scalar extra native bytes".into()))?;
        poll(ctl, budget, bytes.len() as u64)?;
        accepted &= valid(bytes)?;
        Ok(())
    })?;
    if seen != expected {
        return Err(IoError::Unsupported("legacy scalar incomplete native bytes".into()));
    }
    poll(ctl, budget, 0)?;
    Ok(accepted)
}
fn hdr_clip<P: ExactParsedPlane>(file: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if !(file.version == 22
        && file.precision == 600
        && file.sample == Sample::F32
        && file.model == Model::Rgb
        && file.encoding == Encoding::Linear
        && file.icc.is_empty()
        && (file.width, file.height) == (8, 6)
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
        && d.model == Model::Rgb
        && (d.width, d.height) == (8, 6)
        && d.offset == (0, 0)
        && d.mode == 28
        && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
        && d.opacity.to_bits() == 1.0f32.to_bits()
        && d.mask.is_none()
        && d.text.is_none()
        && d.vector.is_none()
        && d.link.is_none()
        && d.transform.is_none()
        && d.floating_attachment.is_none()
        && d.effects.len() == 1
        && d.effects.first().is_some_and(|e| e.operation == "gegl:rgb-clip"))
    {
        return Ok(false);
    }
    let Some(plane) = d.plane.as_ref() else { return Ok(false) };
    let descriptor = plane.descriptor();
    if descriptor.sample != Sample::F32 || descriptor.channels != 4 || plane.native_byte_len() != Some(8 * 6 * 16) {
        return Ok(false);
    }
    visit(plane, 768, 16, ctl, budget, |bytes| {
        for (index, raw) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let value = f32::from_ne_bytes(*raw);
            if !value.is_finite() || index % 4 == 3 && value.to_bits() != 1.0f32.to_bits() {
                return Ok(false);
            }
        }
        Ok(true)
    })
}
fn unpremultiply<P: ExactParsedPlane>(file: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if file.version != 22
        || file.precision != 600
        || file.model != Model::Rgb
        || file.sample != Sample::F32
        || file.encoding != Encoding::Linear
        || (file.width, file.height) != (8, 6)
        || !file.icc.is_empty()
        || file.layers.len() != 1
        || !file.channels.is_empty()
        || !file.paths.is_empty()
        || !file.palette.is_empty()
    {
        return Ok(false);
    }
    let Some(owner) = file.layers.first() else { return Ok(false) };
    if owner.group
        || owner.path.len() > 1
        || !owner.visible
        || !owner.alpha
        || owner.model != Model::Rgb
        || (owner.width, owner.height, owner.offset) != (8, 6, (0, 0))
        || owner.mode != 28
        || (owner.blend_space, owner.composite_space, owner.composite_mode) != (0, -1, -1)
        || owner.opacity.to_bits() != 1f32.to_bits()
        || owner.mask.is_some()
        || owner.text.is_some()
        || owner.vector.is_some()
        || owner.link.is_some()
        || owner.transform.is_some()
        || owner.floating_attachment.is_some()
        || owner.effects.len() != 1
    {
        return Ok(false);
    }
    let Some(effect) = owner.effects.first() else { return Ok(false) };
    if !super::super::unpremultiply::observed_envelope(effect) {
        return Ok(false);
    }
    let Some(mask) = &effect.mask else { return Ok(false) };
    if mask.model != Model::Gray
        || mask.alpha
        || mask.group
        || (mask.width, mask.height, mask.offset) != (8, 6, (0, 0))
        || !mask.visible
        || !mask.mask_enabled
        || mask.mask_shown
        || mask.mask_editing
        || mask.legacy_linked
        || (mask.mode, mask.blend_space, mask.composite_space, mask.composite_mode) != (0, 2, 2, 1)
        || !mask.effects.is_empty()
        || !mask.unsupported.is_empty()
        || mask.mask.is_some()
        || mask.text.is_some()
        || mask.vector.is_some()
        || mask.link.is_some()
        || mask.transform.is_some()
        || mask.floating_attachment.is_some()
    {
        return Ok(false);
    }
    let Some(mask_plane) = &mask.plane else { return Ok(false) };
    let d = mask_plane.descriptor();
    if (d.width, d.height, d.channels, d.sample) != (8, 6, 1, Sample::F32) || mask_plane.native_byte_len() != Some(192) {
        return Ok(false);
    }
    if !visit(mask_plane, 192, 4, ctl, budget, |bytes| Ok(bytes.iter().all(|b| *b == 0)))? {
        return Ok(false);
    }
    let Some(plane) = &owner.plane else { return Ok(false) };
    let d = plane.descriptor();
    if (d.width, d.height, d.channels, d.sample) != (8, 6, 4, Sample::F32) || plane.native_byte_len() != Some(768) {
        return Ok(false);
    }
    visit(plane, 768, 16, ctl, budget, |bytes| {
        for raw in bytes.as_chunks::<16>().0 {
            let mut rgba = [0f32; 4];
            for (value, word) in rgba.iter_mut().zip(raw.as_chunks::<4>().0) {
                *value = f32::from_ne_bytes(*word);
            }
            let [r, g, b, a] = rgba;
            if rgba.iter().any(|v| !v.is_finite())
                || !(0.0..=1.0).contains(&a)
                || effect.state.enabled == Some(true) && a != 0.0 && [r, g, b].iter().any(|c| !(*c / a).is_finite())
            {
                return Ok(false);
            }
        }
        Ok(true)
    })
}

#[cfg(test)]
#[path = "staged_legacy_scalar_tests.rs"]
mod tests;
