//! Explicit opt-in typed configured source candidate, outside admitted dispatch.
//! A schema/epoch witness is a caller assertion, not rendering qualification.
use photocraft_algo::gimp::configured::{
    self, CurveChannel, CurveMode, CurvePointKind, CurveSamplePolicy, CurvesConfiguration, LevelsChannelBits, LevelsConfigurationBits, Limits,
    PreparedConfiguration, Transfer, WorkingContext,
};
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue};
use photocraft_raster::{Interrupt, RasterInputError};

pub const CURVES_SCHEMA: &str = "3648b9ad75319e80e6b5430c13f7b8eada6cfb9cfc69c72599585ea8c8aacf88";
pub const LEVELS_SCHEMA: &str = "4ff18f4784df8602ac2c54684f1502de3c5cbbbff4ca07c65e21e2b8067ed346";
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Defaults use serialized counts derived by prepare_retained_source_plan.
    /// Only an explicit external authoring witness may supply a different request.
    Curves {
        requested_samples: [usize; 5],
    },
    Levels,
}
#[derive(Clone, Copy, Debug)]
pub struct SourceBinding<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    /// Exact complete frozen public schema SHA256, not the short coverage suffix.
    pub schema_sha256: &'a str,
    pub kind: Kind,
    pub working: WorkingContext,
}
/// Caller identity/context only. Transfer and serialized channel forms are
/// derived from the retained typed config, never guessed by the caller.
#[derive(Clone, Copy, Debug)]
pub struct RetainedSourceIdentity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub profile_identity: [u8; 32],
    pub color_channels: usize,
    pub alpha: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelForm {
    Levels,
    Curve { mode: CurveMode, serialized_samples: usize },
}
/// Borrowed validated current configuration. The parser guarantees serialized
/// n-samples equals the retained table length; it is not missing metadata.
/// A pre-authoring requested length is different external witness information.
pub struct RetainedSourcePlan<'a> {
    source: BoundSource<'a>,
    forms: [ChannelForm; 5],
}
impl<'a> RetainedSourcePlan<'a> {
    pub fn binding(&self) -> SourceBinding<'a> {
        self.source.binding
    }
    pub fn original_node(&self) -> &'a FilterNode {
        self.source.node
    }
    pub fn channel_forms(&self) -> &[ChannelForm; 5] {
        &self.forms
    }
    pub fn try_apply_pixels(&self, input: &[f64], actual: WorkingContext, limits: Limits, control: &Interrupt<'_>) -> Result<Vec<f64>, RasterInputError> {
        self.source.try_apply_pixels(input, actual, limits, control)
    }
}
/// Default strict constructor for actual retained typed config, including
/// changed values. All numeric configuration preparation validates before the
/// plan returns. No authoring-request length or sampling policy is inferred.
pub fn prepare_retained_source_plan<'a>(
    node: &'a FilterNode,
    identity: RetainedSourceIdentity<'a>,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<RetainedSourcePlan<'a>, RasterInputError> {
    poll(control)?;
    let kind = match node.operation.name.as_str() {
        "gimp:curves" => Kind::Curves { requested_samples: [0; 5] },
        "gimp:levels" => Kind::Levels,
        _ => return Err(unsupported()),
    };
    let (channels, transfer, _, _) = channels(node, kind)?;
    let mut forms = [ChannelForm::Levels; 5];
    let kind = match kind {
        Kind::Curves { .. } => {
            let mut serialized = [0; 5];
            for ((channel, count), form) in channels.iter().zip(serialized.iter_mut()).zip(forms.iter_mut()) {
                poll(control)?;
                let FilterValue::Curve { free, samples_f64_bits, .. } = &channel.value else {
                    return Err(unsupported());
                };
                *count = samples_f64_bits.len();
                *form = ChannelForm::Curve { mode: if *free { CurveMode::Free } else { CurveMode::Smooth }, serialized_samples: *count };
            }
            Kind::Curves { requested_samples: serialized }
        }
        Kind::Levels => Kind::Levels,
    };
    let working = WorkingContext { transfer, profile_identity: identity.profile_identity, color_channels: identity.color_channels, alpha: identity.alpha };
    let binding =
        SourceBinding { xcf_version: identity.xcf_version, expected_epoch: identity.expected_epoch, schema_sha256: identity.schema_sha256, kind, working };
    let source = bind_source_node(node, binding, control)?;
    source.with_prepared(working, limits, control, |_| Ok(()))?;
    poll(control)?;
    Ok(RetainedSourcePlan { source, forms })
}

pub struct BoundSource<'a> {
    sampling: CurveSamplePolicy,
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
}
fn unsupported() -> RasterInputError {
    RasterInputError::UnsupportedConversion
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn transfer(value: &FilterArgument, kind: Kind) -> Result<Transfer, RasterInputError> {
    let domain = match kind {
        Kind::Curves { .. } => "gimp:curves::trc",
        Kind::Levels => "gimp:levels::trc",
    };
    match &value.value {
        FilterValue::Enum { domain: actual, value: tag } if value.name == "trc" && actual == domain => Transfer::from_stored(*tag),
        _ => Err(unsupported()),
    }
}
fn channels<'a>(node: &'a FilterNode, kind: Kind) -> Result<(&'a [FilterArgument], Transfer, bool, bool), RasterInputError> {
    let (trc, config, clamp_input, clamp_output) = match (kind, node.arguments.as_slice()) {
        (Kind::Curves { .. }, [trc, config]) => (trc, config, false, false),
        (Kind::Levels, [trc, ci, co, config]) if ci.name == "clamp-input" && co.name == "clamp-output" => {
            let (FilterValue::Bool(ci), FilterValue::Bool(co)) = (&ci.value, &co.value) else {
                return Err(unsupported());
            };
            (trc, config, *ci, *co)
        }
        _ => return Err(unsupported()),
    };
    let FilterValue::Config(channels) = &config.value else {
        return Err(unsupported());
    };
    if config.name != "config" || channels.len() != 5 {
        return Err(unsupported());
    }
    for (channel, name) in channels.iter().zip(["value", "red", "green", "blue", "alpha"]) {
        if channel.name != name {
            return Err(unsupported());
        }
    }
    Ok((channels, transfer(trc, kind)?, clamp_input, clamp_output))
}
/// Borrowed source shape admission; does not reinterpret opacity/mask/Replace
/// envelopes, resolve resources, regenerate curves, or select a source renderer.
pub fn bind_source_node<'a>(node: &'a FilterNode, binding: SourceBinding<'a>, control: &Interrupt<'_>) -> Result<BoundSource<'a>, RasterInputError> {
    bind_source_node_with_sample_policy(node, binding, CurveSamplePolicy::RequireExact256, control)
}
pub fn bind_source_node_with_sample_policy<'a>(
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    sampling: CurveSamplePolicy,
    control: &Interrupt<'_>,
) -> Result<BoundSource<'a>, RasterInputError> {
    poll(control)?;
    let (name, schema) = match binding.kind {
        Kind::Curves { .. } => ("gimp:curves", CURVES_SCHEMA),
        Kind::Levels => ("gimp:levels", LEVELS_SCHEMA),
    };
    if binding.xcf_version != 22
        || binding.expected_epoch.is_empty()
        || binding.expected_epoch.len() > 128
        || binding.expected_epoch.contains('\0')
        || node.id.0 == 0
        || node.operation.name != name
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != binding.expected_epoch
        || binding.schema_sha256 != schema
    {
        return Err(unsupported());
    }
    let (channels, trc, _, _) = channels(node, binding.kind)?;
    if let Kind::Curves { requested_samples } = binding.kind {
        for (channel, requested) in channels.iter().zip(requested_samples) {
            let FilterValue::Curve { free, samples_f64_bits, .. } = &channel.value else {
                return Err(unsupported());
            };
            if !sampling.permits(if *free { CurveMode::Free } else { CurveMode::Smooth }, requested, samples_f64_bits.len()) {
                return Err(unsupported());
            }
        }
    }
    if binding.working.transfer != trc {
        return Err(unsupported());
    }
    for channel in channels {
        poll(control)?;
        match (binding.kind, &channel.value) {
            (Kind::Curves { .. }, FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. })
                if points_f64_bits.len() <= 64
                    && point_types.len() == points_f64_bits.len()
                    && samples_f64_bits.len() == 256
                    && point_types.iter().all(|tag| matches!(tag, 0 | 1)) => {}
            (Kind::Levels, FilterValue::Config(fields)) if fields.len() == 5 => {
                for (field, name) in fields.iter().zip(["low-input", "high-input", "gamma", "low-output", "high-output"]) {
                    if field.name != name || !matches!(field.value, FilterValue::Number(ExactNumber::Float64Bits(_))) {
                        return Err(unsupported());
                    }
                }
            }
            _ => return Err(unsupported()),
        }
    }
    poll(control)?;
    Ok(BoundSource { node, binding, sampling })
}
impl BoundSource<'_> {
    pub fn original_node(&self) -> &FilterNode {
        self.node
    }
    /// Preparation and execution share one bounded stack lifetime, without
    /// retained clones or a self-referential configuration. No prepared object
    /// can escape with references to local scratch.
    pub fn with_prepared<T>(
        &self,
        actual: WorkingContext,
        limits: Limits,
        control: &Interrupt<'_>,
        run: impl FnOnce(&PreparedConfiguration<'_>) -> Result<T, RasterInputError>,
    ) -> Result<T, RasterInputError> {
        poll(control)?;
        let (channels, trc, ci, co) = channels(self.node, self.binding.kind)?;
        let result = match self.binding.kind {
            Kind::Curves { requested_samples } => {
                // Fallible source validation precedes fixed scratch construction;
                // the extra64x5 enum bytes are charged in the local lower budget.
                let scratch = std::mem::size_of::<[[CurvePointKind; 64]; 5]>() as u64;
                let lowered = Limits {
                    resident_bytes: limits.resident_bytes.checked_sub(scratch).ok_or(RasterInputError::LimitExceeded)?,
                    work_units: limits.work_units,
                };
                let mut types = [[CurvePointKind::Smooth; 64]; 5];
                for (channel, output) in channels.iter().zip(types.iter_mut()) {
                    let FilterValue::Curve { point_types, .. } = &channel.value else {
                        return Err(unsupported());
                    };
                    for (tag, dst) in point_types.iter().zip(output.iter_mut()) {
                        poll(control)?;
                        *dst = match tag {
                            0 => CurvePointKind::Smooth,
                            1 => CurvePointKind::Corner,
                            _ => return Err(unsupported()),
                        };
                    }
                }
                let empty = CurveChannel { mode: CurveMode::Free, requested_samples: 256, points_f64_bits: &[], point_types: &[], samples_f64_bits: &[] };
                let mut decoded = [empty; 5];
                for (((channel, kinds), requested), output) in channels.iter().zip(types.iter()).zip(requested_samples).zip(decoded.iter_mut()) {
                    poll(control)?;
                    let FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } = &channel.value else {
                        return Err(unsupported());
                    };
                    *output = CurveChannel {
                        mode: if *free { CurveMode::Free } else { CurveMode::Smooth },
                        requested_samples: requested,
                        points_f64_bits,
                        point_types: kinds.get(..point_types.len()).ok_or_else(unsupported)?,
                        samples_f64_bits,
                    };
                }
                let config = CurvesConfiguration { transfer: trc, channels: decoded };
                let prepared = configured::prepare_curves_with_sample_policy(&config, self.sampling, actual, self.binding.working, lowered, control)?;
                run(&prepared)
            }
            Kind::Levels => {
                let empty = LevelsChannelBits { low_input: 0, high_input: 0, gamma: 0, low_output: 0, high_output: 0 };
                let mut decoded = [empty; 5];
                for (channel, output) in channels.iter().zip(decoded.iter_mut()) {
                    poll(control)?;
                    let FilterValue::Config(fields) = &channel.value else {
                        return Err(unsupported());
                    };
                    let mut bits = [0; 5];
                    for (field, dst) in fields.iter().zip(bits.iter_mut()) {
                        poll(control)?;
                        let FilterValue::Number(ExactNumber::Float64Bits(value)) = field.value else {
                            return Err(unsupported());
                        };
                        *dst = value;
                    }
                    let [low_input, high_input, gamma, low_output, high_output] = bits;
                    *output = LevelsChannelBits { low_input, high_input, gamma, low_output, high_output };
                }
                let config = LevelsConfigurationBits { transfer: trc, channels: decoded, clamp_input: ci, clamp_output: co };
                let prepared = configured::prepare_levels(&config, actual, self.binding.working, limits, control)?;
                run(&prepared)
            }
        }?;
        poll(control)?;
        Ok(result)
    }
    pub fn try_apply_pixels(&self, input: &[f64], actual: WorkingContext, limits: Limits, control: &Interrupt<'_>) -> Result<Vec<f64>, RasterInputError> {
        let scratch = match self.binding.kind {
            Kind::Curves { .. } => std::mem::size_of::<[[CurvePointKind; 64]; 5]>() as u64,
            Kind::Levels => 0,
        };
        let execution =
            Limits { resident_bytes: limits.resident_bytes.checked_sub(scratch).ok_or(RasterInputError::LimitExceeded)?, work_units: limits.work_units };
        self.with_prepared(actual, limits, control, |prepared| prepared.try_apply_pixels(input, actual, execution, control))
    }
}

/// Exact retained operation/schema identity, without a guessed working profile.
#[derive(Clone,Copy,Debug)]
pub struct RetainedSchemaIdentity<'a> {
    pub xcf_version:u32,
    pub expected_epoch:&'a str,
    pub schema_sha256:&'a str,
}
/// Borrowed five-selector/TRC admission before profile allocation. Numeric
/// preparation later consumes the same retained fields; no getter proxy is used.
pub fn retained_source_transfer(node:&FilterNode,identity:RetainedSchemaIdentity<'_>,control:&Interrupt<'_>)->Result<Transfer,RasterInputError>{
    poll(control)?;
    let (kind,schema)=match node.operation.name.as_str(){"gimp:levels"=>(Kind::Levels,LEVELS_SCHEMA),"gimp:curves"=>(Kind::Curves{requested_samples:[256;5]},CURVES_SCHEMA),_=>return Err(unsupported())};
    if identity.xcf_version!=22||identity.expected_epoch.is_empty()||identity.expected_epoch.len()>128||identity.expected_epoch.contains('\0')||node.id.0==0||node.operation.stored_version!="0:0"||node.operation.schema_epoch!=identity.expected_epoch||identity.schema_sha256!=schema{return Err(unsupported());}
    let (groups,trc,_,_)=channels(node,kind)?;
    for group in groups {
        poll(control)?;
        match (kind,&group.value) {
            (Kind::Curves{..},FilterValue::Curve{points_f64_bits,point_types,samples_f64_bits,..}) if points_f64_bits.len()<=64&&point_types.len()==points_f64_bits.len()&&samples_f64_bits.len()==256&&point_types.iter().all(|tag|matches!(tag,0|1))=>{
                let mut prior=None;
                for point in points_f64_bits {poll(control)?;let [x,y]=point.map(f64::from_bits);if !x.is_finite()||!y.is_finite()||!(0.0..=1.0).contains(&x)||!(0.0..=1.0).contains(&y)||prior.is_some_and(|p|x<=p){return Err(unsupported());}prior=Some(x);}
                let mut samples=[0.0;256];for (bits,value) in samples_f64_bits.iter().zip(samples.iter_mut()){poll(control)?;*value=f64::from_bits(*bits);}
                photocraft_algo::gimp::curves::FreeCurve{samples:&samples}.validate()?;
            },
            (Kind::Levels,FilterValue::Config(fields)) if fields.len()==5=>{
                let mut values=[0.0;5];
                for ((field,name),value) in fields.iter().zip(["low-input","high-input","gamma","low-output","high-output"]).zip(values.iter_mut()) {
                    if field.name!=name{return Err(unsupported());}
                    let FilterValue::Number(ExactNumber::Float64Bits(bits))=field.value else{return Err(unsupported());};*value=f64::from_bits(bits);
                }
                let [low_input,high_input,gamma,low_output,high_output]=values;
                let channel=photocraft_algo::gimp::levels::LevelsChannel{low_input,high_input,gamma,low_output,high_output};
                photocraft_algo::gimp::levels::LevelsConfiguration{channels:[channel;5],clamp_input:false,clamp_output:false}.validate()?;
            },
            _=>return Err(unsupported()),
        }
    }
    poll(control)?;Ok(trc)
}
