//! Typed literal colors retain all original component bits; no resource lookup.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::{
    point::PointOperation,
    registry::{PointMember, color_to_alpha_member},
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};

pub(super) fn operation(node: &FilterNode) -> Result<(PointMember, PointOperation)> {
    if node.arguments.len() != 3 {
        return Err(CompositionError::Unsupported("Color to Alpha argument count"));
    }
    let get = |name: &str| {
        let mut found = node.arguments.iter().filter(|arg| arg.name == name);
        let value = found.next().ok_or(CompositionError::Unsupported("Color to Alpha missing argument"))?;
        if found.next().is_some() {
            return Err(CompositionError::Unsupported("Color to Alpha duplicate argument"));
        }
        Ok(&value.value)
    };
    let color = |name| -> Result<[f32; 4]> {
        let FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None } = get(name)? else {
            return Err(CompositionError::Unsupported("Color to Alpha literal color encoding/profile"));
        };
        if components.len() != 4 {
            return Err(CompositionError::Unsupported("Color to Alpha literal component count"));
        }
        let mut output = [0.; 4];
        for (target, component) in output.iter_mut().zip(components) {
            let ExactNumber::Float32Bits(bits) = component else {
                return Err(CompositionError::Unsupported("Color to Alpha literal component type"));
            };
            *target = f32::from_bits(*bits);
        }
        Ok(output)
    };
    let threshold = |name| -> Result<f32> {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = get(name)? else {
            return Err(CompositionError::Unsupported("Color to Alpha threshold type"));
        };
        Ok(f32::from_bits(*bits))
    };
    Ok(color_to_alpha_member(Some(&node.operation.stored_version), color("color")?, threshold("transparency-threshold")?, threshold("opacity-threshold")?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node(color_bits: [u32; 4], thresholds: [u32; 2]) -> FilterNode {
        let color = |name: &str, bits: [u32; 4]| FilterArgument {
            name: name.into(),
            value: FilterValue::Color { components: bits.map(ExactNumber::Float32Bits).to_vec(), encoding: SampleEncoding::NonlinearProfile, profile: None },
        };
        let mut arguments = vec![color("color", color_bits)];
        arguments.extend(
            ["transparency-threshold", "opacity-threshold"]
                .into_iter()
                .zip(thresholds)
                .map(|(name, bits)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(bits)) }),
        );
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gegl:color-to-alpha".into(),
                stored_version: "0:0".into(),
                schema_epoch: super::super::COLOR_TO_ALPHA_PRECISION_EPOCH.into(),
            },
            arguments,
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
                stored_blend_space: 1758364241,
                stored_composite_space: 1758364241,
                stored_operator: 1758364241,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    #[ignore = "local unpublished 144 original native Color to Alpha controls"]
    fn typed_point_stack_matches_all_original_native_and_operating_alpha() {
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-xcf-probes/color-to-alpha-all-native-controls.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(fixture["records"].as_array().unwrap().len(), 144);
        let mut maximum = 0f64;
        for record in fixture["records"].as_array().unwrap() {
            let color = std::array::from_fn(|i| record["color_bits"][i].as_u64().unwrap() as u32);
            let thresholds = std::array::from_fn(|i| record["threshold_bits"][i].as_u64().unwrap() as u32);
            let mut node = node(color, thresholds);
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
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let stack = super::super::PointStack::from_nodes(std::slice::from_ref(&node), profile, encoding, 22).unwrap();
            for (input, expected) in record["source_f64_bits"].as_array().unwrap().iter().zip(record["baked_f64_bits"].as_array().unwrap()) {
                let input = if gray {
                    [
                        f64::from_bits(input[0].as_u64().unwrap()),
                        f64::from_bits(input[0].as_u64().unwrap()),
                        f64::from_bits(input[0].as_u64().unwrap()),
                        f64::from_bits(input[1].as_u64().unwrap()),
                    ]
                } else {
                    std::array::from_fn(|i| f64::from_bits(input[i].as_u64().unwrap()))
                };
                let actual = stack.apply_current(input, false, &mut |_| Ok(())).unwrap();
                let quantize = |value: f64| -> f64 {
                    use photocraft_color::{SampleType, try_read_sample_f64, try_round_f32_to_f16_bits, try_write_sample_f64, widen_f16_to_f32_bits};
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
                    if precision.starts_with("HALF_") {
                        return f64::from(f32::from_bits(widen_f16_to_f32_bits(try_round_f32_to_f16_bits(value as f32).unwrap())));
                    }
                    let mut bytes = [0u8; 8];
                    try_write_sample_f64(&mut bytes, sample, 0, value).unwrap();
                    try_read_sample_f64(&bytes, sample, 0).unwrap()
                };
                let quantized = actual.map(quantize);
                let expected_alpha = f64::from_bits(expected[if gray { 1 } else { 3 }].as_u64().unwrap());
                assert!((quantized[3] - expected_alpha).abs() <= 1. / 65535., "{} alpha {} != {}", record["id"], actual[3], expected_alpha);
                let actual_alpha = quantized[3];
                for (index, actual) in quantized.iter().take(if gray { 1 } else { 3 }).enumerate() {
                    let expected = f64::from_bits(expected[index].as_u64().unwrap());
                    let delta = (actual * actual_alpha - expected * expected_alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} delta {delta}", record["id"]);
                }
                if !node.enabled {
                    assert_eq!(actual.map(f64::to_bits), input.map(f64::to_bits));
                }
            }
            assert_eq!(node, before);
        }
        println!("144 native Color to Alpha maximum {maximum} ({} /255)", maximum * 255.);
    }

    #[test]
    fn exact_literal_types_names_domains_and_versions_are_closed() {
        let original = node([0.375f32.to_bits(), 0.5f32.to_bits(), 0.625f32.to_bits(), 0.75f32.to_bits()], [0.125f32.to_bits(), 0.5f32.to_bits()]);
        operation(&original).unwrap();
        for mutation in 0..11 {
            let mut changed = original.clone();
            match mutation {
                0 => changed.arguments[0].name = "unknown".into(),
                1 => changed.arguments[2].name = "transparency-threshold".into(),
                2 => {
                    if let FilterValue::Color { components, .. } = &mut changed.arguments[0].value {
                        components.pop();
                    }
                }
                3 => {
                    if let FilterValue::Color { encoding, .. } = &mut changed.arguments[0].value {
                        *encoding = SampleEncoding::Linear;
                    }
                }
                4 => {
                    if let FilterValue::Color { profile, .. } = &mut changed.arguments[0].value {
                        *profile = Some(photocraft_doc::item_identity::ResourceId(1));
                    }
                }
                5 => {
                    if let FilterValue::Color { components, .. } = &mut changed.arguments[0].value {
                        components[3] = ExactNumber::Float32Bits(f32::NAN.to_bits());
                    }
                }
                6 => changed.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.125f64.to_bits())),
                7 => changed.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(1.001f32.to_bits())),
                8 => changed.operation.stored_version = "1:0".into(),
                9 => {
                    changed.arguments.pop();
                }
                _ => changed.arguments[2].value = FilterValue::Bool(false),
            }
            assert!(operation(&changed).is_err(), "{mutation}");
        }
    }
    #[test]
    fn operating_alpha_requires_full_singleton_replace_and_preserves_disabled_input() {
        let original = node([0.375f32.to_bits(), 0.5f32.to_bits(), 0.625f32.to_bits(), 0.75f32.to_bits()], [0.125f32.to_bits(), 0.5f32.to_bits()]);
        let profile = photocraft_cms::Builtin::Srgb.profile();
        for mutation in 0..7 {
            let mut changed = original.clone();
            match mutation {
                0 => changed.opacity_f64_bits = 0.5f64.to_bits(),
                1 => changed.source_clipping = None,
                2 => changed.source_region = Some(1),
                3 => changed.composition.as_mut().unwrap().source_mode = 28,
                4 => changed.composition.as_mut().unwrap().compatibility_epoch = "unknown".into(),
                5 => changed.operation.schema_epoch = "unknown".into(),
                _ => changed.auxiliary.push(("aux".into(), photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(5)))),
            }
            assert!(super::super::PointStack::from_nodes(&[changed], profile, SampleEncoding::NonlinearProfile, 22).is_err(), "{mutation}");
        }
        assert!(super::super::PointStack::from_nodes(&[original.clone(), original.clone()], profile, SampleEncoding::NonlinearProfile, 22).is_err());
        let mut disabled = original;
        disabled.enabled = false;
        let before = disabled.clone();
        let stack = super::super::PointStack::from_nodes(std::slice::from_ref(&disabled), profile, SampleEncoding::NonlinearProfile, 22).unwrap();
        let input = [0.25, 0.5, 0.75, 0.375];
        assert_eq!(stack.apply_current(input, false, &mut |_| Ok(())).unwrap().map(f64::to_bits), input.map(f64::to_bits));
        assert_eq!(disabled, before);
        disabled.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()));
        assert!(super::super::PointStack::from_nodes(&[disabled], profile, SampleEncoding::NonlinearProfile, 22).is_err());
    }
}
