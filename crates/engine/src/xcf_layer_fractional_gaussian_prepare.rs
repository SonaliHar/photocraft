//! Explicit fractional layer mathematics over a proved raw reader.
//! Finite-unit coverage is a caller-requested mathematical interpretation, not
//! GIMP selection authority. Zero coverage keeps source words; it never aliases
//! binary empty-selection/unrestricted filtering. Derived <=2/255 requires
//! independent evidence for the caller case; author/fresh U8 exact HOLD persists.
//! Caller-established saved-state native source/mask descriptors remain separate
//! from document ICC/profile materialization and original-plane correspondence.
//! Standard sRGB math is a matched-display approximation (<=2/255), never a unique
//! GIMP float schedule, source grade, selection grant, or destructive Bake token.
//! Borrowed observation/container backing beyond the bounded str views remains
//! the enclosing caller's charge; this adapter does not acquire that owner.
use crate::{EngineError, Result, Session, xcf_layer_stored_prepare::PreparedStoredLayer};
use photocraft_compose::gimp_layer_fractional_gaussian_candidate as kernel;
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
use photocraft_raster::{Interrupt, Surface};

/// Explicit private numerical equation policy, not stored-mask semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageInterpretation {
    /// Native coverage must be finite within [0,1] before optional Float32
    /// rounding. Zero preserves raw source; >1 words are never normalized.
    FiniteUnitZeroKeepsSource,
}
/// Required caller observation, not an authority derived from the raw reader or
/// parent Gray ICC. Saved/reopened U8 mask Y' u8 is distinct from author/live Y u8.
pub struct WorkingDomainEvidence<'a> {
    pub source_sha256: &'a str,
    pub source_domain: kernel::SourceDomain,
    pub mask_domain: kernel::MaskDomain,
    pub coverage: CoverageInterpretation,
    pub coverage_precision: kernel::CoveragePrecision,
    pub mix_space: kernel::MixSpace,
    pub mix_precision: photocraft_compose::gimp_channel_gaussian_candidate::DerivedPrecision,
    pub source_native_format: &'a str,
    pub mask_native_format: &'a str,
    pub source_native_space: &'a str,
    pub mask_native_space: &'a str,
    pub source_space_icc_sha256: &'a str,
    pub mask_space_icc_sha256: &'a str,
}
// Actual 1486-byte native Babl space profile recorded in all independent saved
// source/selection captures; distinct from the 544-byte effective Gray profile.
// This checks the caller's declared observation; it does not hash/load an ICC or
// prove these descriptors from an XCF parent profile/operation name.
const OBSERVED_NATIVE_SPACE_SHA: &str = "05c73c41561baabc9be5f872dfbaaf169412e35f7cd4fa3248f561a93abc009f";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "xcf.layer.fractionalGaussian.prepare".into(), msg: message.into() }
}
fn strings<'a>(e: &WorkingDomainEvidence<'a>) -> [&'a str; 7] {
    [
        e.source_sha256,
        e.source_native_format,
        e.mask_native_format,
        e.source_native_space,
        e.mask_native_space,
        e.source_space_icc_sha256,
        e.mask_space_icc_sha256,
    ]
}
fn shape(e: &WorkingDomainEvidence<'_>) -> Result<u64> {
    let mut bytes = 0u64;
    for text in strings(e) {
        if text.is_empty() || text.len() > 64 || text.contains('\0') {
            return Err(bad("bounded native working observation required"));
        }
        bytes = bytes.checked_add(text.len() as u64).ok_or_else(|| bad("observation work overflow"))?;
    }
    for hash in [e.source_sha256, e.source_space_icc_sha256, e.mask_space_icc_sha256] {
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(bad("native observation hash shape"));
        }
    }
    Ok(bytes)
}
fn descriptors(e: &WorkingDomainEvidence<'_>, config: kernel::Config) -> Result<()> {
    let (source, mask) = match (config.source_domain, config.mask_domain) {
        (kernel::SourceDomain::SrgbNonlinearU8, kernel::MaskDomain::NativeNonlinearU8) => ("Y' u8", "Y' u8"),
        (kernel::SourceDomain::SrgbNonlinearF64, kernel::MaskDomain::NativeLinearF64) => ("Y' double", "Y double"),
        _ => return Err(bad("paired observed source and raw mask domain")),
    };
    if e.source_domain != config.source_domain
        || e.mask_domain != config.mask_domain
        || e.coverage != CoverageInterpretation::FiniteUnitZeroKeepsSource
        || e.coverage_precision != config.coverage_precision
        || e.mix_space != config.mix_space
        || e.mix_precision != config.mix_precision
        || e.source_native_format != source
        || e.mask_native_format != mask
        || e.source_native_space != "sRGB"
        || e.mask_native_space != "sRGB"
        || !e.source_space_icc_sha256.eq_ignore_ascii_case(OBSERVED_NATIVE_SPACE_SHA)
        || !e.mask_space_icc_sha256.eq_ignore_ascii_case(OBSERVED_NATIVE_SPACE_SHA)
    {
        return Err(bad("independent saved-state native working observation differs from request"));
    }
    Ok(())
}
/// Exactly one tuple field: the raw reader's transaction prices this actual
/// inline layout, including its generic lease. No extra ownership is hidden.
pub struct PreparedLayerFractionalGaussian<'o, 's, 'b, 'a, L> {
    parts: (PreparedStoredLayer<'o, 's, 'b, 'a, L>, kernel::StagedOutput<()>),
}
impl<'o, 's, 'b, 'a, L> PreparedLayerFractionalGaussian<'o, 's, 'b, 'a, L> {
    pub fn raw(&self) -> &PreparedStoredLayer<'o, 's, 'b, 'a, L> {
        &self.parts.0
    }
    pub fn derived_surface(&self) -> &Surface {
        self.parts.1.surface()
    }
    pub fn unmasked_surface(&self) -> Option<&Surface> {
        self.parts.1.unmasked_surface()
    }
    pub fn config(&self) -> kernel::Config {
        self.parts.1.config()
    }
    pub fn stage_requirements(&self) -> kernel::Requirements {
        self.parts.1.requirements()
    }
    pub fn validate(&self, session: &Session, control: &Interrupt<'_>) -> Result<()> {
        self.parts.0.validate(session, control)
    }
}
/// Consumes the raw reader. Original and mask owners remain alive with the new
/// private stage under the already held lease and remaining Work. Failure drops
/// private stages and never changes the Session or refunds declared work.
pub fn prepare<'o, 's, 'b, 'a, L>(
    raw: PreparedStoredLayer<'o, 's, 'b, 'a, L>,
    session: &Session,
    evidence: WorkingDomainEvidence<'_>,
    config: kernel::Config,
    control: &Interrupt<'_>,
) -> Result<PreparedLayerFractionalGaussian<'o, 's, 'b, 'a, L>> {
    raw.validate(session, control)?;
    let bytes = shape(&evidence)?;
    let node = raw.original_node()?;
    if node.arguments.len() != 5 {
        return Err(bad("original scalar argument envelope"));
    }
    let name_bytes = node.arguments.iter().try_fold(0u64, |sum, arg| {
        if arg.name.len() > 256 {
            return Err(bad("original scalar name bound"));
        }
        sum.checked_add(arg.name.len() as u64).ok_or_else(|| bad("scalar work overflow"))
    })?;
    let work = bytes
        .checked_add(name_bytes)
        .and_then(|bytes| bytes.checked_mul(4))
        .and_then(|bytes| bytes.checked_add(64))
        .ok_or_else(|| bad("working observation work overflow"))?;
    raw.charge_metadata_work(work, control)?;
    descriptors(&evidence, config)?;
    if !evidence.source_sha256.eq_ignore_ascii_case(raw.source_sha256()) {
        return Err(bad("working observation belongs to a different original source"));
    }
    // Exact typed Float32 source words bind requested sigmas. Other raw enums,
    // opaque outer triplets and mask flags are never translated to Auto/Clamp.
    for (name, value) in [("std-dev-x", config.gaussian.std_dev_x), ("std-dev-y", config.gaussian.std_dev_y)] {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let arg = node.arguments.iter().find(|arg| arg.name == name).ok_or_else(|| bad("original Gaussian scalar missing"))?;
        if !matches!(arg.value,FilterValue::Number(ExactNumber::Float32Bits(bits)) if f64::from(f32::from_bits(bits))==value) {
            return Err(bad("Gaussian request differs from original scalar words"));
        }
    }
    let parts = raw.into_fractional_gaussian_stage(session, config, control)?;
    Ok(PreparedLayerFractionalGaussian { parts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianParams};
    use photocraft_compose::gimp_channel_gaussian_candidate::DerivedPrecision;
    fn config() -> kernel::Config {
        kernel::Config {
            source_domain: kernel::SourceDomain::SrgbNonlinearF64,
            mask_domain: kernel::MaskDomain::NativeLinearF64,
            gaussian_precision: DerivedPrecision::F64,
            coverage_precision: kernel::CoveragePrecision::NativeF64,
            mix_space: kernel::MixSpace::RawEncodedGray,
            mix_precision: DerivedPrecision::F64,
            gaussian: GaussianParams { std_dev_x: 1., std_dev_y: 1., abyss: GaussianAbyss::Clamp, clip_extent: true },
            filter: GaussianFilter::Auto,
            output: photocraft_geom::Rect::new(0, 0, 32, 32),
        }
    }
    fn evidence() -> WorkingDomainEvidence<'static> {
        WorkingDomainEvidence {
            source_sha256: OBSERVED_NATIVE_SPACE_SHA,
            source_domain: config().source_domain,
            mask_domain: config().mask_domain,
            coverage: CoverageInterpretation::FiniteUnitZeroKeepsSource,
            coverage_precision: config().coverage_precision,
            mix_space: config().mix_space,
            mix_precision: config().mix_precision,
            source_native_format: "Y' double",
            mask_native_format: "Y double",
            source_native_space: "sRGB",
            mask_native_space: "sRGB",
            source_space_icc_sha256: OBSERVED_NATIVE_SPACE_SHA,
            mask_space_icc_sha256: OBSERVED_NATIVE_SPACE_SHA,
        }
    }
    #[test]
    fn fractional_precision_and_mix_are_explicit_not_binary_or_parent_profile_aliases() {
        let mut e = evidence();
        assert!(shape(&e).is_ok());
        assert!(descriptors(&e, config()).is_ok());
        e.coverage_precision = kernel::CoveragePrecision::RoundedF32;
        assert!(descriptors(&e, config()).is_err());
        e = evidence();
        e.mix_space = kernel::MixSpace::SrgbDecodedLinear;
        assert!(descriptors(&e, config()).is_err());
        e = evidence();
        e.mix_precision = DerivedPrecision::FinalF32;
        assert!(descriptors(&e, config()).is_err());
        e = evidence();
        e.mask_native_format = "Y' double";
        assert!(descriptors(&e, config()).is_err());
    }
    #[test]
    fn fractional_evidence_does_not_relax_native_format_or_hash_bounds() {
        let mut e = evidence();
        e.source_native_format = "Y double";
        assert!(descriptors(&e, config()).is_err());
        e = evidence();
        e.source_sha256 = "0";
        assert!(shape(&e).is_err());
        let text = "x".repeat(65);
        let e = WorkingDomainEvidence { source_native_format: &text, ..evidence() };
        assert!(shape(&e).is_err());
    }
}
