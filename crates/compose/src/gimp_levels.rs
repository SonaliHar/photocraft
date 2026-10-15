//! Checked structured Levels. Configuration and source pixels remain immutable.
use crate::gimp_composite::{CompositionError, Result, WorkingSpace};
use photocraft_algo::gimp::levels::{LevelsChannel, LevelsConfiguration};
use photocraft_cms::{Profile, WideWork};
use photocraft_doc::{
    SampleEncoding,
    import_compositing::GimpSpace,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};

pub(crate) struct LevelsNode {
    config: LevelsConfiguration,
    working: WorkingSpace,
}
impl LevelsNode {
    pub(crate) fn new(node: &FilterNode, profile: &Profile, stored: SampleEncoding, version: u32) -> Result<Self> {
        Self::new_with_interrupt(node,profile,stored,version,&photocraft_raster::Interrupt::NONE)
    }
    pub(crate) fn new_with_interrupt(node:&FilterNode,profile:&Profile,stored:SampleEncoding,version:u32,control:&photocraft_raster::Interrupt<'_>)->Result<Self> {
        control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
        let policy = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing structured Levels policy"))?;
        if version != 22
            || node.operation.stored_version != "0:0"
            || node.operation.schema_epoch != "xcf-22-stock-levels-replace"
            || policy.source_mode != 62
            || policy.legacy
            || policy.pass_through
            || policy.resolved.is_some()
            || policy.stored_blend_space != -1080065455
            || policy.stored_composite_space != -1080065455
            || policy.stored_operator != -1080065455
            || node.opacity_f64_bits != 1.0f64.to_bits()
            || node.source_clipping != Some(0)
            || node.source_region != Some(0)
            || !node.auxiliary.is_empty()
        {
            return Err(CompositionError::Unsupported("unqualified structured Levels envelope"));
        }
        let [trc, ci, co, config] = node.arguments.as_slice() else {
            return Err(CompositionError::Unsupported("structured Levels arguments"));
        };
        let FilterValue::Enum { domain, value } = &trc.value else {
            return Err(CompositionError::Unsupported("Levels TRC type"));
        };
        if trc.name != "trc" || domain != "gimp:levels::trc" || !matches!((*value, stored), (0, SampleEncoding::Linear) | (1, SampleEncoding::NonlinearProfile))
        {
            return Err(CompositionError::Unsupported("Levels TRC/source precision domain"));
        }
        let FilterValue::Bool(clamp_input) = ci.value else {
            return Err(CompositionError::Unsupported("Levels input clamp type"));
        };
        let FilterValue::Bool(clamp_output) = co.value else {
            return Err(CompositionError::Unsupported("Levels output clamp type"));
        };
        if ci.name != "clamp-input" || co.name != "clamp-output" || config.name != "config" {
            return Err(CompositionError::Unsupported("Levels ordered schema"));
        }
        let FilterValue::Config(channels) = &config.value else {
            return Err(CompositionError::Unsupported("Levels channels"));
        };
        if channels.len() != 5 {
            return Err(CompositionError::Unsupported("Levels selector count"));
        }
        let mut decoded = [LevelsChannel::default(); 5];
        for ((arg, name), channel) in channels.iter().zip(["value", "red", "green", "blue", "alpha"]).zip(decoded.iter_mut()) {
            control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
            let FilterValue::Config(fields) = &arg.value else {
                return Err(CompositionError::Unsupported("Levels channel type"));
            };
            if arg.name != name || fields.len() != 5 {
                return Err(CompositionError::Unsupported("Levels ordered selectors"));
            }
            let mut numbers = [0.0; 5];
            for ((field, name), number) in fields.iter().zip(["low-input", "high-input", "gamma", "low-output", "high-output"]).zip(numbers.iter_mut()) {
                let FilterValue::Number(ExactNumber::Float64Bits(bits)) = field.value else {
                    return Err(CompositionError::Unsupported("Levels scalar stored type"));
                };
                if field.name != name {
                    return Err(CompositionError::Unsupported("Levels ordered scalar schema"));
                }
                *number = f64::from_bits(bits);
            }
            *channel = LevelsChannel { low_input: numbers[0], high_input: numbers[1], gamma: numbers[2], low_output: numbers[3], high_output: numbers[4] };
        }
        let config = LevelsConfiguration { channels: decoded, clamp_input, clamp_output };
        config.validate()?;
        let working = WorkingSpace::new_with_interrupt(profile, stored, if *value == 0 { GimpSpace::LinearRgb } else { GimpSpace::ProfileRgb },control)?;
        Ok(Self { config, working })
    }
    pub(crate) fn stock_retained_bytes(&self)->u64 { std::mem::size_of::<Self>() as u64 +65536 }
    pub(crate) fn validate_stock_input(&self,pixel:[f64;4],poll:&mut impl FnMut(WideWork)->Result<()>)->Result<()> {
        poll(WideWork::SampleUnits(384))?;
        if pixel.iter().any(|value|!value.is_finite()) || !(0.0..=1.0).contains(&pixel[3]) {return Err(CompositionError::Unsupported("configured stock finite native colors/unit alpha"))}
        let color=self.working.convert([pixel[0],pixel[1],pixel[2]],true)?;
        if color.iter().any(|value|!value.is_finite()) {return Err(CompositionError::Unsupported("configured stock working input"))}
        Ok(())
    }
    pub(crate) fn apply(&self, pixel: [f64; 4], poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        poll(WideWork::SampleUnits(320))?;
        if pixel.iter().any(|sample| !sample.is_finite()) {
            return Err(CompositionError::Invalid("nonfinite structured Levels sample"));
        }
        crate::gimp_composite::coverage(pixel[3])?;
        let color = self.working.convert([pixel[0], pixel[1], pixel[2]], true)?;
        let mut output = [0.0; 4];
        self.config.try_eval_pixel(&[color[0], color[1], color[2], pixel[3]], 3, &mut output)?;
        let color = self.working.convert([output[0], output[1], output[2]], false)?;
        // This qualified stock envelope uses the finite explicit Alpha result as
        // coverage at the compositor boundary. Original unclamped selector
        // fixtures witness negative/>1 Alpha becoming zero/one coverage;
        // color HDR and immutable source samples remain unclamped.
        Ok([color[0], color[1], color[2], output[3].clamp(0.0, 1.0)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_cms::Builtin;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node() -> FilterNode {
        let channels = ["value", "red", "green", "blue", "alpha"].map(|name| FilterArgument {
            name: name.into(),
            value: FilterValue::Config(
                [("low-input", 0.0f64), ("high-input", 1.0), ("gamma", 1.0), ("low-output", 0.0), ("high-output", 1.0)]
                    .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(value.to_bits())) })
                    .into(),
            ),
        });
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:levels".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-levels-replace".into() },
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            arguments: vec![
                FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:levels::trc".into(), value: 0 } },
                FilterArgument { name: "clamp-input".into(), value: FilterValue::Bool(false) },
                FilterArgument { name: "clamp-output".into(), value: FilterValue::Bool(false) },
                FilterArgument { name: "config".into(), value: FilterValue::Config(channels.into()) },
            ],
            composition: Some(GimpComposition {
                compatibility_epoch: "test".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1080065455,
                stored_composite_space: -1080065455,
                stored_operator: -1080065455,
                pass_through: false,
                resolved: None,
            }),
        }
    }
    #[test]
    fn structured_levels_hdr_alpha_schema_and_cancellation() {
        let original = node();
        let evaluate = LevelsNode::new(&original, Builtin::Srgb.profile(), SampleEncoding::Linear, 22).unwrap();
        let pixel = [-3.0, 2.0, 1.0 + f64::EPSILON, 0.125];
        assert_eq!(evaluate.apply(pixel, &mut |_| Ok(())).unwrap(), pixel);
        assert!(evaluate.apply(pixel, &mut |_| Err(CompositionError::Invalid("cancel"))).is_err());
        assert!(evaluate.apply([f64::NAN, 0.0, 0.0, 0.5], &mut |_| Ok(())).is_err());
        assert!(evaluate.apply([0.0, 0.0, 0.0, 2.0], &mut |_| Ok(())).is_err());
        let mut modified = original.clone();
        let FilterValue::Config(channels) = &mut modified.arguments[3].value else { panic!() };
        let FilterValue::Config(fields) = &mut channels[4].value else { panic!() };
        fields[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        fields[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.75f64.to_bits()));
        let evaluate = LevelsNode::new(&modified, Builtin::Srgb.profile(), SampleEncoding::Linear, 22).unwrap();
        assert_eq!(evaluate.apply([-3.0, 2.0, 1.0, 0.0], &mut |_| Ok(())).unwrap(), [-3.0, 2.0, 1.0, 0.0]);
        assert_eq!(evaluate.apply([-3.0, 2.0, 1.0, 1.0], &mut |_| Ok(())).unwrap(), [-3.0, 2.0, 1.0, 1.0]);
        assert_eq!(original, node());
        modified.arguments[0].value = FilterValue::Enum { domain: "gimp:levels::trc".into(), value: 2 };
        assert!(LevelsNode::new(&modified, Builtin::Srgb.profile(), SampleEncoding::Linear, 22).is_err());
    }
    #[test]
    #[ignore = "24MP release structured Levels kernel timing"]
    fn bench_structured_levels_24mp() {
        use std::{hint::black_box, time::Instant};
        let mut source = node();
        let FilterValue::Config(channels) = &mut source.arguments[3].value else { panic!() };
        for channel in channels.iter_mut() {
            let FilterValue::Config(fields) = &mut channel.value else { panic!() };
            fields[2].value = FilterValue::Number(ExactNumber::Float64Bits(1.31f64.to_bits()));
        }
        let evaluate = LevelsNode::new(&source, Builtin::Srgb.profile(), SampleEncoding::Linear, 22).unwrap();
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(evaluate.apply(black_box([0.23, 0.49, 0.77, 0.63]), &mut |_| Ok(())).unwrap());
        }
        println!("24MP structured Levels seven-gamma checked kernel {:?}", start.elapsed());
    }
}
