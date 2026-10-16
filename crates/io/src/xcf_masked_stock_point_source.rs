//! Sealed nonempty-mask singleton4. Parameter parsing is scalar-only;
//! no mixed envelope, kernel preparation, CMS, or source-opacity authority.
use crate::{IoError, xcf_filters::PointReplaceFamily as F, xcf_import::plane::ExactParsedPlane};
use photocraft_raster::Interrupt;
use photocraft_xcf::{ArgumentValue, Effect, Encoding, Model, Sample, XcfFile};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    InvertLinear,
    InvertGamma,
    BrightnessContrast,
    Levels,
}
impl Kind {
    pub(crate) fn family(self) -> F {
        match self {
            Self::InvertLinear => F::MaskedInvertLinear,
            Self::InvertGamma => F::MaskedInvertGamma,
            Self::BrightnessContrast => F::MaskedBrightnessContrast,
            Self::Levels => F::MaskedGeglLevels,
        }
    }
    pub(crate) fn epoch(self) -> &'static str {
        match self {
            Self::InvertLinear => photocraft_compose::INVERT_LINEAR_MASKED_EPOCH,
            Self::InvertGamma => photocraft_compose::INVERT_GAMMA_MASKED_EPOCH,
            Self::BrightnessContrast => photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_EPOCH,
            Self::Levels => photocraft_compose::GEGL_LEVELS_MASKED_EPOCH,
        }
    }
}
pub(crate) fn kind<P>(e: &Effect<P>) -> Option<Kind> {
    match e.operation.as_str() {
        "gegl:invert-linear" => Some(Kind::InvertLinear),
        "gegl:invert-gamma" => Some(Kind::InvertGamma),
        "gegl:brightness-contrast" => Some(Kind::BrightnessContrast),
        "gegl:levels" => Some(Kind::Levels),
        _ => None,
    }
}
pub(crate) fn selected(f: F) -> bool {
    matches!(f, F::MaskedInvertLinear | F::MaskedInvertGamma | F::MaskedBrightnessContrast | F::MaskedGeglLevels)
}
pub(crate) fn parameters<P>(e: &Effect<P>) -> Option<Kind> {
    if e.version.as_deref() != Some("0:0") {
        return None;
    }
    let k = kind(e)?;
    let names: &[&str] = match k {
        Kind::InvertLinear | Kind::InvertGamma => &[],
        Kind::BrightnessContrast => &["brightness", "contrast"],
        Kind::Levels => &["in-low", "in-high", "out-low", "out-high"],
    };
    if e.arguments.len() != names.len() {
        return None;
    }
    let mut words = [None; 4];
    for a in &e.arguments {
        if a.configuration.is_some() || a.graph.is_some() {
            return None;
        }
        let ArgumentValue::Float32Bits(bits) = a.value else {
            return None;
        };
        let index = names.iter().position(|n| *n == a.name)?;
        if words.get_mut(index)?.replace(bits).is_some() {
            return None;
        }
    }
    use photocraft_algo::gimp::point::PointOperation;
    match k {
        Kind::InvertLinear | Kind::InvertGamma => {}
        Kind::BrightnessContrast => {
            PointOperation::BrightnessContrast { brightness: f64::from(f32::from_bits(words[0]?)), contrast: f64::from(f32::from_bits(words[1]?)) }
                .validate()
                .ok()?
        }
        Kind::Levels => PointOperation::Levels {
            input_low: f64::from(f32::from_bits(words[0]?)),
            input_high: f64::from(f32::from_bits(words[1]?)),
            output_low: f64::from(f32::from_bits(words[2]?)),
            output_high: f64::from(f32::from_bits(words[3]?)),
        }
        .validate()
        .ok()?,
    };
    Some(k)
}
pub(crate) fn mapping_bytes<P>(e: &Effect<P>) -> Option<usize> {
    let n = e.arguments.len();
    if n > 4 {
        return None;
    }
    let count = n.checked_mul(2)?;
    let mut bytes = count
        .checked_mul(std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>())?
        .checked_add(count.checked_mul(std::mem::size_of::<(&str, f64)>())?)?
        .checked_add(std::mem::size_of::<Vec<photocraft_doc::xcf_filters::FilterArgument>>())?
        .checked_add(std::mem::size_of::<Vec<(&str, f64)>>())?;
    for a in &e.arguments {
        bytes = bytes.checked_add(a.name.len().checked_mul(2)?)?;
    }
    Some(bytes)
}
pub(crate) fn hint<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    work(i, Some(b), 32)?;
    if f.layers.len() != 1
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > 1 << 30)
    {
        return Ok(false);
    }
    let Some(d) = f.layers.first() else { return Ok(false) };
    if d.effects.len() != 1 {
        return Ok(false);
    }
    let Some(e) = d.effects.first() else { return Ok(false) };
    if kind(e).is_none() {
        return Ok(false);
    }
    let Some(p) = e.mask.as_ref().and_then(|m| m.plane.as_ref()) else { return Ok(false) };
    let (_, nonempty) = super::masked_exposure_source::visit(p, f.sample, f.width, f.height, 1, true, i, Some(b))?;
    Ok(nonempty)
}
pub(crate) fn envelope<P>(e: &Effect<P>) -> bool {
    let s = &e.state;
    e.properties.len() <= 16
        && parameters(e).is_some()
        && s.enabled == Some(true)
        && s.mode == Some(62)
        && s.opacity_bits == Some(1f32.to_bits())
        && s.opacity_u8.is_none()
        && s.clipping == Some(0)
        && s.region == Some(0)
        && s.blend_space.is_some()
        && s.composite_space.is_some()
        && s.composite_mode.is_some()
        && e.properties.iter().all(|p| matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46))
}
fn work(i: &Interrupt<'_>, b: Option<&photocraft_xcf::Budget<'_>>, n: usize) -> Result<(), IoError> {
    i.check().map_err(|_| IoError::Cancelled)?;
    if let Some(b) = b {
        b.consume_work(n as u64, "masked singleton4 source")
            .map_err(|e| if e.kind == photocraft_xcf::ErrorKind::Cancelled { IoError::Cancelled } else { IoError::Xcf(e) })?;
    }
    i.check().map_err(|_| IoError::Cancelled)
}
pub(crate) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    // Fixed512 prepays the bounded repeated0/0/2/4 scalar shape/domain walks
    // through qualification and the later selected mapper; no kernel/CMS work.
    work(i, b, 512)?;
    if f.version != 22
        || f.model != Model::Rgb
        || f.encoding != Encoding::NonlinearProfile
        || !matches!((f.sample, f.precision), (Sample::F32, 650) | (Sample::F64, 750))
        || !f.icc.is_empty()
        || f.width == 0
        || f.height == 0
        || f.width > 262144
        || f.height > 262144
        || u64::from(f.width).checked_mul(u64::from(f.height)).is_none_or(|n| n > (1u64 << 30))
        || f.layers.len() != 1
        || !f.channels.is_empty()
        || !f.paths.is_empty()
        || !f.palette.is_empty()
        || !f.item_sets.is_empty()
    {
        return Ok(false);
    }
    let Some(d) = f.layers.first() else { return Ok(false) };
    if d.group
        || d.model != Model::Rgb
        || !d.alpha
        || !d.visible
        || d.opacity.to_bits() != 1f32.to_bits()
        || (d.width, d.height, d.offset) != (f.width, f.height, (0, 0))
        || d.mode != 28
        || (d.blend_space, d.composite_space, d.composite_mode) != (0, -1, -1)
        || d.path.len() > 1
        || d.mask.is_some()
        || d.text.is_some()
        || d.vector.is_some()
        || d.legacy_linked
        || d.selection
        || d.link.is_some()
        || d.transform.is_some()
        || d.floating_attachment.is_some()
        || d.effects.len() != 1
        || d.group_flags != 0
        || d.text_flags != 0
    {
        return Ok(false);
    }
    work(i, b, d.unsupported.len())?;
    if d.unsupported.iter().any(|s| s != "GEGL layer effects") {
        return Ok(false);
    }
    let Some(e) = d.effects.first() else { return Ok(false) };
    if !envelope(e) {
        return Ok(false);
    }
    let Some(p) = d.plane.as_ref() else { return Ok(false) };
    if !super::masked_exposure_source::visit(p, f.sample, f.width, f.height, 4, false, i, b)?.0 {
        return Ok(false);
    }
    let Some(m) = e.mask.as_ref() else { return Ok(false) };
    if m.group
        || m.alpha
        || m.model != Model::Gray
        || !m.visible
        || m.opacity.to_bits() != 0.5f32.to_bits()
        || (m.width, m.height, m.offset) != (f.width, f.height, (0, 0))
        || !m.mask_enabled
        || m.mask_shown
        || m.mask_editing
        || m.legacy_linked
        || m.mode != 0
        || (m.blend_space, m.composite_space, m.composite_mode) != (2, 2, 1)
        || !m.path.is_empty()
        || !m.effects.is_empty()
        || m.mask.is_some()
        || !m.unsupported.is_empty()
        || m.text.is_some()
        || m.text_flags != 0
        || m.group_flags != 0
        || m.vector.is_some()
        || m.link.is_some()
        || m.transform.is_some()
        || m.floating_attachment.is_some()
    {
        return Ok(false);
    }
    let Some(p) = m.plane.as_ref() else { return Ok(false) };
    let (valid, nonempty) = super::masked_exposure_source::visit(p, f.sample, f.width, f.height, 1, true, i, b)?;
    Ok(valid && nonempty)
}
