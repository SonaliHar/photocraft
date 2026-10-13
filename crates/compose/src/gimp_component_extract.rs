//! Exact public ParamChoice and Bool arguments; ten choices remain closed.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::{
    point::PointOperation,
    registry::{PointMember, component_extract_member},
};
use photocraft_doc::xcf_filters::{FilterNode, FilterValue};

pub(super) fn qualified_epoch(epoch: &str) -> bool {
    matches!(
        epoch,
        super::COMPONENT_EXTRACT_PRECISION_EPOCH
            | super::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH
            | super::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH
            | super::COMPONENT_EXTRACT_GEOMETRY_EPOCH
    )
}

pub(super) fn operation(node: &FilterNode) -> Result<(PointMember, PointOperation)> {
    if node.arguments.len() != 3 {
        return Err(CompositionError::Unsupported("Component Extract argument count"));
    }
    let get = |name: &str| {
        let mut found = node.arguments.iter().filter(|argument| argument.name == name);
        let value = found.next().ok_or(CompositionError::Unsupported("Component Extract missing argument"))?;
        if found.next().is_some() {
            return Err(CompositionError::Unsupported("Component Extract duplicate argument"));
        }
        Ok(&value.value)
    };
    let FilterValue::Enum { domain, value } = get("component")? else {
        return Err(CompositionError::Unsupported("Component Extract choice type"));
    };
    if domain != "gegl:component-extract/component" {
        return Err(CompositionError::Unsupported("Component Extract choice domain"));
    }
    let component = u8::try_from(*value).map_err(|_| CompositionError::Unsupported("Component Extract choice"))?;
    let valid_choice = match node.operation.schema_epoch.as_str() {
        super::COMPONENT_EXTRACT_PRECISION_EPOCH => matches!(component, 0 | 1 | 2 | 20),
        super::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH => (8..=11).contains(&component),
        super::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH => (12..=14).contains(&component),
        super::COMPONENT_EXTRACT_GEOMETRY_EPOCH => matches!(component, 0 | 1 | 2 | 8..=14 | 20),
        _ => false,
    };
    if !valid_choice {
        return Err(CompositionError::Unsupported("Component Extract choice/epoch mismatch"));
    }
    if node.operation.schema_epoch != super::COMPONENT_EXTRACT_PRECISION_EPOCH
        && node.composition.as_ref().is_none_or(|policy| {
            (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator)
                != if node.operation.schema_epoch == super::COMPONENT_EXTRACT_GEOMETRY_EPOCH {
                    (87327313, 87327313, 87327313)
                } else {
                    (-305954223, -305954223, -305954223)
                }
        })
    {
        return Err(CompositionError::Unsupported("CMYK/YCbCr Component Extract stored field envelope"));
    }
    let boolean = |name| -> Result<bool> {
        let FilterValue::Bool(value) = get(name)? else {
            return Err(CompositionError::Unsupported("Component Extract boolean type"));
        };
        Ok(*value)
    };
    Ok(component_extract_member(Some(&node.operation.stored_version), component, boolean("invert")?, boolean("linear")?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_cms::Builtin;
    use photocraft_doc::{
        SampleEncoding,
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gegl:component-extract".into(),
                stored_version: "0:0".into(),
                schema_epoch: super::super::COMPONENT_EXTRACT_PRECISION_EPOCH.into(),
            },
            arguments: vec![
                FilterArgument { name: "component".into(), value: FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: 0 } },
                FilterArgument { name: "invert".into(), value: FilterValue::Bool(false) },
                FilterArgument { name: "linear".into(), value: FilterValue::Bool(false) },
            ],
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: super::super::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -466517423,
                stored_composite_space: -466517423,
                stored_operator: -466517423,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    fn measured_typed_component_is_opaque_and_disabled_preserves_original() {
        let node = node();
        let stack = super::super::PointStack::from_nodes(std::slice::from_ref(&node), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        let input = [0.25, 0.5, 0.75, 0.125];
        assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap(), [0.25, 0.25, 0.25, 1.]);
        assert!(stack.changes_alpha());
        let mut disabled = node;
        disabled.enabled = false;
        let stack = super::super::PointStack::from_nodes(&[disabled], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
        assert_eq!(stack.apply(input, &mut |_| Ok(())).unwrap(), input);
        assert!(!stack.changes_alpha());
    }

    #[test]
    fn opaque_point_extent_preserves_outside_source_bits() {
        let node = node();
        let extent = photocraft_geom::Rect::from_xywh(-7, 5, 19, 13);
        let source = photocraft_doc::xcf_filters::FilterStack {
            owner: photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
            nodes: vec![node.clone()],
            revision: 0,
        };
        let plan =
            crate::gimp_neighborhood::OrderedStack::new(&source, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22, extent, [false; 3]).unwrap();
        let hidden = [0.75, 0.25, -0.0, -0.0];
        for position in [(-8, 5), (12, 5), (0, 4), (0, 18)] {
            assert_eq!(plan.point_at(hidden, position, &mut |_| Ok(())).unwrap().map(f64::to_bits), hidden.map(f64::to_bits));
        }
        assert_eq!(plan.point_at([0.25, 0.5, 0.75, 0.125], (-7, 5), &mut |_| Ok(())).unwrap()[3], 1.);
        let mut disabled = source;
        disabled.nodes[0].enabled = false;
        let plan =
            crate::gimp_neighborhood::OrderedStack::new(&disabled, Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22, extent, [false; 3]).unwrap();
        assert_eq!(plan.point_at(hidden, (-7, 5), &mut |_| Ok(())).unwrap().map(f64::to_bits), hidden.map(f64::to_bits));
    }

    #[test]
    fn independent_subtractive_epochs_consume_typed_opaque_controls() {
        for (component, epoch, expected) in
            [(8, "xcf-22-stock-component-extract-cmyk-precision-v1", 0.), (12, "xcf-22-stock-component-extract-ycbcr-precision-v1", 0.299)]
        {
            let mut node = node();
            node.operation.schema_epoch = epoch.into();
            node.arguments[0].value = FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: component };
            let policy = node.composition.as_mut().unwrap();
            policy.stored_blend_space = -305954223;
            policy.stored_composite_space = -305954223;
            policy.stored_operator = -305954223;
            let stack =
                super::super::PointStack::from_nodes(std::slice::from_ref(&node), Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).unwrap();
            let output = stack.apply([1., 0., 0., 0.125], &mut |_| Ok(())).unwrap();
            assert_eq!(output[3], 1.);
            for value in &output[..3] {
                assert!((*value - expected).abs() <= 1e-6, "component {component}: {output:?}");
            }
        }
    }

    #[test]
    fn component_choices_bools_versions_and_envelopes_are_closed() {
        let original = node();
        operation(&original).unwrap();
        // A qualified kernel choice does not authorize another source epoch.
        for component in 8..=14 {
            let mut changed = original.clone();
            changed.arguments[0].value = FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: component };
            assert!(operation(&changed).is_err());
        }
        for (epoch, choices) in [
            (super::super::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH, vec![0, 12, 15, 20]),
            (super::super::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH, vec![0, 8, 15, 20]),
        ] {
            for component in choices {
                let mut changed = original.clone();
                changed.operation.schema_epoch = epoch.into();
                changed.arguments[0].value = FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: component };
                assert!(operation(&changed).is_err());
            }
        }
        for mutation in 0..10 {
            let mut changed = original.clone();
            match mutation {
                0 => changed.operation.stored_version = "0:1".into(),
                1 => changed.arguments[0].value = FilterValue::Enum { domain: "other".into(), value: 0 },
                2 => changed.arguments[0].value = FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: 3 },
                3 => changed.arguments[1].value = FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Signed { width: 32, value: 0 }),
                4 => changed.arguments[2].value = FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(0)),
                5 => changed.arguments[2].name = "invert".into(),
                6 => changed.arguments[0].name = "unknown".into(),
                7 => {
                    changed.arguments.pop();
                }
                8 => changed.arguments.push(changed.arguments[0].clone()),
                _ => changed.arguments[0].value = FilterValue::Bool(false),
            }
            assert!(operation(&changed).is_err(), "{mutation}");
        }
        for mutation in 0..5 {
            let mut changed = original.clone();
            match mutation {
                0 => changed.opacity_f64_bits = 0.5f64.to_bits(),
                1 => changed.source_region = Some(1),
                2 => changed.source_clipping = None,
                3 => changed.operation.schema_epoch = "other".into(),
                _ => changed.composition.as_mut().unwrap().source_mode = 28,
            }
            assert!(super::super::PointStack::from_nodes(&[changed], Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, 22).is_err());
        }
    }

    #[test]
    #[ignore = "local unpublished 720 independently authored Component Extract native and disabled controls"]
    fn original_component_native_and_opaque_alpha_controls() {
        let bytes =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/component-extract-all-native-controls.json"))
                .unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "d414dcd7d7d06f5af0dc0672eec98ea2bf9e36dac5a124ed97ab1d7d2bd0c8da"
        );
        let fixture: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(fixture["records"].as_array().unwrap().len(), 720);
        let mut maximum = 0f64;
        for record in fixture["records"].as_array().unwrap() {
            let mut node = node();
            node.arguments[0].value =
                FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: record["component"].as_i64().unwrap() as i32 };
            node.arguments[1].value = FilterValue::Bool(record["invert"].as_bool().unwrap());
            node.arguments[2].value = FilterValue::Bool(record["linear"].as_bool().unwrap());
            node.enabled = record["enabled"].as_bool().unwrap();
            let before = node.clone();
            let gray = record["model"] == "GRAY";
            let precision = record["precision"].as_str().unwrap();
            let encoding = if precision.ends_with("NON_LINEAR") {
                SampleEncoding::NonlinearProfile
            } else if precision.ends_with("PERCEPTUAL") {
                SampleEncoding::Perceptual
            } else {
                SampleEncoding::Linear
            };
            let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
            let stack = super::super::PointStack::from_nodes(std::slice::from_ref(&node), profile, encoding, 22).unwrap();
            for (input, expected) in record["source_f64_bits"].as_array().unwrap().iter().zip(record["baked_f64_bits"].as_array().unwrap()) {
                let input = if gray {
                    let color = f64::from_bits(input[0].as_u64().unwrap());
                    [color, color, color, f64::from_bits(input[1].as_u64().unwrap())]
                } else {
                    std::array::from_fn(|i| f64::from_bits(input[i].as_u64().unwrap()))
                };
                let actual = stack.apply_current(input, false, &mut |_| Ok(())).unwrap();
                let quantize = |value: f64| -> f64 {
                    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
                    if precision.starts_with("HALF_") {
                        return f64::from(f32::from_bits(widen_f16_to_f32_bits(try_round_f32_to_f16_bits(value as f32).unwrap())));
                    }
                    let sample = if precision.starts_with("U8_") {
                        SampleType::U8
                    } else if precision.starts_with("U16_") {
                        SampleType::U16
                    } else if precision.starts_with("U32_") {
                        SampleType::U32
                    } else if precision.starts_with("DOUBLE_") {
                        SampleType::F64
                    } else {
                        SampleType::F32
                    };
                    let mut bytes = [0u8; 8];
                    try_write_sample_f64(&mut bytes, sample, 0, value).unwrap();
                    try_read_sample_f64(&bytes, sample, 0).unwrap()
                };
                let actual = actual.map(quantize);
                let expected_alpha = f64::from_bits(expected[if gray { 1 } else { 3 }].as_u64().unwrap());
                assert_eq!(actual[3], expected_alpha, "{} alpha", record["id"]);
                for (index, actual) in actual.iter().take(if gray { 1 } else { 3 }).enumerate() {
                    let expected = f64::from_bits(expected[index].as_u64().unwrap());
                    let delta = (actual * expected_alpha - expected * expected_alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} {} /255", record["id"], delta * 255.);
                }
                if !node.enabled {
                    assert_eq!(actual.map(f64::to_bits), input.map(f64::to_bits));
                }
            }
            assert_eq!(node, before);
        }
        println!("720 Component native/disabled controls max {}/255", maximum * 255.);
    }
    #[test]
    #[ignore = "local unpublished1260 independent CMYK/YCbCr native and disabled controls"]
    fn original_subtractive_native_and_opaque_alpha_controls() {
        let bytes =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/component-subtractive-all-native-controls.json"))
                .unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "7a35e582e6339970d03d9a7d9e10e6bf60e06509c6cb9af3dee41e6141f07bd3"
        );
        let fixture: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(fixture["records"].as_array().unwrap().len(), 1260);
        let mut maximum = 0f64;
        for record in fixture["records"].as_array().unwrap() {
            let mut node = node();
            node.operation.schema_epoch = if record["family"] == "cmyk" {
                super::super::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH
            } else {
                super::super::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH
            }
            .into();
            let policy = node.composition.as_mut().unwrap();
            policy.stored_blend_space = -305954223;
            policy.stored_composite_space = -305954223;
            policy.stored_operator = -305954223;
            node.arguments[0].value =
                FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: record["component"].as_i64().unwrap() as i32 };
            node.arguments[1].value = FilterValue::Bool(record["invert"].as_bool().unwrap());
            node.arguments[2].value = FilterValue::Bool(record["linear"].as_bool().unwrap());
            node.enabled = record["enabled"].as_bool().unwrap();
            let before = node.clone();
            let gray = record["model"] == "GRAY";
            let precision = record["precision"].as_str().unwrap();
            let encoding = if precision.ends_with("NON_LINEAR") {
                SampleEncoding::NonlinearProfile
            } else if precision.ends_with("PERCEPTUAL") {
                SampleEncoding::Perceptual
            } else {
                SampleEncoding::Linear
            };
            let profile = if gray { Builtin::SGray.profile() } else { Builtin::Srgb.profile() };
            let stack = super::super::PointStack::from_nodes(std::slice::from_ref(&node), profile, encoding, 22).unwrap();
            for (input, expected) in record["source_f64_bits"].as_array().unwrap().iter().zip(record["baked_f64_bits"].as_array().unwrap()) {
                let input = if gray {
                    let color = f64::from_bits(input[0].as_u64().unwrap());
                    [color, color, color, f64::from_bits(input[1].as_u64().unwrap())]
                } else {
                    std::array::from_fn(|i| f64::from_bits(input[i].as_u64().unwrap()))
                };
                let actual = stack.apply_current(input, false, &mut |_| Ok(())).unwrap();
                let quantize = |value: f64| -> f64 {
                    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
                    if precision.starts_with("HALF_") {
                        return f64::from(f32::from_bits(widen_f16_to_f32_bits(try_round_f32_to_f16_bits(value as f32).unwrap())));
                    }
                    let sample = if precision.starts_with("U8_") {
                        SampleType::U8
                    } else if precision.starts_with("U16_") {
                        SampleType::U16
                    } else if precision.starts_with("U32_") {
                        SampleType::U32
                    } else if precision.starts_with("DOUBLE_") {
                        SampleType::F64
                    } else {
                        SampleType::F32
                    };
                    let mut bytes = [0u8; 8];
                    try_write_sample_f64(&mut bytes, sample, 0, value).unwrap();
                    try_read_sample_f64(&bytes, sample, 0).unwrap()
                };
                let actual = actual.map(quantize);
                let expected_alpha = f64::from_bits(expected[if gray { 1 } else { 3 }].as_u64().unwrap());
                assert_eq!(actual[3], expected_alpha, "{} alpha", record["id"]);
                for (index, actual) in actual.iter().take(if gray { 1 } else { 3 }).enumerate() {
                    let expected = f64::from_bits(expected[index].as_u64().unwrap());
                    let delta = (actual * expected_alpha - expected * expected_alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} {} /255", record["id"], delta * 255.);
                }
                if !node.enabled {
                    assert_eq!(actual.map(f64::to_bits), input.map(f64::to_bits));
                }
            }
            assert_eq!(node, before);
        }
        println!("1260 CMYK/YCbCr Component native/disabled controls max {}/255", maximum * 255.);
    }
}
