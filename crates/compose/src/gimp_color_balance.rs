//! Structured Color Balance: exact current arguments, immutable source evidence.
use crate::gimp_composite::{CompositionError, Result, WorkingSpace};
use photocraft_algo::gimp::color_balance::ColorBalance;
use photocraft_cms::{Profile, WideWork};
use photocraft_doc::{
    SampleEncoding,
    import_compositing::GimpSpace,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};

pub(crate) const EPOCH: &str = "xcf-22-stock-color-balance-precision-v1";
pub(crate) const DEPTH_EPOCH: &str = "xcf-22-stock-color-balance-depth-v1";
pub(crate) struct ColorBalanceNode {
    config: ColorBalance,
    working: WorkingSpace,
}
impl ColorBalanceNode {
    pub(crate) fn new(node: &FilterNode, profile: &Profile, stored: SampleEncoding, version: u32) -> Result<Self> {
        Self::new_with_interrupt(node,profile,stored,version,&photocraft_raster::Interrupt::NONE)
    }
    pub(crate) fn new_with_interrupt(node:&FilterNode,profile:&Profile,stored:SampleEncoding,version:u32,control:&photocraft_raster::Interrupt<'_>)->Result<Self> {
        control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
        let policy = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Color Balance policy"))?;
        let depth = node.operation.schema_epoch == DEPTH_EPOCH;
        let raw = if depth { -1215462831 } else { -113999279 };
        if version != 22
            || node.operation.name != "gimp:color-balance"
            || node.operation.stored_version != "0:0"
            || !matches!(node.operation.schema_epoch.as_str(), EPOCH | DEPTH_EPOCH)
            || policy.compatibility_epoch != node.operation.schema_epoch
            || policy.source_mode != 62
            || policy.legacy
            || policy.pass_through
            || policy.resolved.is_some()
            || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (raw, raw, raw)
            || node.opacity_f64_bits != 1.0_f64.to_bits()
            || node.source_clipping != Some(0)
            || node.source_region != Some(0)
            || !node.auxiliary.is_empty()
            || profile.channels() != 3
            || !(matches!(stored, SampleEncoding::Linear | SampleEncoding::NonlinearProfile) || depth && stored == SampleEncoding::Perceptual)
        {
            return Err(CompositionError::Unsupported("unqualified Color Balance envelope"));
        }
        let [preserve, configuration] = node.arguments.as_slice() else {
            return Err(CompositionError::Unsupported("Color Balance arguments"));
        };
        let FilterValue::Bool(preserve_luminosity) = preserve.value else {
            return Err(CompositionError::Unsupported("Color Balance luminosity type"));
        };
        let FilterValue::Config(source) = &configuration.value else {
            return Err(CompositionError::Unsupported("Color Balance range type"));
        };
        if preserve.name != "preserve-luminosity" || configuration.name != "config" || source.len() != 3 {
            return Err(CompositionError::Unsupported("Color Balance ordered schema"));
        }
        let mut ranges = [[0.0; 3]; 3];
        for ((argument, name), output) in source.iter().zip(["shadows", "midtones", "highlights"]).zip(ranges.iter_mut()) {
            control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
            let FilterValue::Config(fields) = &argument.value else {
                return Err(CompositionError::Unsupported("Color Balance scalar type"));
            };
            if argument.name != name || fields.len() != 3 {
                return Err(CompositionError::Unsupported("Color Balance range order"));
            }
            for ((field, name), value) in fields.iter().zip(["cyan-red", "magenta-green", "yellow-blue"]).zip(output.iter_mut()) {
                let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field.value else {
                    return Err(CompositionError::Unsupported("Color Balance exact scalar type"));
                };
                if field.name != name {
                    return Err(CompositionError::Unsupported("Color Balance scalar order"));
                }
                *value = f64::from_bits(bits);
            }
        }
        let config = ColorBalance { ranges, preserve_luminosity };
        config.validate()?;
        Ok(Self { config, working: WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::ProfileRgb,control)? })
    }
    pub(crate) fn stock_retained_bytes(&self)->u64 { std::mem::size_of::<Self>() as u64 +65536 }
    pub(crate) fn validate_stock_input(&self,pixel:[f64;4],poll:&mut impl FnMut(WideWork)->Result<()>)->Result<()> {
        poll(WideWork::SampleUnits(384))?;
        if pixel.iter().any(|value|!value.is_finite() || !(0.0..=1.0).contains(value)) || !(0.0..=1.0).contains(&pixel[3]) {return Err(CompositionError::Unsupported("configured stock finite native colors/unit alpha"))}
        let color=self.working.convert([pixel[0],pixel[1],pixel[2]],true)?;
        if color.iter().any(|value|!value.is_finite() || !(0.0..=1.0).contains(value)) {return Err(CompositionError::Unsupported("configured stock working input"))}
        Ok(())
    }
    pub(crate) fn apply(&self, pixel: [f64; 4], poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        poll(WideWork::SampleUnits(384))?;
        let rgb = self.working.convert([pixel[0], pixel[1], pixel[2]], true)?;
        let mut output = [0.0; 4];
        self.config.try_eval_pixel(&[rgb[0], rgb[1], rgb[2], pixel[3]], &mut output)?;
        let rgb = self.working.convert([output[0], output[1], output[2]], false)?;
        Ok([rgb[0], rgb[1], rgb[2], output[3]])
    }
}
