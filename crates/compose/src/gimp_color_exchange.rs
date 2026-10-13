//! Typed literal colors retain all original component bits; no resource lookup.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::{
    point::PointOperation,
    registry::{PointMember, color_exchange_member},
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};

pub(super) fn operation(node: &FilterNode) -> Result<(PointMember, PointOperation)> {
    if node.arguments.len() != 5 {
        return Err(CompositionError::Unsupported("Color Exchange argument count"));
    }
    let get = |name: &str| {
        let mut found = node.arguments.iter().filter(|arg| arg.name == name);
        let value = found.next().ok_or(CompositionError::Unsupported("Color Exchange missing argument"))?;
        if found.next().is_some() {
            return Err(CompositionError::Unsupported("Color Exchange duplicate argument"));
        }
        Ok(&value.value)
    };
    let color = |name| -> Result<[f32; 4]> {
        let FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None } = get(name)? else {
            return Err(CompositionError::Unsupported("Color Exchange literal color encoding/profile"));
        };
        if components.len() != 4 {
            return Err(CompositionError::Unsupported("Color Exchange literal component count"));
        }
        let mut output = [0.; 4];
        for (target, component) in output.iter_mut().zip(components) {
            let ExactNumber::Float32Bits(bits) = component else {
                return Err(CompositionError::Unsupported("Color Exchange literal component type"));
            };
            *target = f32::from_bits(*bits);
        }
        Ok(output)
    };
    let mut thresholds = [0.; 3];
    for (target, name) in thresholds.iter_mut().zip(["red-threshold", "green-threshold", "blue-threshold"]) {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = get(name)? else {
            return Err(CompositionError::Unsupported("Color Exchange threshold type"));
        };
        *target = f32::from_bits(*bits);
    }
    Ok(color_exchange_member(Some(&node.operation.stored_version), color("from-color")?, color("to-color")?, thresholds)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, OperationIdentity},
    };
    fn node(from: [u32; 4], to: [u32; 4], thresholds: [u32; 3]) -> FilterNode {
        let color = |name: &str, bits: [u32; 4]| FilterArgument {
            name: name.into(),
            value: FilterValue::Color { components: bits.map(ExactNumber::Float32Bits).to_vec(), encoding: SampleEncoding::NonlinearProfile, profile: None },
        };
        let mut arguments = vec![color("from-color", from), color("to-color", to)];
        arguments.extend(
            ["red-threshold", "green-threshold", "blue-threshold"]
                .into_iter()
                .zip(thresholds)
                .map(|(name, bits)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(bits)) }),
        );
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gegl:color-exchange".into(),
                stored_version: "0:0".into(),
                schema_epoch: super::super::COLOR_EXCHANGE_PRECISION_EPOCH.into(),
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
                stored_blend_space: 798851665,
                stored_composite_space: 798851665,
                stored_operator: 798851665,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    #[ignore = "local unpublished 144 original native Color Exchange controls"]
    fn typed_point_stack_matches_all_original_native_and_preserves_alpha() {
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_COLOR_EXCHANGE_CONTROLS").unwrap())
                    .parent()
                    .unwrap()
                    .join("color-exchange-all-native-controls.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(fixture["records"].as_array().unwrap().len(), 144);
        let mut maximum = 0f64;
        for record in fixture["records"].as_array().unwrap() {
            let from = std::array::from_fn(|i| record["from_bits"][i].as_u64().unwrap() as u32);
            let to = std::array::from_fn(|i| record["to_bits"][i].as_u64().unwrap() as u32);
            let thresholds = std::array::from_fn(|i| (record["thresholds"][i].as_f64().unwrap() as f32).to_bits());
            let mut node = node(from, to, thresholds);
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
                assert_eq!(actual[3].to_bits(), input[3].to_bits());
                for (index, actual) in actual.iter().take(if gray { 1 } else { 3 }).enumerate() {
                    let expected = f64::from_bits(expected[index].as_u64().unwrap());
                    let delta = (actual - expected).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} delta {delta}", record["id"]);
                }
                if !node.enabled {
                    assert_eq!(actual.map(f64::to_bits), input.map(f64::to_bits));
                }
            }
            assert_eq!(node, before);
        }
        println!("144 native Color Exchange maximum {maximum} ({} /255)", maximum * 255.);
    }
    #[test]
    fn typed_literal_rejects_profile_encoding_components_duplicates_and_bad_bits() {
        let original = node(
            [0.375f32.to_bits(), 0.5f32.to_bits(), 0.625f32.to_bits(), 0.75f32.to_bits()],
            [0.125f32.to_bits(), 0.75f32.to_bits(), 0.25f32.to_bits(), 0.25f32.to_bits()],
            [0.125f32.to_bits(), 0.1875f32.to_bits(), 0.25f32.to_bits()],
        );
        for mutation in 0..8 {
            let mut changed = original.clone();
            match mutation {
                0 => changed.arguments[0].name = "unknown".into(),
                1 => changed.arguments[1].name = "from-color".into(),
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
                6 => changed.arguments[2].value = FilterValue::Number(ExactNumber::Float64Bits(0.125f64.to_bits())),
                _ => changed.operation.stored_version = "1:0".into(),
            }
            assert!(operation(&changed).is_err(), "{mutation}");
            assert_eq!(original.arguments.len(), 5);
        }
    }
}
