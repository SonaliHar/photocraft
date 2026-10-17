//! Owned CMS preparation for an explicitly bound retained configuration.
//! Resolved effective profiles/transfer are caller inputs, never inferred ICC
//! defaults. No default dispatcher, composition envelope or source admission.
use super::*;
use crate::gimp_configured_candidate::{BoundSource, bind_source_node_with_sample_policy};
use photocraft_cms::{Intent, Profile, control};

const CONFIG_BYTES: u64 = 65536;
const CONFIG_WORK: u64 = 16384;
const CENSUS_WORK: u64 = 2 * 32 * (3 * 4096 + 256);
// Four possible stored curve slots: finite scan plus the existing <=4096
// sample and 257×128 semantic validation schedule, and fixed matrix checks.
const PROFILE_VALIDATION_WORK: u64 = 4 * (8192 + 32896) + 1024;
fn cms_error(error: control::Error) -> RasterInputError {
    match error {
        control::Error::Cancelled => RasterInputError::Cancelled,
        control::Error::Allocation => RasterInputError::AllocationFailed,
        control::Error::Limit(_) => RasterInputError::LimitExceeded,
        control::Error::Cms(_) => RasterInputError::UnsupportedConversion,
    }
}
fn lowered(limits: control::Limits, bytes: usize, work: u64) -> Result<control::Limits, RasterInputError> {
    Ok(control::Limits {
        resident_bytes: limits.resident_bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?,
        work_units: limits.work_units.checked_sub(work).ok_or(RasterInputError::LimitExceeded)?,
    })
}
fn profile_domain(profile: &Profile, limits: control::Limits, control: &Interrupt<'_>) -> Result<u64, RasterInputError> {
    // Preserve the existing candidate's non-CLUT/<=4096-table envelope; numeric
    // policy comes from CMS's controlled XCF validator, not writer quantization.
    poll(control)?;
    if !matches!(profile.channels(), 1 | 3) || profile.a2b.iter().chain(&profile.b2a).any(Option::is_some) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    for curve in profile.trc.iter().flatten().chain(profile.gray_trc.iter()) {
        poll(control)?;
        let size = match curve {
            Curve::Table(v) => v.len(),
            Curve::Table64(v) => v.len(),
            _ => 0,
        };
        if size > 4096 {
            return Err(RasterInputError::UnsupportedConversion);
        }
    }
    if PROFILE_VALIDATION_WORK > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    // Even currently inactive public model fields must not be encoded through a
    // writer's saturating conversion. This is finite admission, not tone math.
    for curve in profile.trc.iter().flatten().chain(profile.gray_trc.iter()) {
        poll(control)?;
        match curve {
            Curve::Identity => {}
            Curve::Gamma(v) => {
                if !v.is_finite() {
                    return Err(RasterInputError::UnsupportedConversion);
                }
            }
            Curve::Parametric { p, .. } => {
                for v in p {
                    poll(control)?;
                    if !v.is_finite() {
                        return Err(RasterInputError::UnsupportedConversion);
                    }
                }
            }
            Curve::Table(v) => {
                for part in v.chunks(256) {
                    poll(control)?;
                    if part.iter().any(|v| !v.is_finite()) {
                        return Err(RasterInputError::UnsupportedConversion);
                    }
                }
            }
            Curve::Table64(v) => {
                for part in v.chunks(256) {
                    poll(control)?;
                    if part.iter().any(|v| !v.is_finite()) {
                        return Err(RasterInputError::UnsupportedConversion);
                    }
                }
            }
        }
    }
    for matrix in profile.matrix.iter().chain(profile.chad.iter()) {
        for row in matrix {
            poll(control)?;
            if row.iter().any(|v| !v.is_finite()) {
                return Err(RasterInputError::UnsupportedConversion);
            }
        }
    }
    photocraft_cms::xcf_color::validate_profile_with_control(
        profile,
        control::Limits { resident_bytes: limits.resident_bytes, work_units: PROFILE_VALIDATION_WORK },
        &|| control.cancelled(),
    )
    .map_err(cms_error)?;
    poll(control)?;
    Ok(PROFILE_VALIDATION_WORK)
}
fn digest(bytes: &[u8], bound: usize, control: &Interrupt<'_>) -> Result<[u8; 32], RasterInputError> {
    use sha2::{Digest, Sha256};
    if bytes.len() > bound {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut hash = Sha256::new();
    for chunk in bytes.chunks(4096) {
        poll(control)?;
        hash.update(chunk);
    }
    poll(control)?;
    Ok(hash.finalize().into())
}
/// Owns the pair's complete acceleration/Post storage, while the exact original
/// node remains immutably borrowed. It cannot manufacture a source qualification.
/// PreparedConfiguration remains closure-local because it borrows decoded fixed
/// configuration scratch; this avoids a self-referential owned structure.
pub struct OwnedConfiguredColorCandidate<'a> {
    bound: BoundSource<'a>,
    forward: Transform,
    inverse: Transform,
    source: WorkingContext,
    working: WorkingContext,
    retained_bytes: u64,
    work_per_pixel: u64,
    preparation_work: u64,
    ceiling: Limits,
}
impl OwnedConfiguredColorCandidate<'_> {
    pub fn original_node(&self) -> &FilterNode {
        self.bound.original_node()
    }
    pub fn preparation_work(&self) -> u64 {
        self.preparation_work
    }
    pub fn source_context(&self) -> WorkingContext {
        self.source
    }
    pub fn working_context(&self) -> WorkingContext {
        self.working
    }
    pub(crate) fn with_prepared<T>(
        &self,
        control: &Interrupt<'_>,
        run: impl FnOnce(&photocraft_algo::gimp::configured::PreparedConfiguration<'_>) -> Result<T, RasterInputError>,
    ) -> Result<T, RasterInputError> {
        self.bound.with_prepared(self.working, Limits { resident_bytes: CONFIG_BYTES, work_units: CONFIG_WORK }, control, run)
    }
    pub fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }
    pub(crate) fn pair(&self) -> TransformPair<'_> {
        TransformPair {
            source: self.source,
            working: self.working,
            forward: &self.forward,
            inverse: &self.inverse,
            retained_bytes: self.retained_bytes,
            work_per_pixel: self.work_per_pixel,
        }
    }
    /// Configuration is prepared before the first ROI/source copy. Total work
    /// retains constructor/hash/preparation charges, with no upstream refunds.
    // Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        source: &Surface,
        extent: Rect,
        roi: Rect,
        actual_source: WorkingContext,
        working_default: &[u8],
        policy: OutputPolicy,
        limits: Limits,
        control: &Interrupt<'_>,
    ) -> Result<RenderedCandidate, RasterInputError> {
        poll(control)?;
        if actual_source != self.source {
            return Err(RasterInputError::UnsupportedConversion);
        }
        if limits.resident_bytes > self.ceiling.resident_bytes || limits.work_units > self.ceiling.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        let scan_work = requirements_preparation_work(source, &self.pair())?;
        let execution = Limits {
            resident_bytes: limits.resident_bytes,
            work_units: limits
                .work_units
                .checked_sub(self.preparation_work)
                .and_then(|n| n.checked_sub(CONFIG_WORK))
                .and_then(|n| n.checked_sub(scan_work))
                .ok_or(RasterInputError::LimitExceeded)?,
        };
        let config_limits = Limits {
            resident_bytes: execution.resident_bytes.checked_sub(self.retained_bytes).ok_or(RasterInputError::LimitExceeded)?.min(CONFIG_BYTES),
            work_units: CONFIG_WORK,
        };
        self.bound.with_prepared(self.working, config_limits, control, |prepared| {
            render_prepared_color_candidate(prepared, source, extent, roi, actual_source, &self.pair(), working_default, policy, execution, control)
        })
    }
}
/// Source/working profiles must already represent the caller's explicit transfer
/// contexts. Exact encoded-byte SHA256s and channel/alpha identities are checked;
/// this does not establish their provenance or reinterpret a retained source ICC.
/// Constructors preserve RelativeColorimetric/no-BPC and existing native ROI math.
/// Both profile/encoding and constructor envelopes are admitted before copying.
#[allow(clippy::too_many_arguments)]
pub fn prepare_owned_color_candidate<'a>(
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    sampling: CurveSamplePolicy,
    actual_source: WorkingContext,
    source_profile: &Profile,
    working_profile: &Profile,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<OwnedConfiguredColorCandidate<'a>, RasterInputError> {
    prepare_owned_with_envelope(node, binding, sampling, actual_source, source_profile, working_profile, limits, control, None)
}
#[allow(clippy::too_many_arguments)]
fn prepare_owned_with_envelope<'a>(
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    sampling: CurveSamplePolicy,
    actual_source: WorkingContext,
    source_profile: &Profile,
    working_profile: &Profile,
    limits: Limits,
    control: &Interrupt<'_>,
    upper: Option<&photocraft_cms::transform::EffectivePairRequirements>,
) -> Result<OwnedConfiguredColorCandidate<'a>, RasterInputError> {
    poll(control)?;
    if limits.resident_bytes == 0
        || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
        || !matches!(actual_source.color_channels, 1 | 3)
        || actual_source.color_channels != source_profile.channels()
        || binding.working.color_channels != working_profile.channels()
        || actual_source.alpha != binding.working.alpha
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let bound = bind_source_node_with_sample_policy(node, binding, sampling, control)?;
    // Existing typed domain is five channels, <=64 points and canonical256 stored
    // tables. Reserve its complete fixed local preparation envelope before tables.
    let resident = limits.resident_bytes.checked_sub(CONFIG_BYTES).ok_or(RasterInputError::LimitExceeded)?;
    let work_limit = limits.work_units.checked_sub(CONFIG_WORK).and_then(|n| n.checked_sub(CENSUS_WORK)).ok_or(RasterInputError::LimitExceeded)?;
    let maximum = control::Limits::default();
    let cms_limits = control::Limits {
        resident_bytes: usize::try_from(resident.min(maximum.resident_bytes as u64)).map_err(|_| RasterInputError::LimitExceeded)?,
        work_units: work_limit.min(maximum.work_units),
    };
    let cancelled = || control.cancelled();
    let validation_work = mul(PROFILE_VALIDATION_WORK, 2)?;
    if validation_work > cms_limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    profile_domain(source_profile, cms_limits, control)?;
    profile_domain(working_profile, cms_limits, control)?;
    let cms_limits = lowered(cms_limits, 0, validation_work)?;
    let se = source_profile.encode_requirements_with_control(cms_limits, &cancelled).map_err(cms_error)?;
    let we = working_profile.encode_requirements_with_control(cms_limits, &cancelled).map_err(cms_error)?;
    let encoding_peak = se.peak_resident_bytes.checked_add(we.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?;
    let source_hash_bound = se.encoded_bytes_upper_bound.max(se.retained_bytes);
    let working_hash_bound = we.encoded_bytes_upper_bound.max(we.retained_bytes);
    let encoding_work = add(mul(add(se.work_units, we.work_units)?, 3)?, add(source_hash_bound as u64, working_hash_bound as u64)?)?;
    let pair_limits = lowered(cms_limits, encoding_peak, encoding_work)?;
    let options = photocraft_cms::transform::TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, ..Default::default() };
    let f = Transform::requirements_with_control(source_profile, working_profile, options, pair_limits, &cancelled).map_err(cms_error)?;
    let r = Transform::requirements_with_control(working_profile, source_profile, options, pair_limits, &cancelled).map_err(cms_error)?;
    if let Some(upper) = upper {
        for (actual, bound) in [(se, upper.source_encoding), (we, upper.working_encoding)] {
            if actual.retained_bytes > bound.retained_bytes
                || actual.peak_resident_bytes > bound.peak_resident_bytes
                || actual.work_units > bound.work_units
                || actual.encoded_bytes_upper_bound > bound.encoded_bytes_upper_bound
            {
                return Err(RasterInputError::LimitExceeded);
            }
        }
        for (actual, bound) in [(f, upper.forward), (r, upper.inverse)] {
            if actual.retained_bytes > bound.retained_bytes || actual.peak_resident_bytes > bound.peak_resident_bytes || actual.work_units > bound.work_units {
                return Err(RasterInputError::LimitExceeded);
            }
        }
    }
    let pair_peak = f.peak_resident_bytes.checked_add(r.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?;
    let pair_work = mul(add(f.work_units, r.work_units)?, 3)?;
    if pair_peak > pair_limits.resident_bytes || pair_work > pair_limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    // No raster/source clone occurs during either schema validation or preparation.
    bound.with_prepared(binding.working, Limits { resident_bytes: CONFIG_BYTES, work_units: CONFIG_WORK }, control, |_| Ok(()))?;
    let source_bytes = source_profile.to_bytes_with_control(lowered(cms_limits, we.peak_resident_bytes, we.work_units)?, &cancelled).map_err(cms_error)?;
    let working_bytes = working_profile.to_bytes_with_control(lowered(cms_limits, se.peak_resident_bytes, se.work_units)?, &cancelled).map_err(cms_error)?;
    if digest(&source_bytes, source_hash_bound, control)? != actual_source.profile_identity
        || digest(&working_bytes, working_hash_bound, control)? != binding.working.profile_identity
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let forward_limits = match upper {
        Some(bound) => phase_limits(bound.forward.peak_resident_bytes, mul(bound.forward.work_units, 3)?)?,
        None => lowered(pair_limits, r.peak_resident_bytes, mul(r.work_units, 3)?)?,
    };
    let inverse_limits = match upper {
        Some(bound) => phase_limits(bound.inverse.peak_resident_bytes, mul(bound.inverse.work_units, 3)?)?,
        None => lowered(pair_limits, f.peak_resident_bytes, mul(f.work_units, 3)?)?,
    };
    let forward = Transform::with_options_with_control(source_profile, working_profile, options, forward_limits, &cancelled).map_err(cms_error)?;
    let inverse = Transform::with_options_with_control(working_profile, source_profile, options, inverse_limits, &cancelled).map_err(cms_error)?;
    let retained = add(
        forward.retained_bytes_with_control(pair_limits, &cancelled).map_err(cms_error)? as u64,
        inverse.retained_bytes_with_control(pair_limits, &cancelled).map_err(cms_error)? as u64,
    )?;
    let (_, fw) = transform_requirements(&forward, control)?;
    let (_, rw) = transform_requirements(&inverse, control)?;
    // The actual pipeline scan is prepaid from the existing bounded 32-stage,
    // three-channel <=4096-table candidate domain, once for each pair member.
    let preparation_work = add(add(add(add(CONFIG_WORK, encoding_work)?, pair_work)?, CENSUS_WORK)?, validation_work)?;
    if preparation_work > limits.work_units || retained.checked_add(CONFIG_BYTES).is_none_or(|n| n > limits.resident_bytes) {
        return Err(RasterInputError::LimitExceeded);
    }
    poll(control)?;
    Ok(OwnedConfiguredColorCandidate {
        bound,
        forward,
        inverse,
        source: actual_source,
        working: binding.working,
        retained_bytes: retained,
        work_per_pixel: add(fw, rw)?,
        preparation_work,
        ceiling: limits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gimp_configured_candidate::{Kind, LEVELS_SCHEMA};
    use photocraft_doc::{
        item_identity::FilterId,
        xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
    };
    fn fixture() -> FilterNode {
        let channels = ["value", "red", "green", "blue", "alpha"].map(|name| FilterArgument {
            name: name.into(),
            value: FilterValue::Config(
                [("low-input", 0.0f64), ("high-input", 1.0), ("gamma", 1.0), ("low-output", 0.0), ("high-output", 1.0)]
                    .map(|(name, v)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(v.to_bits())) })
                    .into(),
            ),
        });
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:levels".into(), stored_version: "0:0".into(), schema_epoch: "owned-test".into() },
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
            arguments: vec![
                FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:levels::trc".into(), value: 1 } },
                FilterArgument { name: "clamp-input".into(), value: FilterValue::Bool(false) },
                FilterArgument { name: "clamp-output".into(), value: FilterValue::Bool(false) },
                FilterArgument { name: "config".into(), value: FilterValue::Config(channels.into()) },
            ],
        }
    }
    fn context(profile: &Profile) -> WorkingContext {
        let bytes = profile.to_bytes();
        WorkingContext {
            transfer: photocraft_algo::gimp::configured::Transfer::NonlinearProfile,
            profile_identity: digest(&bytes, bytes.len(), &Interrupt::NONE).unwrap(),
            color_channels: 3,
            alpha: true,
        }
    }
    fn binding(context: WorkingContext) -> SourceBinding<'static> {
        SourceBinding { xcf_version: 22, expected_epoch: "owned-test", schema_sha256: LEVELS_SCHEMA, kind: Kind::Levels, working: context }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 256 << 20, work_units: 1_000_000_000 }
    }
    #[test]
    fn owned_pair_preserves_exact_original_constructor_and_borrowed_config() {
        let node = fixture();
        let before = node.clone();
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let context = context(profile);
        let prepared =
            prepare_owned_color_candidate(&node, binding(context), CurveSamplePolicy::RequireExact256, context, profile, profile, limits(), &Interrupt::NONE)
                .unwrap();
        let old = Transform::new(profile, profile, Intent::RelativeColorimetric, false).unwrap();
        assert_eq!(prepared.forward.pipeline(), old.pipeline());
        assert_eq!(prepared.inverse.pipeline(), old.pipeline());
        assert_eq!(prepared.original_node(), &before);
        prepared
            .bound
            .with_prepared(context, Limits { resident_bytes: CONFIG_BYTES, work_units: CONFIG_WORK }, &Interrupt::NONE, |config| {
                let input = [0.125, 0.5, 0.875, 0.25];
                let mut output = [0.0; 4];
                config.try_eval_pixel(&input, &mut output, context, &Interrupt::NONE)?;
                assert_eq!(input.map(f64::to_bits), output.map(f64::to_bits));
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn retained_constructor_uses_five_stored_selectors_and_rejects_wrong_schema() {
        let mut node = fixture();
        let FilterValue::Config(channels) = &mut node.arguments[3].value else { panic!("fixture config") };
        let FilterValue::Config(alpha) = &mut channels[4].value else { panic!("alpha config") };
        alpha[3].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        let before = node.clone();
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let context = context(profile);
        let identity = crate::gimp_configured_candidate::RetainedSourceIdentity {
            xcf_version: 22,
            expected_epoch: "owned-test",
            schema_sha256: LEVELS_SCHEMA,
            profile_identity: context.profile_identity,
            color_channels: 3,
            alpha: true,
        };
        let candidate = prepare_retained_color_candidate(&node, identity, context, profile, profile, limits(), &Interrupt::NONE).unwrap();
        candidate
            .bound
            .with_prepared(context, Limits { resident_bytes: CONFIG_BYTES, work_units: CONFIG_WORK }, &Interrupt::NONE, |prepared| {
                let mut actual = [0.0; 4];
                prepared.try_eval_pixel(&[0.2, 0.4, 0.6, 0.0], &mut actual, context, &Interrupt::NONE)?;
                assert_eq!(actual[3].to_bits(), 0.25f64.to_bits());
                assert_eq!(actual[..3], [0.2, 0.4, 0.6]);
                Ok(())
            })
            .unwrap();
        assert_eq!(candidate.original_node(), &before);
        assert!(candidate.preparation_work() >= 2 * CONFIG_WORK);
        let wrong = crate::gimp_configured_candidate::RetainedSourceIdentity { schema_sha256: "unbound-schema", ..identity };
        assert!(prepare_retained_color_candidate(&node, wrong, context, profile, profile, limits(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn one_explicit_profile_basis_derives_only_the_retained_transfer() {
        use photocraft_algo::gimp::configured::Transfer;
        use photocraft_cms::xcf_color::{self, Encoding};
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let before = basis.to_bytes();
        let actual = context(basis);
        for (tag, encoding, transfer) in [
            (0, Encoding::Linear, Transfer::Linear),
            (1, Encoding::NonlinearProfile, Transfer::NonlinearProfile),
            (2, Encoding::Perceptual, Transfer::Perceptual),
        ] {
            let mut node = fixture();
            node.arguments[0].value = FilterValue::Enum { domain: "gimp:levels::trc".into(), value: tag };
            let schema =
                crate::gimp_configured_candidate::RetainedSchemaIdentity { xcf_version: 22, expected_epoch: "owned-test", schema_sha256: LEVELS_SCHEMA };
            let candidate =
                prepare_profile_owned_color_candidate(&node, schema, basis, Encoding::NonlinearProfile, actual, limits(), &Interrupt::NONE).unwrap();
            let old = xcf_color::effective_profile_from_definition(basis, encoding).unwrap();
            let old_bytes = old.to_bytes();
            assert_eq!(candidate.source_context(), actual);
            assert_eq!(candidate.working_context().transfer, transfer);
            assert_eq!(candidate.working_context().profile_identity, digest(&old_bytes, old_bytes.len(), &Interrupt::NONE).unwrap());
            let old_forward = Transform::new(basis, &old, Intent::RelativeColorimetric, false).unwrap();
            assert_eq!(candidate.forward.pipeline(), old_forward.pipeline());
            assert_eq!(basis.to_bytes().as_slice(), before.as_slice());
        }
    }
    #[test]
    fn profile_owned_source_identity_encoding_and_invalid_retained_fields_do_not_fallback() {
        use photocraft_cms::xcf_color::Encoding;
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let node = fixture();
        let actual = context(basis);
        let schema = crate::gimp_configured_candidate::RetainedSchemaIdentity { xcf_version: 22, expected_epoch: "owned-test", schema_sha256: LEVELS_SCHEMA };
        assert!(matches!(
            prepare_profile_owned_color_candidate(&node, schema, basis, Encoding::Linear, actual, limits(), &Interrupt::NONE),
            Err(RasterInputError::UnsupportedConversion)
        ));
        let mut wrong = actual;
        wrong.profile_identity[0] ^= 1;
        assert!(prepare_profile_owned_color_candidate(&node, schema, basis, Encoding::NonlinearProfile, wrong, limits(), &Interrupt::NONE).is_err());
        let mut malformed = node.clone();
        let FilterValue::Config(groups) = &mut malformed.arguments[3].value else { panic!("config") };
        let FilterValue::Config(fields) = &mut groups[4].value else { panic!("alpha") };
        fields[2].value = FilterValue::Number(ExactNumber::Float64Bits(f64::NAN.to_bits()));
        assert!(crate::gimp_configured_candidate::retained_source_transfer(&malformed, schema, &Interrupt::NONE).is_err());
        let stop = || true;
        assert!(matches!(
            prepare_profile_owned_color_candidate(&node, schema, basis, Encoding::NonlinearProfile, actual, limits(), &Interrupt::cancel_only(&stop)),
            Err(RasterInputError::Cancelled)
        ));
    }
    #[test]
    fn malformed_original_profile_values_refuse_before_encoding_or_pair_copy() {
        let node = fixture();
        let valid = photocraft_cms::Builtin::Srgb.profile();
        let actual = context(valid);
        let before = valid.to_bytes();
        let mut profiles = Vec::new();
        for curve in [
            Curve::Gamma(f64::NAN),
            Curve::Gamma(f64::INFINITY),
            Curve::Parametric { kind: 0, p: [f64::NAN; 7] },
            Curve::Table(vec![0.0, f32::NAN, 1.0]),
            Curve::Table64(vec![0.0, f64::INFINITY, 1.0]),
        ] {
            let mut profile = valid.clone();
            profile.trc = Some([curve, Curve::Identity, Curve::Identity]);
            profiles.push(profile);
        }
        let mut profile = valid.clone();
        profile.matrix.as_mut().unwrap()[0][0] = f64::INFINITY;
        profiles.push(profile);
        let mut profile = valid.clone();
        profile.matrix = Some([[0.0; 3]; 3]);
        profiles.push(profile);
        for profile in &profiles {
            assert!(matches!(
                prepare_owned_color_candidate(&node, binding(actual), CurveSamplePolicy::RequireExact256, actual, profile, valid, limits(), &Interrupt::NONE),
                Err(RasterInputError::UnsupportedConversion)
            ));
            assert_eq!(profile.to_bytes().as_slice(), before.as_slice());
        }
        assert_eq!(valid.to_bytes().as_slice(), before.as_slice());
    }
    #[test]
    fn profile_numeric_validation_obeys_work_and_resident_quotas_and_cancel() {
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let before = profile.to_bytes();
        assert!(matches!(
            profile_domain(profile, control::Limits { resident_bytes: 256 << 20, work_units: PROFILE_VALIDATION_WORK - 1 }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        ));
        assert!(matches!(
            profile_domain(profile, control::Limits { resident_bytes: 1, work_units: PROFILE_VALIDATION_WORK }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        ));
        let stop = || true;
        assert!(matches!(profile_domain(profile, control::Limits::default(), &Interrupt::cancel_only(&stop)), Err(RasterInputError::Cancelled)));
        assert_eq!(profile.to_bytes().as_slice(), before.as_slice());
    }
    #[test]
    fn complete_node_census_precedes_effective_copies_and_bounds_real_constructor() {
        use photocraft_cms::xcf_color::Encoding;
        let mut node = fixture();
        node.arguments[0].value = FilterValue::Enum { domain: "gimp:levels::trc".into(), value: 0 };
        let original_node = node.clone();
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let bytes = basis.to_bytes();
        let actual = context(basis);
        let schema = crate::gimp_configured_candidate::RetainedSchemaIdentity { xcf_version: 22, expected_epoch: "owned-test", schema_sha256: LEVELS_SCHEMA };
        let required = profile_owned_color_requirements(&node, schema, basis, Encoding::NonlinearProfile, actual, limits(), &Interrupt::NONE).unwrap();
        let quota = Limits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units };
        let candidate = prepare_profile_owned_color_candidate(&node, schema, basis, Encoding::NonlinearProfile, actual, quota, &Interrupt::NONE).unwrap();
        assert!(candidate.retained_bytes() <= required.retained_bytes);
        assert_eq!(candidate.preparation_work(), required.work_units);
        assert_eq!(candidate.original_node(), &original_node);
        assert_eq!(basis.to_bytes().as_slice(), bytes.as_slice());
        assert!(matches!(
            prepare_profile_owned_color_candidate(
                &node,
                schema,
                basis,
                Encoding::NonlinearProfile,
                actual,
                Limits { resident_bytes: required.peak_resident_bytes - 1, ..quota },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        ));
        assert!(matches!(
            prepare_profile_owned_color_candidate(
                &node,
                schema,
                basis,
                Encoding::NonlinearProfile,
                actual,
                Limits { work_units: required.work_units - 1, ..quota },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        ));
        assert_eq!(node, original_node);
    }
    #[test]
    fn whole_node_borrowed_census_preserves_malformed_source_and_propagates_cancel() {
        use photocraft_cms::xcf_color::Encoding;
        let node = fixture();
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let actual = context(basis);
        let schema = crate::gimp_configured_candidate::RetainedSchemaIdentity { xcf_version: 22, expected_epoch: "owned-test", schema_sha256: LEVELS_SCHEMA };
        let stop = || true;
        assert!(matches!(
            profile_owned_color_requirements(&node, schema, basis, Encoding::NonlinearProfile, actual, limits(), &Interrupt::cancel_only(&stop)),
            Err(RasterInputError::Cancelled)
        ));
        let mut malformed = basis.clone();
        malformed.matrix.as_mut().unwrap()[0][0] = f64::NAN;
        let before = malformed.to_bytes();
        assert!(matches!(
            profile_owned_color_requirements(&node, schema, &malformed, Encoding::NonlinearProfile, actual, limits(), &Interrupt::NONE),
            Err(RasterInputError::UnsupportedConversion)
        ));
        assert_eq!(malformed.to_bytes().as_slice(), before.as_slice());
    }
    #[test]
    fn changed_profile_identity_schema_and_resource_limits_refuse_before_roi() {
        let node = fixture();
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let context = context(profile);
        let mut stale = context;
        stale.profile_identity[0] ^= 1;
        assert!(matches!(
            prepare_owned_color_candidate(&node, binding(context), CurveSamplePolicy::RequireExact256, stale, profile, profile, limits(), &Interrupt::NONE),
            Err(RasterInputError::UnsupportedConversion)
        ));
        let mut wrong = binding(context);
        wrong.schema_sha256 = "wrong";
        assert!(
            prepare_owned_color_candidate(&node, wrong, CurveSamplePolicy::RequireExact256, context, profile, profile, limits(), &Interrupt::NONE).is_err()
        );
        assert!(matches!(
            prepare_owned_color_candidate(
                &node,
                binding(context),
                CurveSamplePolicy::RequireExact256,
                context,
                profile,
                profile,
                Limits { resident_bytes: 1, work_units: 1 },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        ));
        let stop = || true;
        assert!(matches!(
            prepare_owned_color_candidate(
                &node,
                binding(context),
                CurveSamplePolicy::RequireExact256,
                context,
                profile,
                profile,
                limits(),
                &Interrupt::cancel_only(&stop)
            ),
            Err(RasterInputError::Cancelled)
        ));
    }
}

/// Decode the kind/TRC/channel forms from retained typed configuration, including
/// all five serialized selectors. No public GObject selected-channel proxy or
/// requested sampling metadata supplies storage authority. Profiles remain the
/// caller's explicit already-resolved effective definitions; outer composition,
/// mask/region and source admission remain outside this candidate API.
#[allow(clippy::too_many_arguments)]
pub fn prepare_retained_color_candidate<'a>(
    node: &'a FilterNode,
    identity: crate::gimp_configured_candidate::RetainedSourceIdentity<'a>,
    actual_source: WorkingContext,
    source_profile: &Profile,
    working_profile: &Profile,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<OwnedConfiguredColorCandidate<'a>, RasterInputError> {
    poll(control)?;
    if limits.resident_bytes < CONFIG_BYTES || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units < CONFIG_WORK {
        return Err(RasterInputError::LimitExceeded);
    }
    let plan = crate::gimp_configured_candidate::prepare_retained_source_plan(
        node,
        identity,
        Limits { resident_bytes: CONFIG_BYTES, work_units: CONFIG_WORK },
        control,
    )?;
    let remaining =
        Limits { resident_bytes: limits.resident_bytes, work_units: limits.work_units.checked_sub(CONFIG_WORK).ok_or(RasterInputError::LimitExceeded)? };
    let mut candidate = prepare_owned_color_candidate(
        node,
        plan.binding(),
        CurveSamplePolicy::RequireExact256,
        actual_source,
        source_profile,
        working_profile,
        remaining,
        control,
    )?;
    candidate.preparation_work = add(candidate.preparation_work, CONFIG_WORK)?;
    candidate.ceiling = limits;
    poll(control)?;
    Ok(candidate)
}

/// Complete prospective owned-node envelope. All profile/ICC/pipeline owners
/// are derived from constructor-owned borrowed CMS requirements, not caller sizes.
#[derive(Clone, Copy, Debug)]
pub struct ProfileOwnedColorRequirements {
    pub peak_resident_bytes: u64,
    pub retained_bytes: u64,
    pub work_units: u64,
}
struct ProfileOwnedPlan {
    required: ProfileOwnedColorRequirements,
    profiles: photocraft_cms::transform::EffectivePairRequirements,
    working_encoding: photocraft_cms::xcf_color::Encoding,
    owned_work: u64,
}
fn phase_limits(bytes: usize, work: u64) -> Result<control::Limits, RasterInputError> {
    let maximum = control::Limits::default();
    if bytes > maximum.resident_bytes || work > maximum.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(control::Limits { resident_bytes: bytes, work_units: work })
}
#[allow(clippy::too_many_arguments)]
fn profile_owned_plan(
    node: &FilterNode,
    schema: crate::gimp_configured_candidate::RetainedSchemaIdentity<'_>,
    basis: &Profile,
    current_encoding: photocraft_cms::xcf_color::Encoding,
    actual_source: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<ProfileOwnedPlan, RasterInputError> {
    use photocraft_algo::gimp::configured::Transfer;
    use photocraft_cms::xcf_color::Encoding;
    poll(control)?;
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units < CONFIG_WORK {
        return Err(RasterInputError::LimitExceeded);
    }
    let working_transfer = crate::gimp_configured_candidate::retained_source_transfer(node, schema, control)?;
    let source_transfer = match current_encoding {
        Encoding::Linear => Transfer::Linear,
        Encoding::NonlinearProfile => Transfer::NonlinearProfile,
        Encoding::Perceptual => Transfer::Perceptual,
    };
    if actual_source.transfer != source_transfer
        || actual_source.color_channels != basis.channels()
        || !matches!(basis.color_space, photocraft_cms::ColorSpace::Rgb | photocraft_cms::ColorSpace::Gray)
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let working_encoding = match working_transfer {
        Transfer::Linear => Encoding::Linear,
        Transfer::NonlinearProfile => Encoding::NonlinearProfile,
        Transfer::Perceptual => Encoding::Perceptual,
    };
    let maximum = control::Limits::default();
    let lower = control::Limits {
        resident_bytes: usize::try_from(limits.resident_bytes.min(maximum.resident_bytes as u64)).map_err(|_| RasterInputError::LimitExceeded)?,
        work_units: limits.work_units.min(maximum.work_units),
    };
    profile_domain(basis, lower, control)?;
    let profiles =
        Transform::effective_pair_requirements_with_control(basis, current_encoding, working_encoding, lower, &|| control.cancelled()).map_err(cms_error)?;
    let source_hash_bound = profiles.source_encoding.encoded_bytes_upper_bound.max(profiles.source_encoding.retained_bytes);
    let working_hash_bound = profiles.working_encoding.encoded_bytes_upper_bound.max(profiles.working_encoding.retained_bytes);
    let encoding_work = add(
        mul(add(profiles.source_encoding.work_units, profiles.working_encoding.work_units)?, 3)?,
        add(source_hash_bound as u64, working_hash_bound as u64)?,
    )?;
    let pair_work = mul(add(profiles.forward.work_units, profiles.inverse.work_units)?, 3)?;
    let owned_work = add(add(add(mul(CONFIG_WORK, 2)?, CENSUS_WORK)?, mul(PROFILE_VALIDATION_WORK, 2)?)?, add(encoding_work, pair_work)?)?;
    let effective_work = mul(add(profiles.source_profile.work_units, profiles.working_profile.work_units)?, 3)?;
    let hash_work = add(mul(profiles.working_encoding.work_units, 3)?, working_hash_bound as u64)?;
    // Two complete borrowed census passes cover graph preadmission and the actual
    // constructor's recheck; construction phase work is separately reserved below.
    let census = add(
        add(add(PROFILE_VALIDATION_WORK, CONFIG_WORK)?, add(profiles.source_profile.work_units, profiles.working_profile.work_units)?)?,
        add(add(profiles.source_encoding.work_units, profiles.working_encoding.work_units)?, add(profiles.forward.work_units, profiles.inverse.work_units)?)?,
    )?;
    let work_units = add(add(add(add(mul(census, 2)?, effective_work)?, hash_work)?, owned_work)?, CONFIG_WORK)?;
    let mut peak = usize::try_from(CONFIG_BYTES).map_err(|_| RasterInputError::LimitExceeded)?;
    for bytes in [
        profiles.original_retained_bytes,
        profiles.source_profile.peak_resident_bytes,
        profiles.working_profile.peak_resident_bytes,
        profiles.source_encoding.peak_resident_bytes,
        profiles.working_encoding.peak_resident_bytes,
        profiles.forward.peak_resident_bytes,
        profiles.inverse.peak_resident_bytes,
    ] {
        peak = peak.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?;
    }
    let retained = profiles
        .original_retained_bytes
        .checked_add(profiles.forward.peak_resident_bytes)
        .and_then(|n| n.checked_add(profiles.inverse.peak_resident_bytes))
        .ok_or(RasterInputError::LimitExceeded)?;
    let required = ProfileOwnedColorRequirements {
        peak_resident_bytes: u64::try_from(peak).map_err(|_| RasterInputError::LimitExceeded)?,
        retained_bytes: u64::try_from(retained).map_err(|_| RasterInputError::LimitExceeded)?,
        work_units,
    };
    if required.peak_resident_bytes > limits.resident_bytes || required.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    // Each eventual builder receives a fixed derived subquota; do not clamp or
    // reset a full CMS budget after earlier phases have retained their owners.
    phase_limits(profiles.source_profile.peak_resident_bytes, mul(profiles.source_profile.work_units, 3)?)?;
    phase_limits(profiles.working_profile.peak_resident_bytes, mul(profiles.working_profile.work_units, 3)?)?;
    phase_limits(profiles.working_encoding.peak_resident_bytes, mul(profiles.working_encoding.work_units, 3)?)?;
    phase_limits(profiles.forward.peak_resident_bytes, mul(profiles.forward.work_units, 3)?)?;
    phase_limits(profiles.inverse.peak_resident_bytes, mul(profiles.inverse.work_units, 3)?)?;
    poll(control)?;
    Ok(ProfileOwnedPlan { required, profiles, working_encoding, owned_work })
}
/// Allocation-free whole-node census before effective/profile/hash/pair copies.
/// Original node/source/graph backing remains the enclosing owner's reservation.
#[allow(clippy::too_many_arguments)]
pub fn profile_owned_color_requirements(
    node: &FilterNode,
    schema: crate::gimp_configured_candidate::RetainedSchemaIdentity<'_>,
    basis: &Profile,
    current_encoding: photocraft_cms::xcf_color::Encoding,
    actual_source: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<ProfileOwnedColorRequirements, RasterInputError> {
    profile_owned_plan(node, schema, basis, current_encoding, actual_source, limits, control).map(|plan| plan.required)
}
/// One explicit original definition and current encoding. Working transfer comes
/// only from the retained selector/TRC state; no absent/default profile inference.
#[allow(clippy::too_many_arguments)]
pub fn prepare_profile_owned_color_candidate<'a>(
    node: &'a FilterNode,
    schema: crate::gimp_configured_candidate::RetainedSchemaIdentity<'a>,
    basis: &Profile,
    current_encoding: photocraft_cms::xcf_color::Encoding,
    actual_source: WorkingContext,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<OwnedConfiguredColorCandidate<'a>, RasterInputError> {
    use photocraft_cms::xcf_color;
    // The complete envelope is checked before either effective definition exists.
    let plan = profile_owned_plan(node, schema, basis, current_encoding, actual_source, limits, control)?;
    let profiles = plan.profiles;
    let cancelled = || control.cancelled();
    let source_profile = xcf_color::effective_profile_from_definition_with_control(
        basis,
        current_encoding,
        phase_limits(profiles.source_profile.peak_resident_bytes, mul(profiles.source_profile.work_units, 3)?)?,
        &cancelled,
    )
    .map_err(cms_error)?;
    let working_profile = xcf_color::effective_profile_from_definition_with_control(
        basis,
        plan.working_encoding,
        phase_limits(profiles.working_profile.peak_resident_bytes, mul(profiles.working_profile.work_units, 3)?)?,
        &cancelled,
    )
    .map_err(cms_error)?;
    let source_model = source_profile
        .clone_requirements_with_control(phase_limits(profiles.source_profile.peak_resident_bytes, mul(profiles.source_profile.work_units, 3)?)?, &cancelled)
        .map_err(cms_error)?;
    let working_model = working_profile
        .clone_requirements_with_control(phase_limits(profiles.working_profile.peak_resident_bytes, mul(profiles.working_profile.work_units, 3)?)?, &cancelled)
        .map_err(cms_error)?;
    if source_model.retained_bytes > profiles.source_profile.peak_resident_bytes || working_model.retained_bytes > profiles.working_profile.peak_resident_bytes
    {
        return Err(RasterInputError::LimitExceeded);
    }
    let hash_bound = profiles.working_encoding.encoded_bytes_upper_bound.max(profiles.working_encoding.retained_bytes);
    let hash_limits = phase_limits(profiles.working_encoding.peak_resident_bytes, mul(profiles.working_encoding.work_units, 3)?)?;
    let bytes = working_profile.to_bytes_with_control(hash_limits, &cancelled).map_err(cms_error)?;
    let working_identity = digest(&bytes, hash_bound, control)?;
    drop(bytes);
    let identity = crate::gimp_configured_candidate::RetainedSourceIdentity {
        xcf_version: schema.xcf_version,
        expected_epoch: schema.expected_epoch,
        schema_sha256: schema.schema_sha256,
        profile_identity: working_identity,
        color_channels: basis.channels(),
        alpha: actual_source.alpha,
    };
    let candidate_limits = Limits {
        resident_bytes: limits.resident_bytes.checked_sub(profiles.original_retained_bytes as u64).ok_or(RasterInputError::LimitExceeded)?,
        work_units: plan.owned_work,
    };
    let retained_plan = crate::gimp_configured_candidate::prepare_retained_source_plan(
        node,
        identity,
        Limits { resident_bytes: CONFIG_BYTES, work_units: CONFIG_WORK },
        control,
    )?;
    let lower_candidate = Limits {
        resident_bytes: candidate_limits.resident_bytes,
        work_units: candidate_limits.work_units.checked_sub(CONFIG_WORK).ok_or(RasterInputError::LimitExceeded)?,
    };
    let mut candidate = prepare_owned_with_envelope(
        node,
        retained_plan.binding(),
        CurveSamplePolicy::RequireExact256,
        actual_source,
        &source_profile,
        &working_profile,
        lower_candidate,
        control,
        Some(&profiles),
    )?;
    candidate.preparation_work = add(candidate.preparation_work, CONFIG_WORK)?;
    candidate.retained_bytes = add(candidate.retained_bytes, profiles.original_retained_bytes as u64)?;
    if candidate.retained_bytes > plan.required.retained_bytes || candidate.preparation_work > plan.owned_work {
        return Err(RasterInputError::LimitExceeded);
    }
    // Keep the admitted aggregate work/residency through rendering; no upstream
    // graph/source reservation is refunded when temporary definitions are dropped.
    candidate.preparation_work = plan.required.work_units;
    candidate.ceiling = limits;
    poll(control)?;
    Ok(candidate)
}
