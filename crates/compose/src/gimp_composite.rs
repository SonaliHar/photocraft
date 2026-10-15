//! Checked straight-alpha GIMP Normal composition in explicit source-profile spaces.
//! Operators select union/backdrop/layer/intersection coverage; alpha is never gamma encoded.
use photocraft_cms::xcf_color::Encoding;
use photocraft_cms::{CmsError, Profile, Transform};

#[path = "gimp_composite_controlled.rs"]
mod controlled;
use photocraft_doc::{
    SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompositionError {
    Invalid(&'static str),
    Unsupported(&'static str),
    Color(CmsError),
    Raster(photocraft_raster::RasterInputError),
}
impl std::fmt::Display for CompositionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "invalid imported composition: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported imported composition: {m}"),
            Self::Color(e) => e.fmt(f),
            Self::Raster(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for CompositionError {}
impl From<CmsError> for CompositionError {
    fn from(e: CmsError) -> Self {
        Self::Color(e)
    }
}
impl From<photocraft_raster::RasterInputError> for CompositionError {
    fn from(e: photocraft_raster::RasterInputError) -> Self {
        Self::Raster(e)
    }
}
pub type Result<T> = std::result::Result<T, CompositionError>;
pub fn encoding(e: SampleEncoding) -> Encoding {
    match e {
        SampleEncoding::Linear => Encoding::Linear,
        SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
        SampleEncoding::Perceptual => Encoding::Perceptual,
    }
}
pub fn space(e: SampleEncoding) -> GimpSpace {
    match e {
        SampleEncoding::Linear => GimpSpace::LinearRgb,
        SampleEncoding::NonlinearProfile => GimpSpace::ProfileRgb,
        SampleEncoding::Perceptual => GimpSpace::PerceptualRgb,
    }
}

/// Prepared transforms are built once per working space, not once per pixel/layer.
#[derive(Debug)]
pub struct WorkingSpace {
    kind: GimpSpace,
    forward: Option<Transform>,
    reverse: Option<Transform>,
    channels: usize,
    transfer: Option<([Option<photocraft_cms::curve::Curve>; 3], Encoding, Encoding)>,
    luminance: Option<[f64; 3]>,
    physical_lab: Option<(photocraft_cms::pipeline::Pipeline, photocraft_cms::pipeline::Pipeline)>,
}
impl WorkingSpace {
    pub fn new(profile: &Profile, stored: SampleEncoding, working: GimpSpace) -> Result<Self> {
        Self::new_with_interrupt(profile, stored, working, &photocraft_raster::Interrupt::NONE)
    }
    /// Same color equations and admission as `new`, with preparation fences and
    /// fallible retained curve storage and controlled paired CMS transforms.
    /// Physical-Lab side-pipeline construction and builtin initialization retain
    /// their explicit preparation boundaries.
    pub fn new_with_interrupt(profile: &Profile, stored: SampleEncoding, working: GimpSpace, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        controlled::prepare(control, || Self::prepare_with_interrupt(profile, stored, working, control))
    }
    fn prepare_with_interrupt(profile: &Profile, stored: SampleEncoding, working: GimpSpace, control: &photocraft_raster::Interrupt<'_>) -> Result<Self> {
        controlled::validate_interpretation(profile, encoding(stored), control)?;
        let luminance = crate::gimp_blend::luminance_coefficients(profile);
        let same = working == space(stored);
        if same {
            return Ok(Self { kind: working, forward: None, reverse: None, channels: profile.channels(), transfer: None, physical_lab: None, luminance });
        }
        if working != GimpSpace::Lab {
            let to = match working {
                GimpSpace::LinearRgb => Encoding::Linear,
                GimpSpace::ProfileRgb => Encoding::NonlinearProfile,
                GimpSpace::PerceptualRgb => Encoding::Perceptual,
                GimpSpace::Lab => return Err(CompositionError::Invalid("working transfer")),
            };
            controlled::validate_interpretation(profile, to, control)?;
            // Exact curve-definition equality proves this cross-encoding
            // transfer is an identity. Parsed/quantized/table profiles are not
            // approximated to sRGB, and the resolved working-space kind remains.
            let perceptual = photocraft_cms::curve::srgb_trc();
            let identity_transfer =
                matches!((encoding(stored), to), (Encoding::NonlinearProfile, Encoding::Perceptual) | (Encoding::Perceptual, Encoding::NonlinearProfile))
                    && match profile.channels() {
                        3 => profile.trc.as_ref().is_some_and(|curves| curves.iter().all(|curve| curve == &perceptual)),
                        1 => profile.gray_trc.as_ref().is_some_and(|curve| curve == &perceptual),
                        _ => false,
                    };
            if identity_transfer {
                return Ok(Self { kind: working, forward: None, reverse: None, channels: profile.channels(), transfer: None, physical_lab: None, luminance });
            }
            return Ok(Self {
                kind: working,
                forward: None,
                reverse: None,
                channels: profile.channels(),
                transfer: Some((controlled::transfer_curves(profile, control)?, encoding(stored), to)),
                physical_lab: None,
                luminance,
            });
        }
        let source = photocraft_cms::xcf_color::effective_profile_from_definition_with_control(profile, encoding(stored),
            photocraft_cms::control::Limits::default(), &|| control.cancelled()).map_err(controlled::cms_error)?;
        // Builtin initialization remains a fixed boundary; its owned clone is controlled.
        let target = photocraft_cms::Builtin::LabD50.profile().try_clone_with_control(
            photocraft_cms::control::Limits::default(), &|| control.cancelled()).map_err(controlled::cms_error)?;
        // Physical Lab bypasses ICC v4 normalized Lab output clipping. Only
        // matrix/TRC RGB definitions are qualified for these component modes.
        let physical = source.channels() == 3
            && source.matrix.is_some() && source.trc.is_some()
            && source.a2b.iter().all(Option::is_none)
            && source.b2a.iter().all(Option::is_none);
        let (forward, reverse, physical_lab) = controlled::lab_bundle(&source, &target, physical, control)?;
        Ok(Self {
            kind: working,
            forward: Some(forward),
            reverse: Some(reverse),
            channels: profile.channels(),
            transfer: None,
            physical_lab,
            luminance,
        })
    }
    fn convert_physical_lab(&self, value: [f64; 3], forward: bool) -> Result<[f64; 3]> {
        let (to, from) = self.physical_lab.as_ref().ok_or(CompositionError::Unsupported("physical Lab matrix/TRC interpretation is not qualified"))?;
        let mut output = [0.0; 3];
        (if forward { to } else { from }).try_eval_f64(&value, &mut output)?;
        Ok(output)
    }
    pub(crate) fn convert(&self, rgb: [f64; 3], forward: bool) -> Result<[f64; 3]> {
        if let Some((curves, from, to)) = &self.transfer {
            let mut out = [0.0; 3];
            let (from, to) = if forward { (*from, *to) } else { (*to, *from) };
            for (channel, (&input, output)) in rgb
                .get(..self.channels)
                .ok_or(CompositionError::Invalid("transfer channels"))?
                .iter()
                .zip(out.get_mut(..self.channels).ok_or(CompositionError::Invalid("transfer channels"))?)
                .enumerate()
            {
                let profile_curve = curves.get(channel).and_then(Option::as_ref).ok_or(CompositionError::Invalid("missing prepared transfer curve"))?;
                let perceptual = photocraft_cms::curve::srgb_trc();
                let curve = |encoding| match encoding {
                    Encoding::Linear => None,
                    Encoding::NonlinearProfile => Some(profile_curve),
                    Encoding::Perceptual => Some(&perceptual),
                };
                let linear = curve(from).map_or(input, |curve| curve.eval64(input));
                *output = curve(to).map_or(linear, |curve| curve.eval_inverse64(linear));
                if !output.is_finite() {
                    return Err(CompositionError::Invalid("nonfinite working transfer output"));
                }
            }
            if self.channels == 1 {
                out = [out[0]; 3];
            }
            return Ok(out);
        }
        let Some(transform) = (if forward { &self.forward } else { &self.reverse }) else {
            return Ok(rgb);
        };
        let input = if self.channels == 1 && forward { &rgb[..1] } else { &rgb[..transform.inputs()] };
        let mut out = [0.0; 3];
        transform.try_eval_f64(input, out.get_mut(..transform.outputs()).ok_or(CompositionError::Invalid("working color channels"))?)?;
        if transform.outputs() == 1 {
            out = [out[0]; 3];
        }
        Ok(out)
    }
    pub fn normal(&self, backdrop: [f64; 4], source: [f64; 4], opacity: f64, operator: GimpOperator) -> Result<[f64; 4]> {
        validate_pixel(backdrop)?;
        validate_pixel(source)?;
        coverage(opacity)?;
        if operator == GimpOperator::Union && backdrop.iter().all(|value| value.to_bits() == 0) {
            return normal_over_zero_backdrop(source, opacity);
        }
        let ab = backdrop[3];
        let alpha = source[3] * opacity;
        if alpha == 0.0 && matches!(operator, GimpOperator::Union | GimpOperator::ClipToBackdrop) {
            return Ok(backdrop);
        }
        let ao = match operator {
            GimpOperator::Union => alpha + ab * (1.0 - alpha),
            GimpOperator::ClipToBackdrop => ab,
            GimpOperator::ClipToLayer => alpha,
            GimpOperator::Intersection => alpha * ab,
        };
        if ao == 0.0 {
            return Ok([0.0; 4]);
        }
        // Normal's overlap color is the source. Avoid unnecessary transfer/math on identity.
        if matches!(operator, GimpOperator::ClipToLayer | GimpOperator::Intersection) || ab == 0.0 || alpha == 1.0 {
            return Ok([source[0], source[1], source[2], ao]);
        }
        let b = self.convert([backdrop[0], backdrop[1], backdrop[2]], true)?;
        let s = self.convert([source[0], source[1], source[2]], true)?;
        let k = if operator == GimpOperator::ClipToBackdrop { alpha } else { alpha / ao };
        let mixed = std::array::from_fn(|i| b[i] * (1.0 - k) + s[i] * k);
        let rgb = self.convert(mixed, false)?;
        let output = [rgb[0], rgb[1], rgb[2], ao];
        validate_pixel(output)?;
        Ok(output)
    }
    /// Narrow finite-alpha Normal witness: full-opacity Union over transparent or opaque
    /// backdrop in linear working light. This is separate from unit-alpha layer composition.
    pub fn normal_finite_source(&self, backdrop: [f64; 4], source: [f64; 4], opacity: f64, operator: GimpOperator) -> Result<[f64; 4]> {
        validate_pixel(backdrop)?;
        coverage(opacity)?;
        if self.kind != GimpSpace::LinearRgb || operator != GimpOperator::Union || opacity != 1.0 || (backdrop[3] != 0.0 && backdrop[3] != 1.0) {
            return Err(CompositionError::Unsupported("finite source alpha requires full-opacity linear Normal Union and transparent/opaque backdrop"));
        }
        if source.iter().any(|value| !value.is_finite()) {
            return Err(CompositionError::Invalid("nonfinite source pixel"));
        }
        if backdrop[3] == 0.0 {
            return Ok(source);
        }
        let alpha = source[3].max(0.0);
        if alpha == 0.0 {
            return Ok(backdrop);
        }
        let b = self.convert([backdrop[0], backdrop[1], backdrop[2]], true)?;
        let source_rgb = self.convert([source[0], source[1], source[2]], true)?;
        let mixed = std::array::from_fn(|channel| b[channel] * (1.0 - alpha) + source_rgb[channel] * alpha);
        let rgb = self.convert(mixed, false)?;
        let output = [rgb[0], rgb[1], rgb[2], 1.0];
        validate_pixel(output)?;
        Ok(output)
    }
    pub fn group_mix(&self, before: [f64; 4], after: [f64; 4], amount: f64) -> Result<[f64; 4]> {
        validate_pixel(before)?;
        validate_pixel(after)?;
        coverage(amount)?;
        if amount == 0.0 {
            return Ok(before);
        }
        if amount == 1.0 {
            return Ok(after);
        }
        let wa = before[3] * (1.0 - amount);
        let wb = after[3] * amount;
        let alpha = wa + wb;
        if alpha == 0.0 {
            return Ok([0.0; 4]);
        }
        let a = self.convert([before[0], before[1], before[2]], true)?;
        let b = self.convert([after[0], after[1], after[2]], true)?;
        let rgb = self.convert(std::array::from_fn(|i| (a[i] * wa + b[i] * wb) / alpha), false)?;
        let result = [rgb[0], rgb[1], rgb[2], alpha];
        validate_pixel(result)?;
        Ok(result)
    }
    /// Modern separable Multiply with distinct blend and compositing spaces.
    /// Only the overlap is multiplied; coverage operators select the retained zones.
    pub fn multiply(&self, blend: &WorkingSpace, backdrop: [f64; 4], source: [f64; 4], opacity: f64, operator: GimpOperator) -> Result<[f64; 4]> {
        self.separable(blend, backdrop, source, opacity, operator, crate::gimp_blend::GimpBlendMode::Multiply)
    }
    /// Checked mode equations, including scoped RGB HSV/HSL component replacement.
    /// Space/operator and finite sample domains are checked before composition.
    pub fn separable(
        &self,
        blend: &WorkingSpace,
        backdrop: [f64; 4],
        source: [f64; 4],
        opacity: f64,
        operator: GimpOperator,
        mode: crate::gimp_blend::GimpBlendMode,
    ) -> Result<[f64; 4]> {
        use crate::gimp_blend::GimpBlendMode;
        if mode == GimpBlendMode::Dissolve {
            if self.channels != 3 || operator != GimpOperator::Union {
                return Err(CompositionError::Unsupported("Dissolve RGB Union endpoint policy is not qualified"));
            }
            return crate::gimp_blend::dissolve_endpoint(backdrop, source, opacity);
        }
        if mode == GimpBlendMode::ColorErase {
            validate_pixel(backdrop)?;
            validate_pixel(source)?;
            coverage(opacity)?;
            if !matches!(self.channels, 1 | 3)
                || blend.channels != self.channels
                || (self.channels == 1 && [backdrop, source].iter().any(|p| p[0].to_bits() != p[1].to_bits() || p[0].to_bits() != p[2].to_bits()))
                || self.kind != GimpSpace::LinearRgb
                || blend.kind != GimpSpace::LinearRgb
                || operator != GimpOperator::ClipToBackdrop
            {
                return Err(CompositionError::Unsupported("Color Erase RGB/Gray linear space/operator domain is not qualified"));
            }
            if backdrop[..3].iter().chain(source[..3].iter()).any(|v| !(0.0..=1.0).contains(v)) {
                return Err(CompositionError::Unsupported("Color Erase stored unit RGB domain is not qualified"));
            }
            let alpha = source[3] * opacity;
            if alpha == 0.0 {
                return Ok(backdrop);
            }
            let b = self.convert([backdrop[0], backdrop[1], backdrop[2]], true)?;
            let s = self.convert([source[0], source[1], source[2]], true)?;
            let mixed = crate::gimp_blend::erase_color([b[0], b[1], b[2], backdrop[3]], [s[0], s[1], s[2], alpha])?;
            if mixed[3] == 0.0 {
                return Ok(mixed);
            }
            let rgb = self.convert([mixed[0], mixed[1], mixed[2]], false)?;
            let output = [rgb[0], rgb[1], rgb[2], mixed[3]];
            validate_pixel(output)?;
            return Ok(output);
        }
        if matches!(mode, GimpBlendMode::Erase | GimpBlendMode::Merge | GimpBlendMode::Split) {
            validate_pixel(backdrop)?;
            validate_pixel(source)?;
            coverage(opacity)?;
            let expected = if mode == GimpBlendMode::Merge { GimpOperator::Union } else { GimpOperator::ClipToBackdrop };
            if !matches!(self.channels, 1 | 3)
                || (self.channels == 1 && [backdrop, source].iter().any(|p| p[0].to_bits() != p[1].to_bits() || p[0].to_bits() != p[2].to_bits()))
                || operator != expected
                || (mode == GimpBlendMode::Merge && self.kind != GimpSpace::LinearRgb)
            {
                return Err(CompositionError::Unsupported("deterministic alpha RGB operator/space is not qualified"));
            }
            if backdrop[..3].iter().chain(source[..3].iter()).any(|v| !(0.0..=1.0).contains(v)) {
                return Err(CompositionError::Unsupported("deterministic alpha stored unit RGB domain is not qualified"));
            }
            let alpha = source[3] * opacity;
            if alpha == 0.0 {
                return Ok(backdrop);
            }
            if matches!(mode, GimpBlendMode::Erase | GimpBlendMode::Split) {
                return crate::gimp_blend::alpha_components(mode, backdrop, [source[0], source[1], source[2], alpha]);
            }
            let b = self.convert([backdrop[0], backdrop[1], backdrop[2]], true)?;
            let s = self.convert([source[0], source[1], source[2]], true)?;
            let mixed = crate::gimp_blend::alpha_components(mode, [b[0], b[1], b[2], backdrop[3]], [s[0], s[1], s[2], alpha])?;
            if mixed[3] == 0.0 {
                return Ok(mixed);
            }
            let rgb = self.convert([mixed[0], mixed[1], mixed[2]], false)?;
            let result = [rgb[0], rgb[1], rgb[2], mixed[3]];
            validate_pixel(result)?;
            return Ok(result);
        }
        let luma_mode = matches!(mode, GimpBlendMode::LumaDarken | GimpBlendMode::LumaLighten | GimpBlendMode::Luminance);
        if luma_mode
            && (self.channels != 3
                || blend.channels != 3
                || self.kind != GimpSpace::LinearRgb
                || blend.kind != GimpSpace::LinearRgb
                || operator != GimpOperator::ClipToBackdrop
                || blend.luminance.is_none())
        {
            return Err(CompositionError::Unsupported("native linear RGB luminance clip-to-backdrop domain is not qualified"));
        }
        let lch_mode = matches!(mode, GimpBlendMode::LchHue | GimpBlendMode::LchChroma | GimpBlendMode::LchColor | GimpBlendMode::LchLightness);
        if lch_mode
            && (self.channels != 3
                || blend.channels != 3
                || self.kind != GimpSpace::LinearRgb
                || blend.kind != GimpSpace::Lab
                || operator != GimpOperator::ClipToBackdrop
                || blend.physical_lab.is_none())
        {
            return Err(CompositionError::Unsupported("LCh Lab blend/linear RGB clip-to-backdrop domain is not qualified"));
        }
        let legacy_component = match mode {
            GimpBlendMode::LegacyHsvHue => Some(GimpBlendMode::HsvHue),
            GimpBlendMode::LegacyHsvSaturation => Some(GimpBlendMode::HsvSaturation),
            GimpBlendMode::LegacyHslColor => Some(GimpBlendMode::HslColor),
            GimpBlendMode::LegacyHsvValue => Some(GimpBlendMode::HsvValue),
            _ => None,
        };
        if legacy_component.is_some() && (self.channels != 3 || blend.channels != 3) {
            return Err(CompositionError::Unsupported("legacy HSV/HSL RGB model is not qualified"));
        }
        let component_mode = matches!(mode, GimpBlendMode::HsvHue | GimpBlendMode::HsvSaturation | GimpBlendMode::HslColor | GimpBlendMode::HsvValue);
        if component_mode
            && (self.channels != 3
                || blend.channels != 3
                || self.kind != GimpSpace::LinearRgb
                || blend.kind != GimpSpace::ProfileRgb
                || operator != GimpOperator::ClipToBackdrop)
        {
            return Err(CompositionError::Unsupported("HSV/HSL RGB profile-blend linear clip-to-backdrop domain is not qualified"));
        }
        let legacy = matches!(
            mode,
            GimpBlendMode::LegacyMultiply
                | GimpBlendMode::LegacyScreen
                | GimpBlendMode::LegacyDifference
                | GimpBlendMode::LegacyAddition
                | GimpBlendMode::LegacySubtract
                | GimpBlendMode::LegacyDarkenOnly
                | GimpBlendMode::LegacyLightenOnly
                | GimpBlendMode::LegacyDivide
                | GimpBlendMode::LegacyDodge
                | GimpBlendMode::LegacyBurn
                | GimpBlendMode::LegacyHardLight
                | GimpBlendMode::LegacyGrainExtract
                | GimpBlendMode::LegacyGrainMerge
                | GimpBlendMode::LegacySoftLight
                | GimpBlendMode::LegacyHsvHue
                | GimpBlendMode::LegacyHsvSaturation
                | GimpBlendMode::LegacyHslColor
                | GimpBlendMode::LegacyHsvValue
        );
        let legacy_contrast = matches!(
            mode,
            GimpBlendMode::LegacyDivide
                | GimpBlendMode::LegacyDodge
                | GimpBlendMode::LegacyBurn
                | GimpBlendMode::LegacyHardLight
                | GimpBlendMode::LegacyGrainExtract
                | GimpBlendMode::LegacyGrainMerge
        );
        if legacy_contrast && (self.channels != 3 || blend.channels != 3) {
            return Err(CompositionError::Unsupported("legacy contrast RGB domain is not qualified"));
        }
        if legacy && (operator != GimpOperator::ClipToBackdrop || self.kind != GimpSpace::ProfileRgb || blend.kind != GimpSpace::ProfileRgb) {
            return Err(CompositionError::Unsupported("legacy separable coverage operator is not qualified"));
        }
        if mode == GimpBlendMode::Normal {
            return Err(CompositionError::Unsupported("Normal uses its dedicated coverage evaluator"));
        }
        validate_pixel(backdrop)?;
        validate_pixel(source)?;
        if (component_mode || legacy_component.is_some() || lch_mode || luma_mode || legacy_contrast)
            && backdrop[..3].iter().chain(source[..3].iter()).any(|v| !(0.0..=1.0).contains(v))
        {
            return Err(CompositionError::Unsupported("component blend stored unit RGB domain is not qualified"));
        }

        coverage(opacity)?;
        let ab = backdrop[3];
        let alpha = source[3] * opacity;
        let ao = match operator {
            GimpOperator::Union => alpha + ab * (1.0 - alpha),
            GimpOperator::ClipToBackdrop => ab,
            GimpOperator::ClipToLayer => alpha,
            GimpOperator::Intersection => alpha * ab,
        };
        if ao == 0.0 {
            return Ok([0.0; 4]);
        }
        if alpha == 0.0 {
            return Ok([backdrop[0], backdrop[1], backdrop[2], ao]);
        }
        if ab == 0.0 {
            return Ok([source[0], source[1], source[2], ao]);
        }
        let convert_blend = |rgb, forward| if lch_mode { blend.convert_physical_lab(rgb, forward) } else { blend.convert(rgb, forward) };
        let b = convert_blend([backdrop[0], backdrop[1], backdrop[2]], true)?;
        let s = convert_blend([source[0], source[1], source[2]], true)?;
        if legacy && b.iter().chain(s.iter()).any(|v| !(0.0..=1.0).contains(v)) {
            return Err(CompositionError::Unsupported("legacy unit blend domain is not qualified"));
        }
        if matches!(
            mode,
            GimpBlendMode::HardLight
                | GimpBlendMode::HardMix
                | GimpBlendMode::SoftLight
                | GimpBlendMode::PinLight
                | GimpBlendMode::ColorDodge
                | GimpBlendMode::ColorBurn
                | GimpBlendMode::VividLight
                | GimpBlendMode::Divide
                | GimpBlendMode::LinearBurn
        ) && b.iter().chain(s.iter()).any(|v| !(0.0..=1.0).contains(v))
        {
            return Err(CompositionError::Unsupported("bounded contrast HDR blend domain is not qualified"));
        }
        if mode == GimpBlendMode::ColorDodge && s.contains(&1.0) {
            return Err(CompositionError::Unsupported("Color Dodge singular endpoint is not qualified"));
        }
        if mode == GimpBlendMode::ColorBurn && s.contains(&0.0) {
            return Err(CompositionError::Unsupported("Color Burn singular endpoint is not qualified"));
        }
        if matches!(mode, GimpBlendMode::Divide | GimpBlendMode::VividLight) && s.contains(&0.0) {
            return Err(CompositionError::Unsupported("singular blend endpoint is not qualified"));
        }
        if mode == GimpBlendMode::VividLight && s.contains(&1.0) {
            return Err(CompositionError::Unsupported("Vivid Light singular endpoint is not qualified"));
        }
        let product = if let Some(component) = legacy_component {
            crate::gimp_blend::components(component, b, s)?
        } else if luma_mode {
            crate::gimp_blend::luminance_components(mode, b, s, blend.luminance.ok_or(CompositionError::Unsupported("missing luminance basis"))?)?
        } else if lch_mode {
            crate::gimp_blend::lch_components(mode, b, s)?
        } else if matches!(mode, GimpBlendMode::HsvHue | GimpBlendMode::HsvSaturation | GimpBlendMode::HslColor | GimpBlendMode::HsvValue) {
            crate::gimp_blend::components(mode, b, s)?
        } else {
            std::array::from_fn(|i| match mode {
                GimpBlendMode::Multiply | GimpBlendMode::LegacyMultiply => b[i] * s[i],
                // Algebraic Screen preserves tiny nonzero samples with a zero source.
                GimpBlendMode::Screen | GimpBlendMode::LegacyScreen => b[i] + s[i] - b[i] * s[i],
                GimpBlendMode::Difference | GimpBlendMode::LegacyDifference => (b[i] - s[i]).abs(),
                GimpBlendMode::Addition => b[i] + s[i],
                GimpBlendMode::LegacyAddition => (b[i] + s[i]).min(1.0),
                GimpBlendMode::Subtract => b[i] - s[i],
                GimpBlendMode::LegacySubtract => (b[i] - s[i]).max(0.0),
                GimpBlendMode::DarkenOnly | GimpBlendMode::LegacyDarkenOnly => b[i].min(s[i]),
                GimpBlendMode::LightenOnly | GimpBlendMode::LegacyLightenOnly => b[i].max(s[i]),
                GimpBlendMode::LegacyDivide => {
                    if b[i] == 0.0 {
                        0.0
                    } else if s[i] == 0.0 {
                        1.0
                    } else {
                        (b[i] / s[i]).min(1.0)
                    }
                }
                GimpBlendMode::LegacyDodge => {
                    if b[i] == 0.0 {
                        0.0
                    } else if s[i] == 1.0 {
                        1.0
                    } else {
                        (b[i] / (1.0 - s[i])).min(1.0)
                    }
                }
                GimpBlendMode::LegacyBurn => {
                    if b[i] == 1.0 {
                        1.0
                    } else if s[i] == 0.0 {
                        0.0
                    } else {
                        (1.0 - (1.0 - b[i]) / s[i]).max(0.0)
                    }
                }
                GimpBlendMode::LegacyHardLight => {
                    // The public byte-domain high branch includes 128's offset;
                    // saved FLOAT_NON_LINEAR observations retain that offset.
                    if s[i] <= f64::from(128.0_f32 / 255.0_f32) { (2.0 * b[i] * s[i]).min(1.0) } else { 1.0 - (511.0 / 255.0 - 2.0 * s[i]) * (1.0 - b[i]) }
                }
                GimpBlendMode::GrainExtract => b[i] - s[i] + 0.5,
                GimpBlendMode::LegacyGrainExtract => (b[i] - s[i] + 128.0 / 255.0).clamp(0.0, 1.0),
                GimpBlendMode::LegacyGrainMerge => (b[i] + s[i] - 128.0 / 255.0).clamp(0.0, 1.0),
                GimpBlendMode::GrainMerge => b[i] + s[i] - 0.5,
                GimpBlendMode::LinearLight => b[i] + 2.0 * s[i] - 1.0,
                GimpBlendMode::LinearBurn => b[i] + s[i] - 1.0,
                GimpBlendMode::Divide => b[i] / s[i],
                // GIMP public manual: channel sum >= 1 selects one, otherwise zero.
                GimpBlendMode::HardMix => {
                    if b[i] + s[i] >= 1.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
                GimpBlendMode::VividLight => {
                    if s[i] <= 0.5 {
                        (1.0 - (1.0 - b[i]) / (2.0 * s[i])).max(0.0)
                    } else {
                        (b[i] / (2.0 * (1.0 - s[i]))).min(1.0)
                    }
                }
                GimpBlendMode::ColorBurn => 1.0 - (1.0 - b[i]) / s[i],
                GimpBlendMode::ColorDodge => b[i] / (1.0 - s[i]),
                GimpBlendMode::PinLight => {
                    if s[i] <= 0.5 {
                        b[i].min(2.0 * s[i])
                    } else {
                        b[i].max(2.0 * (s[i] - 0.5))
                    }
                }
                GimpBlendMode::SoftLight | GimpBlendMode::LegacySoftLight => {
                    if s[i] == 0.5 {
                        b[i]
                    } else {
                        b[i] * b[i] + 2.0 * b[i] * s[i] * (1.0 - b[i])
                    }
                }
                GimpBlendMode::HardLight => {
                    if s[i] <= 0.5 {
                        2.0 * b[i] * s[i]
                    } else {
                        1.0 - 2.0 * (1.0 - b[i]) * (1.0 - s[i])
                    }
                }
                GimpBlendMode::Exclusion => b[i] + s[i] - 2.0 * b[i] * s[i],
                GimpBlendMode::Overlay => {
                    if b[i] < 0.5 {
                        2.0 * b[i] * s[i]
                    } else {
                        1.0 - 2.0 * (1.0 - b[i]) * (1.0 - s[i])
                    }
                }
                GimpBlendMode::Erase
                | GimpBlendMode::Dissolve
                | GimpBlendMode::ColorErase
                | GimpBlendMode::Merge
                | GimpBlendMode::Split
                | GimpBlendMode::LegacyHsvHue
                | GimpBlendMode::LegacyHsvSaturation
                | GimpBlendMode::LegacyHslColor
                | GimpBlendMode::LegacyHsvValue
                | GimpBlendMode::LumaDarken
                | GimpBlendMode::LumaLighten
                | GimpBlendMode::Luminance
                | GimpBlendMode::LchHue
                | GimpBlendMode::LchChroma
                | GimpBlendMode::LchColor
                | GimpBlendMode::LchLightness
                | GimpBlendMode::Normal
                | GimpBlendMode::HsvHue
                | GimpBlendMode::HsvSaturation
                | GimpBlendMode::HslColor
                | GimpBlendMode::HsvValue => 0.0,
            })
        };
        if product.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Invalid("separable blend overflow"));
        }
        let stored = convert_blend(product, false)?;
        let mixed = self.convert(stored, true)?;
        let b = if matches!(operator, GimpOperator::Union | GimpOperator::ClipToBackdrop) {
            self.convert([backdrop[0], backdrop[1], backdrop[2]], true)?
        } else {
            [0.0; 3]
        };
        let s =
            if matches!(operator, GimpOperator::Union | GimpOperator::ClipToLayer) { self.convert([source[0], source[1], source[2]], true)? } else { [0.0; 3] };
        if legacy {
            // Authored legacy references retain backdrop alpha. Their overlap
            // coverage uses the lesser native alpha before layer opacity.
            let overlap = ab.min(source[3]) * opacity;
            let denominator = ab + (1.0 - ab) * overlap;
            if denominator <= 0.0 || !denominator.is_finite() {
                return Err(CompositionError::Invalid("legacy overlap denominator"));
            }
            let weight = overlap / denominator;
            let colors = std::array::from_fn(|i| if b[i] == mixed[i] { b[i] } else { b[i] * (1.0 - weight) + mixed[i] * weight });
            let rgb = self.convert(colors, false)?;
            let output = [rgb[0], rgb[1], rgb[2], ab];
            validate_pixel(output)?;
            return Ok(output);
        }
        let colors = std::array::from_fn(|i| match operator {
            GimpOperator::Union => (ab * (1.0 - alpha) * b[i] + alpha * (1.0 - ab) * s[i] + ab * alpha * mixed[i]) / ao,
            GimpOperator::ClipToBackdrop => (1.0 - alpha) * b[i] + alpha * mixed[i],
            GimpOperator::ClipToLayer => (1.0 - ab) * s[i] + ab * mixed[i],
            GimpOperator::Intersection => mixed[i],
        });
        let rgb = self.convert(colors, false)?;
        let output = [rgb[0], rgb[1], rgb[2], ao];
        validate_pixel(output)?;
        Ok(output)
    }
}
/// Pure existing Normal/Union identity over an all-bit-positive-zero backdrop.
/// The caller has selected that exact operator/backdrop. This performs no color
/// transfer and grants no permission to substitute it for another composition.
pub(crate) fn normal_over_zero_backdrop(source: [f64; 4], opacity: f64) -> Result<[f64; 4]> {
    validate_pixel(source)?;
    coverage(opacity)?;
    let alpha = source[3] * opacity;
    if alpha == 0.0 {
        return Ok([0.0; 4]);
    }
    Ok([source[0], source[1], source[2], alpha])
}

pub fn coverage(value: f64) -> Result<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) { Ok(()) } else { Err(CompositionError::Invalid("coverage outside finite unit interval")) }
}
fn validate_pixel(pixel: [f64; 4]) -> Result<()> {
    if pixel.iter().any(|value| !value.is_finite()) {
        return Err(CompositionError::Invalid("nonfinite color sample"));
    }
    coverage(pixel[3])
}

#[cfg(test)]
mod physical_lch_tests {
    use super::*;
    #[test]
    fn physical_lab_reverse_retains_out_of_gamut_signed_rgb_without_icc_clipping() {
        let lab = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::Lab).unwrap();
        let rgb = lab.convert_physical_lab([53.0, 200.0, -300.0], false).unwrap();
        assert!(rgb.iter().all(|v| v.is_finite()));
        assert!(rgb.iter().any(|v| *v < 0.0));
        assert!(rgb.iter().any(|v| *v > 1.0));
        let marker = 53.0 + 2.0_f64.powi(-35);
        let a = lab.convert_physical_lab([53.0, 30.0, -20.0], false).unwrap();
        let b = lab.convert_physical_lab([marker, 30.0, -20.0], false).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn lch_checked_context_rejects_gray_wrong_spaces_coverage_and_hidden_hdr() {
        use crate::gimp_blend::GimpBlendMode;
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let blend = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::Lab).unwrap();
        let composite = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [GimpBlendMode::LchHue, GimpBlendMode::LchChroma, GimpBlendMode::LchColor, GimpBlendMode::LchLightness] {
            let b = [0.2, 0.4, 0.8, 0.5];
            let s = [0.7, 0.3, 0.1, 0.4];
            assert_eq!(composite.separable(&blend, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).unwrap()[3], 0.5);
            assert!(composite.separable(&blend, b, s, 0.75, GimpOperator::Union, mode).is_err());
            assert!(blend.separable(&blend, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(gray.separable(&blend, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(composite.separable(&blend, b, [-1.0, 0.0, 0.0, 0.0], 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
        }
    }
    #[test]
    #[ignore = "24MP release physical LCh composition throughput measurement"]
    fn lch_composition_24mp_release() {
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let blend = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::Lab).unwrap();
        let composite = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [
            crate::gimp_blend::GimpBlendMode::LchHue,
            crate::gimp_blend::GimpBlendMode::LchChroma,
            crate::gimp_blend::GimpBlendMode::LchColor,
            crate::gimp_blend::GimpBlendMode::LchLightness,
        ] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(composite.separable(&blend, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn luminance_context_rejects_other_models_spaces_coverage_and_hidden_hdr() {
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let linear = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let nonlinear = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [crate::gimp_blend::GimpBlendMode::LumaDarken, crate::gimp_blend::GimpBlendMode::LumaLighten, crate::gimp_blend::GimpBlendMode::Luminance] {
            let b = [0.2, 0.4, 0.8, 0.5];
            let s = [0.7, 0.3, 0.1, 0.4];
            assert_eq!(linear.separable(&linear, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).unwrap()[3], 0.5);
            assert!(linear.separable(&nonlinear, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(linear.separable(&linear, b, s, 0.75, GimpOperator::Union, mode).is_err());
            assert!(gray.separable(&linear, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(linear.separable(&linear, b, [-1.0, 0.0, 0.0, 0.0], 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
        }
    }
    #[test]
    #[ignore = "24MP release luminance composition throughput measurement"]
    fn luminance_composition_24mp_release() {
        let linear = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [crate::gimp_blend::GimpBlendMode::LumaDarken, crate::gimp_blend::GimpBlendMode::LumaLighten, crate::gimp_blend::GimpBlendMode::Luminance] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(linear.separable(&linear, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn gray_alpha_color_domain_requires_single_component_and_retains_precision() {
        use crate::gimp_blend::GimpBlendMode;
        let profile = photocraft_cms::Builtin::SGray.profile();
        let linear = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let encoded = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let marker = 0.5 + 2.0_f64.powi(-40);
        let b = [marker, marker, marker, 0.75];
        let source = [0.2, 0.2, 0.2, 0.5];
        for mode in [GimpBlendMode::Erase, GimpBlendMode::Split] {
            let result = encoded.separable(&linear, b, source, 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
            assert_eq!(&result[..3], &b[..3]);
            assert_eq!(result[3], if mode == GimpBlendMode::Erase { 0.5625 } else { 0.5 });
            assert!(encoded.separable(&linear, b, [0.2, 0.3, 0.2, 0.5], 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
        }
        for (mode, operator) in [(GimpBlendMode::Merge, GimpOperator::Union), (GimpBlendMode::ColorErase, GimpOperator::ClipToBackdrop)] {
            let result = linear.separable(&linear, b, source, 0.5, operator, mode).unwrap();
            assert_eq!(result[0].to_bits(), result[1].to_bits());
            assert_eq!(result[0].to_bits(), result[2].to_bits());
            assert_eq!(linear.separable(&linear, b, source, 0.0, operator, mode).unwrap(), b);
            assert!(linear.separable(&linear, b, [-1.0, -1.0, -1.0, 0.0], 0.5, operator, mode).is_err());
        }
        assert!(linear.separable(&linear, b, source, 1.0, GimpOperator::Union, GimpBlendMode::Dissolve).is_err());
    }
    #[test]
    #[ignore = "24MP release Gray alpha/color composition throughput measurement"]
    fn gray_alpha_composition_24mp_release() {
        let linear = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [
            crate::gimp_blend::GimpBlendMode::Erase,
            crate::gimp_blend::GimpBlendMode::Merge,
            crate::gimp_blend::GimpBlendMode::Split,
            crate::gimp_blend::GimpBlendMode::ColorErase,
        ] {
            let operator = if mode == crate::gimp_blend::GimpBlendMode::Merge { GimpOperator::Union } else { GimpOperator::ClipToBackdrop };
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let value = 0.15 + f64::from(i % 128) * 0.001;
                let b = std::hint::black_box([value, value, value, 0.7]);
                std::hint::black_box(linear.separable(&linear, b, [0.8, 0.8, 0.8, 0.6], 0.7125, operator, mode).unwrap());
            }
            println!("Gray {mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn color_erase_profile_transfer_and_domain_guards() {
        use crate::gimp_blend::GimpBlendMode;
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let linear = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let encoded = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let marker = 0.5 + 2.0_f64.powi(-40);
        let b = [marker, 0.2, 0.8, 0.75];
        let s = [0.8, 0.1, 0.2, 0.5];
        assert_eq!(linear.separable(&linear, b, s, 0.0, GimpOperator::ClipToBackdrop, GimpBlendMode::ColorErase).unwrap(), b);
        assert_eq!(linear.separable(&linear, b, b, 1.0, GimpOperator::ClipToBackdrop, GimpBlendMode::ColorErase).unwrap()[3], 0.1875);
        assert!(linear.separable(&encoded, b, s, 0.5, GimpOperator::ClipToBackdrop, GimpBlendMode::ColorErase).is_err());
        assert!(encoded.separable(&linear, b, s, 0.5, GimpOperator::ClipToBackdrop, GimpBlendMode::ColorErase).is_err());
        assert!(linear.separable(&linear, b, s, 0.5, GimpOperator::Union, GimpBlendMode::ColorErase).is_err());
        assert!(gray.separable(&linear, b, s, 0.5, GimpOperator::ClipToBackdrop, GimpBlendMode::ColorErase).is_err());
        assert!(linear.separable(&linear, b, [-1.0, 0.0, 0.0, 0.0], 0.5, GimpOperator::ClipToBackdrop, GimpBlendMode::ColorErase).is_err());
    }
    #[test]
    #[ignore = "24MP release Color Erase composition throughput measurement"]
    fn color_erase_composition_24mp_release() {
        let linear = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let start = std::time::Instant::now();
        for i in 0..24_000_000 {
            let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
            std::hint::black_box(
                linear
                    .separable(&linear, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, crate::gimp_blend::GimpBlendMode::ColorErase)
                    .unwrap(),
            );
        }
        println!("ColorErase: 24MP {:?}", start.elapsed());
    }
    #[test]
    fn deterministic_alpha_coverage_preserves_color_and_checks_domains() {
        use crate::gimp_blend::GimpBlendMode;
        let linear = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let marker = 0.5 + 2.0_f64.powi(-40);
        let b = [marker, 0.2, 0.8, 0.75];
        let s = [0.9, 0.1, 0.4, 0.5];
        for (mode, expected_alpha) in [(GimpBlendMode::Erase, 0.5625), (GimpBlendMode::Split, 0.5)] {
            let actual = profile.separable(&linear, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
            assert_eq!(actual, [marker, 0.2, 0.8, expected_alpha]);
            assert_eq!(linear.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).unwrap(), actual);
            assert_eq!(profile.separable(&profile, b, s, 0.0, GimpOperator::ClipToBackdrop, mode).unwrap(), b);
            assert_eq!(profile.separable(&profile, b, [0.1, 0.2, 0.3, 1.0], 1.0, GimpOperator::ClipToBackdrop, mode).unwrap(), [0.0; 4]);
            assert!(profile.separable(&profile, b, s, 0.5, GimpOperator::Union, mode).is_err());
            assert!(gray.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(profile.separable(&profile, b, [-1.0, 0.2, 0.3, 0.0], 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(profile.separable(&profile, b, s, f64::NAN, GimpOperator::ClipToBackdrop, mode).is_err());
        }
        let rgb = [0.2, 0.4, 0.8, 0.75];
        let merged = linear.separable(&profile, rgb, [0.8, 0.2, 0.1, 0.5], 0.5, GimpOperator::Union, GimpBlendMode::Merge).unwrap();
        assert_eq!(merged[3], 1.0);
        let b_linear = linear.convert([0.2, 0.4, 0.8], true).unwrap();
        let s_linear = linear.convert([0.8, 0.2, 0.1], true).unwrap();
        let expected = linear.convert(std::array::from_fn(|i| 0.75 * b_linear[i] + 0.25 * s_linear[i]), false).unwrap();
        for i in 0..3 {
            assert!((merged[i] - expected[i]).abs() < 1e-15);
        }
        // Coverage beyond one clips backdrop contribution, retaining source-over color.
        assert_eq!(crate::gimp_blend::alpha_components(GimpBlendMode::Merge, [0.0, 0.0, 0.0, 0.8], [1.0, 1.0, 1.0, 0.8]).unwrap(), [0.8, 0.8, 0.8, 1.0]);
        assert!(profile.separable(&profile, b, s, 0.5, GimpOperator::Union, GimpBlendMode::Merge).is_err());
        assert!(linear.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, GimpBlendMode::Merge).is_err());
        assert!(crate::gimp_blend::dissolve_endpoint(b, s, 0.5).is_err());
        assert!(crate::gimp_blend::mode(22).is_err());
    }
    #[test]
    #[ignore = "24MP release deterministic alpha composition throughput measurement"]
    fn alpha_composition_24mp_release() {
        let linear = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [crate::gimp_blend::GimpBlendMode::Erase, crate::gimp_blend::GimpBlendMode::Merge, crate::gimp_blend::GimpBlendMode::Split] {
            let operator = if mode == crate::gimp_blend::GimpBlendMode::Merge { GimpOperator::Union } else { GimpOperator::ClipToBackdrop };
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(linear.separable(&linear, b, [0.8, 0.25, 0.4, 0.6], 0.7125, operator, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn legacy_hsv_keeps_legacy_coverage_and_checked_rgb_domain() {
        use crate::gimp_blend::GimpBlendMode;
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let marker = 0.5 + 2.0_f64.powi(-40);
        for mode in [GimpBlendMode::LegacyHsvHue, GimpBlendMode::LegacyHsvSaturation, GimpBlendMode::LegacyHslColor, GimpBlendMode::LegacyHsvValue] {
            let out =
                profile.separable(&profile, [marker, marker, marker, 0.75], [marker, marker, marker, 0.5], 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
            assert_eq!(out, [marker, marker, marker, 0.75]);
            assert!(profile.separable(&profile, [0.2, 0.4, 0.8, 0.75], [0.7, 0.3, 0.1, 0.5], 0.5, GimpOperator::Union, mode).is_err());
            assert!(gray.separable(&profile, [0.2, 0.4, 0.8, 0.75], [0.7, 0.3, 0.1, 0.5], 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(profile.separable(&profile, [0.2, 0.4, 0.8, 0.75], [-1.0, 0.0, 0.0, 0.0], 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
        }
        let b = [0.2, 0.4, 0.8, 0.75];
        let source = [0.8, 0.2, 0.4, 0.5];
        let output = profile.separable(&profile, b, source, 0.5, GimpOperator::ClipToBackdrop, GimpBlendMode::LegacyHsvHue).unwrap();
        let mixed = crate::gimp_blend::components(GimpBlendMode::HsvHue, [b[0], b[1], b[2]], [source[0], source[1], source[2]]).unwrap();
        let overlap = 0.25;
        let weight = overlap / (0.75 + 0.25 * overlap);
        for channel in 0..3 {
            assert!((output[channel] - (b[channel] * (1.0 - weight) + mixed[channel] * weight)).abs() < 1e-15);
        }
        assert_eq!(output[3], 0.75);
    }
    #[test]
    #[ignore = "24MP release legacy HSV composition throughput measurement"]
    fn legacy_hsv_composition_24mp_release() {
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        for mode in [
            crate::gimp_blend::GimpBlendMode::LegacyHsvHue,
            crate::gimp_blend::GimpBlendMode::LegacyHsvSaturation,
            crate::gimp_blend::GimpBlendMode::LegacyHslColor,
            crate::gimp_blend::GimpBlendMode::LegacyHsvValue,
        ] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(profile.separable(&profile, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn legacy_grain_byte_midpoint_clamps_and_retains_checked_coverage() {
        use crate::gimp_blend::GimpBlendMode;
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let marker = 0.5 + 2.0_f64.powi(-40);
        let b = [marker, 0.0, 1.0, 0.75];
        let s = [128.0 / 255.0, 1.0, 0.0, 0.5];
        for mode in [GimpBlendMode::LegacyGrainExtract, GimpBlendMode::LegacyGrainMerge] {
            let actual = profile.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).unwrap();
            assert_eq!(actual[0], marker);
            assert_eq!(actual[3], b[3]);
            let weight = 0.25 / (0.75 + 0.25 * 0.25);
            let product = if mode == GimpBlendMode::LegacyGrainExtract { [marker, 0.0, 1.0] } else { [marker, 1.0 - 128.0 / 255.0, 1.0 - 128.0 / 255.0] };
            for c in 0..3 {
                assert!((actual[c] - (b[c] * (1.0 - weight) + product[c] * weight)).abs() < 1e-15);
            }
            assert_eq!(profile.separable(&profile, b, s, 0.0, GimpOperator::ClipToBackdrop, mode).unwrap(), b);
            assert!(profile.separable(&profile, b, s, 0.5, GimpOperator::Union, mode).is_err());
            assert!(gray.separable(&profile, b, s, 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(profile.separable(&profile, b, [f64::NAN, 0.0, 1.0, 0.5], 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(profile.separable(&profile, b, [-0.1, 0.0, 1.0, 0.5], 0.5, GimpOperator::ClipToBackdrop, mode).is_err());
        }
    }
    #[test]
    #[ignore = "24MP release legacy grain composition throughput measurement"]
    fn legacy_grain_composition_24mp_release() {
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        for mode in [crate::gimp_blend::GimpBlendMode::LegacyGrainExtract, crate::gimp_blend::GimpBlendMode::LegacyGrainMerge] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(profile.separable(&profile, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn legacy_contrast_normalized_endpoints_and_native_precision_are_checked() {
        use crate::gimp_blend::GimpBlendMode;
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let gray = WorkingSpace::new(photocraft_cms::Builtin::SGray.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        for (mode, b, s, expected) in [
            (GimpBlendMode::LegacyDivide, [0.0, 0.25, 1.0], [0.0, 0.0, 0.0], [0.0, 1.0, 1.0]),
            (GimpBlendMode::LegacyDodge, [0.0, 0.25, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]),
            (GimpBlendMode::LegacyBurn, [0.0, 0.25, 1.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            (GimpBlendMode::LegacyHardLight, [0.0, 0.5, 1.0], [1.0, 1.0, 1.0], [1.0 - 1.0 / 255.0, 1.0 - 0.5 / 255.0, 1.0]),
        ] {
            let back = [b[0], b[1], b[2], 1.0];
            let upper = [s[0], s[1], s[2], 1.0];
            let out = profile.separable(&profile, back, upper, 1.0, GimpOperator::ClipToBackdrop, mode).unwrap();
            for c in 0..3 {
                assert!((out[c] - expected[c]).abs() < 1e-15);
            }
            assert_eq!(out[3], 1.0);
            assert!(profile.separable(&profile, back, upper, 1.0, GimpOperator::Union, mode).is_err());
            assert!(gray.separable(&profile, back, upper, 1.0, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(profile.separable(&profile, back, [1.01, 0.0, 0.0, 0.0], 0.0, GimpOperator::ClipToBackdrop, mode).is_err());
        }
        let marker = 0.5 + 2.0_f64.powi(-40);
        let out = profile.separable(&profile, [marker; 4], [1.0; 4], 1.0, GimpOperator::ClipToBackdrop, GimpBlendMode::LegacyDivide).unwrap();
        assert_eq!(out, [marker; 4]);
        let threshold = f64::from(128.0_f32 / 255.0_f32);
        assert_eq!(profile.separable(&profile, [1.0; 4], [threshold; 4], 1.0, GimpOperator::ClipToBackdrop, GimpBlendMode::LegacyHardLight).unwrap(), [1.0; 4]);
    }
    #[test]
    #[ignore = "24MP release legacy ratio and HardLight throughput measurement"]
    fn legacy_contrast_composition_24mp_release() {
        let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        for mode in [
            crate::gimp_blend::GimpBlendMode::LegacyDivide,
            crate::gimp_blend::GimpBlendMode::LegacyDodge,
            crate::gimp_blend::GimpBlendMode::LegacyBurn,
            crate::gimp_blend::GimpBlendMode::LegacyHardLight,
        ] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(profile.separable(&profile, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    #[ignore = "24MP release modern Grain composition throughput measurement"]
    fn modern_grain_composition_24mp_release() {
        let linear = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let perceptual = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).unwrap();
        for mode in [crate::gimp_blend::GimpBlendMode::GrainExtract, crate::gimp_blend::GimpBlendMode::GrainMerge] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(linear.separable(&perceptual, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
    #[test]
    fn exact_transfer_identity_preserves_bits_and_rejects_curve_approximations() {
        use photocraft_cms::curve::Curve;
        let source = photocraft_cms::Builtin::Srgb.profile();
        let exact = WorkingSpace::new(source, SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).unwrap();
        let marker = [-0.0, 0.5 + 2.0_f64.powi(-40), 3.25];
        let identity = exact.convert(marker, true).unwrap();
        for c in 0..3 {
            assert_eq!(identity[c].to_bits(), marker[c].to_bits());
        }
        assert!(exact.transfer.is_none());
        assert_eq!(exact.kind, GimpSpace::PerceptualRgb);
        let mut unequal = source.clone();
        unequal.trc.as_mut().unwrap()[1] = Curve::Gamma(2.3);
        let changed = WorkingSpace::new(&unequal, SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).unwrap();
        assert!(changed.transfer.is_some());
        assert!((changed.convert([0.5; 3], true).unwrap()[1] - 0.5).abs() > 1e-3);
        let parsed = photocraft_cms::Profile::parse(&source.to_bytes()).unwrap();
        let quantized = WorkingSpace::new(&parsed, SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).unwrap();
        assert!(quantized.transfer.is_some());
        let mut table = source.clone();
        table.trc = Some([Curve::Table64(vec![0.0, 0.25, 1.0]), Curve::Gamma(1.6), Curve::Gamma(2.8)]);
        assert!(WorkingSpace::new(&table, SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).unwrap().transfer.is_some());
    }
}
