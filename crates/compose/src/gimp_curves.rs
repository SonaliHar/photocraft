//! Checked, narrowly qualified stored FREE curve tables; no point regeneration.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::curves::OwnedPreparedCurveTables;
use photocraft_cms::WideWork;
use photocraft_doc::xcf_filters::{FilterNode, FilterValue};

pub(crate) struct CurvesNode {
    tables: OwnedPreparedCurveTables,
}
impl CurvesNode {
    pub(crate) fn new(node: &FilterNode, version: u32) -> Result<Self> {
        Self::prepare(node,version,&photocraft_raster::Interrupt::NONE,false)
    }
    pub(crate) fn new_with_interrupt(node:&FilterNode,version:u32,control:&photocraft_raster::Interrupt<'_>)->Result<Self> {
        Self::prepare(node,version,control,true)
    }
    fn prepare(node:&FilterNode,version:u32,control:&photocraft_raster::Interrupt<'_>,bounded:bool)->Result<Self> {
        control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
        let policy = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing curves policy"))?;
        if version != 22
            || node.operation.stored_version != "0:0"
            || node.operation.schema_epoch != "xcf-22-stock-curves-replace"
            || policy.source_mode != 62
            || policy.legacy
            || policy.pass_through
            || policy.resolved.is_some()
            || policy.stored_blend_space != -1080065455
            || policy.stored_composite_space != -1080065455
            || policy.stored_operator != -1080065455
            || node.opacity_f64_bits != 1.0_f64.to_bits()
            || node.source_clipping != Some(0)
            || node.source_region != Some(0)
            || !node.auxiliary.is_empty()
        {
            return Err(CompositionError::Unsupported("unqualified FREE curves Replace envelope"));
        }
        let [trc, config] = node.arguments.as_slice() else {
            return Err(CompositionError::Unsupported("FREE curves arguments"));
        };
        if trc.name != "trc" || trc.value != (FilterValue::Enum { domain: "gimp:curves::trc".into(), value: 1 }) || config.name != "config" {
            return Err(CompositionError::Unsupported("FREE curves transfer/schema"));
        }
        let FilterValue::Config(channels) = &config.value else {
            return Err(CompositionError::Unsupported("FREE curves config"));
        };
        if channels.len() != 5 {
            return Err(CompositionError::Unsupported("FREE curves channels"));
        }
        let mut tables: [Vec<f64>; 5] = std::array::from_fn(|_| Vec::new());
        for ((arg, name), table) in channels.iter().zip(["value", "red", "green", "blue", "alpha"]).zip(tables.iter_mut()) {
            control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
            let FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } = &arg.value else {
                return Err(CompositionError::Unsupported("FREE curves channel type"));
            };
            if arg.name != name || !free || !points_f64_bits.is_empty() || !point_types.is_empty() || samples_f64_bits.len() != 256 {
                return Err(CompositionError::Unsupported("FREE256 stored schema"));
            }
            table.try_reserve_exact(256).map_err(|_| CompositionError::Invalid("FREE curves allocation denied"))?;
            if bounded && table.capacity()>512 {return Err(CompositionError::Unsupported("FREE256 temporary capacity envelope"))}
            for block in samples_f64_bits.chunks(64) {
                control.check().map_err(|_|photocraft_raster::RasterInputError::Cancelled)?;
                table.extend(block.iter().map(|bits| f64::from_bits(*bits)));
            }
        }
        Ok(Self { tables: OwnedPreparedCurveTables::try_new(tables)? })
    }
    pub(crate) fn stock_retained_bytes(&self)->u64 { std::mem::size_of::<Self>() as u64 }
    pub(crate) fn validate_stock_input(&self,pixel:[f64;4],poll:&mut impl FnMut(WideWork)->Result<()>)->Result<()> {
        poll(WideWork::SampleUnits(160))?;
        if pixel.iter().any(|value|!value.is_finite() || !(0.0..=1.0).contains(value)) {return Err(CompositionError::Unsupported("FREE256 current unit native input"))}
        Ok(())
    }
    pub(crate) fn apply(&self, pixel: [f64; 4], poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        poll(WideWork::SampleUnits(160))?;
        let mut output = [0.0; 4];
        self.tables.try_eval_pixel(&pixel, 3, &mut output)?;
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:curves".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-curves-replace".into() },
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            arguments: vec![
                FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:curves::trc".into(), value: 1 } },
                FilterArgument {
                    name: "config".into(),
                    value: FilterValue::Config(
                        ["value", "red", "green", "blue", "alpha"]
                            .map(|name| FilterArgument {
                                name: name.into(),
                                value: FilterValue::Curve {
                                    free: true,
                                    points_f64_bits: vec![],
                                    point_types: vec![],
                                    samples_f64_bits: (0..256).map(|i| (f64::from(i) / 255.0).to_bits()).collect(),
                                },
                            })
                            .into(),
                    ),
                },
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
    fn free_schema_and_alpha_are_checked_without_source_mutation() {
        let original = node();
        let curves = CurvesNode::new(&original, 22).unwrap();
        let input = [0.25, 0.5, 0.75, 0.125];
        let actual = curves.apply(input, &mut |_| Ok(())).unwrap();
        for (a, b) in actual.iter().zip(input) {
            assert!((a - b).abs() < 1e-15);
        }
        assert!(curves.apply(input, &mut |_| Err(CompositionError::Invalid("cancel"))).is_err());
        for invalid in [21, 23] {
            assert!(CurvesNode::new(&original, invalid).is_err());
        }
        let mut invalid = original.clone();
        let FilterValue::Config(channels) = &mut invalid.arguments[1].value else { panic!() };
        let FilterValue::Curve { free, .. } = &mut channels[0].value else { panic!() };
        *free = false;
        assert!(CurvesNode::new(&invalid, 22).is_err());
        assert_eq!(original, node());
    }
    #[test]
    #[ignore = "24MP release FREE256 kernel timing"]
    fn bench_free256_24mp() {
        use std::{hint::black_box, time::Instant};
        let curves = CurvesNode::new(&node(), 22).unwrap();
        let start = Instant::now();
        for _ in 0..24_000_000 {
            black_box(curves.apply(black_box([0.23, 0.49, 0.77, 0.63]), &mut |_| Ok(())).unwrap());
        }
        println!("24MP checked prepared FREE256 five-table kernel {:?}", start.elapsed());
    }
}
