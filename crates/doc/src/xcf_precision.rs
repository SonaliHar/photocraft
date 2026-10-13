//! Mutable precision of generated filter buffers, never original source samples.
use crate::xcf_filters::{ExactNumber, FilterValue};
use crate::{Document, SampleType, XcfState, XcfValidationError, item_identity::ItemRef};
use serde::{Deserialize, Serialize};

fn component_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    node.operation.name == "gegl:component-extract"
        && node.operation.stored_version == "0:0"
        && matches!(
            node.operation.schema_epoch.as_str(),
            "xcf-22-stock-component-extract-four-precision-v1"
                | "xcf-22-stock-component-extract-cmyk-precision-v1"
                | "xcf-22-stock-component-extract-ycbcr-precision-v1"
        )
        && (node.operation.schema_epoch == "xcf-22-stock-component-extract-four-precision-v1"
            || node.composition.as_ref().is_some_and(|policy| {
                (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) == (-305954223, -305954223, -305954223)
            }))
        && node.arguments.len() == 3
        && ["component", "invert", "linear"].iter().all(|name| {
            let mut values = node.arguments.iter().filter(|argument| argument.name == *name);
            let Some(argument) = values.next() else {
                return false;
            };
            if values.next().is_some() {
                return false;
            }
            match (&argument.value, *name) {
                (FilterValue::Enum { domain, value }, "component") => {
                    domain == "gegl:component-extract/component"
                        && match node.operation.schema_epoch.as_str() {
                            "xcf-22-stock-component-extract-four-precision-v1" => matches!(value, 0 | 1 | 2 | 20),
                            "xcf-22-stock-component-extract-cmyk-precision-v1" => (8..=11).contains(value),
                            "xcf-22-stock-component-extract-ycbcr-precision-v1" => (12..=14).contains(value),
                            _ => false,
                        }
                }
                (FilterValue::Bool(_), "invert" | "linear") => true,
                _ => false,
            }
        })
}

fn alpha_clip_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    if node.operation.name != "gegl:alpha-clip"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf22-stock-alpha-clip-precision-v1"
        || node.arguments.len() != 4
    {
        return false;
    }
    let mut low = None;
    let mut high = None;
    for name in ["clip-low", "low-limit", "clip-high", "high-limit"] {
        let mut values = node.arguments.iter().filter(|a| a.name == name);
        let Some(a) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        match (&a.value, name) {
            (FilterValue::Bool(_), "clip-low" | "clip-high") => {}
            (FilterValue::Number(ExactNumber::Float32Bits(bits)), "low-limit" | "high-limit") => {
                let value = f32::from_bits(*bits);
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return false;
                }
                if name == "low-limit" {
                    low = Some(value);
                } else {
                    high = Some(value);
                }
            }
            _ => return false,
        }
    }
    matches!((low,high),(Some(l),Some(h)) if l<=h)
}

fn mono_mixer_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    if node.operation.name != "gegl:mono-mixer"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf-22-stock-mono-mixer-signed-precision-v1"
        || node.arguments.len() != 4
        || node.composition.as_ref().is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (1799848529, 1799848529, 1799848529))
    {
        return false;
    }
    let mut weights = [0f32; 3];
    let mut preserve = false;
    for (i, name) in ["red", "green", "blue", "preserve-luminosity"].iter().enumerate() {
        let mut values = node.arguments.iter().filter(|a| a.name == *name);
        let Some(a) = values.next() else { return false };
        if values.next().is_some() {
            return false;
        }
        match (&a.value, *name) {
            (FilterValue::Bool(v), "preserve-luminosity") => preserve = *v,
            (FilterValue::Number(ExactNumber::Float32Bits(bits)), "red" | "green" | "blue") => {
                let value = f32::from_bits(*bits);
                if !value.is_finite() || !(-5.0..=5.0).contains(&value) {
                    return false;
                }
                let Some(target) = weights.get_mut(i) else { return false };
                *target = value;
            }
            (FilterValue::Number(ExactNumber::Float64Bits(bits)), "red" | "green" | "blue") => {
                let value = f64::from_bits(*bits);
                if !value.is_finite() || !(-5.0..=5.0).contains(&value) {
                    return false;
                }
                let Some(target) = weights.get_mut(i) else { return false };
                *target = value as f32;
            }
            _ => return false,
        }
    }
    let sum = (weights[0] + weights[1]) + weights[2];
    !node.enabled || !preserve || sum == 0.0 || (1.0f32 / sum.abs()).is_finite()
}

fn channel_mixer_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    if node.operation.name != "gegl:channel-mixer"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf-22-stock-channel-mixer-precision-v1"
        || node.arguments.len() != 10
        || node
            .composition
            .as_ref()
            .is_none_or(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) != (-1150320047, -1150320047, -1150320047))
    {
        return false;
    }
    let names = [["rr-gain", "rg-gain", "rb-gain"], ["gr-gain", "gg-gain", "gb-gain"], ["br-gain", "bg-gain", "bb-gain"]];
    let mut weights = [[0f64; 3]; 3];
    for (row, row_names) in weights.iter_mut().zip(names) {
        for (value, name) in row.iter_mut().zip(row_names) {
            let mut matches = node.arguments.iter().filter(|a| a.name == name);
            let Some(arg) = matches.next() else {
                return false;
            };
            if matches.next().is_some() {
                return false;
            }
            *value = match arg.value {
                FilterValue::Number(ExactNumber::Float32Bits(bits)) => f64::from(f32::from_bits(bits)),
                FilterValue::Number(ExactNumber::Float64Bits(bits)) => f64::from_bits(bits),
                _ => return false,
            };
            if !value.is_finite() || !(-2.0..=2.0).contains(value) {
                return false;
            }
        }
    }
    let mut matches = node.arguments.iter().filter(|a| a.name == "preserve-luminosity");
    let Some(arg) = matches.next() else {
        return false;
    };
    if matches.next().is_some() {
        return false;
    }
    let FilterValue::Bool(preserve) = arg.value else {
        return false;
    };
    !node.enabled
        || !preserve
        || weights.iter().all(|row| {
            let sum = (row[0] + row[1]) + row[2];
            sum == 0.0 || (1.0 / sum.abs()).is_finite()
        })
}

fn absolute_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    node.operation.name == "gegl:absolute"
        && node.operation.stored_version == "0:0"
        && node.operation.schema_epoch == "xcf-22-stock-absolute-linear-precision-v1"
        && node.arguments.is_empty()
        && node
            .composition
            .as_ref()
            .is_some_and(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) == (1574142545, 1574142545, 1574142545))
}

fn red_eye_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    node.operation.name == "gegl:red-eye-removal"
        && node.operation.stored_version == "0:0"
        && node.operation.schema_epoch == "xcf-22-stock-red-eye-removal-precision-v1"
        && node.composition.as_ref().is_some_and(|p| (p.stored_blend_space, p.stored_composite_space, p.stored_operator) == (127500881, 127500881, 127500881))
        && matches!(node.arguments.as_slice(), [argument] if argument.name=="threshold" && matches!(&argument.value,FilterValue::Number(ExactNumber::Float32Bits(bits)) if (0.0..=0.8).contains(&f64::from(f32::from_bits(*bits)))))
}

fn color_overlay_half_arguments(node: &crate::xcf_filters::FilterNode) -> bool {
    if node.operation.name != "gegl:color-overlay"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf-22-stock-color-overlay-six-precision-v1"
        || node.arguments.len() != 2
        || node
            .composition
            .as_ref()
            .is_none_or(|policy| (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (1435337297, 1435337297, 1435337297))
    {
        return false;
    }
    ["value", "srgb"].iter().all(|name| {
        let mut arguments = node.arguments.iter().filter(|a| a.name == *name);
        let Some(argument) = arguments.next() else {
            return false;
        };
        if arguments.next().is_some() {
            return false;
        }
        match (&argument.value, *name) {
            (FilterValue::Color { components, encoding: crate::SampleEncoding::NonlinearProfile, profile: None }, "value") => {
                components.len() == 4 && components.iter().all(|v| matches!(v, ExactNumber::Float32Bits(bits) if (0.0..=1.0).contains(&f32::from_bits(*bits))))
            }
            (FilterValue::Bool(_), "srgb") => true,
            _ => false,
        }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOutputPrecision {
    #[default]
    Canonical,
    Half,
}
impl FilterOutputPrecision {
    pub fn is_canonical(&self) -> bool {
        *self == Self::Canonical
    }
    pub fn validate(self, state: &XcfState, doc: &Document) -> Result<(), XcfValidationError> {
        if self == Self::Canonical {
            return Ok(());
        }
        let valid = matches!(doc.mode, crate::ColorMode::Rgb | crate::ColorMode::Grayscale)
            && doc.depth == SampleType::F32
            && !state.filter_stacks.is_empty()
            && state.filter_stacks.iter().all(|stack| {
                matches!(stack.owner, ItemRef::Layer(_))
                    && stack.nodes.len() == 1
                    && stack.nodes.first().is_some_and(|node| {
                        ((node.operation.name == "gegl:color-to-alpha"
                            && node.operation.stored_version == "0:0"
                            && node.operation.schema_epoch == "xcf-22-stock-color-to-alpha-precision-v1"
                            && node.arguments.len() == 3
                            && ["color", "transparency-threshold", "opacity-threshold"].iter().all(|name| {
                                let mut matches = node.arguments.iter().filter(|argument| argument.name == *name);
                                let Some(argument) = matches.next() else {
                                    return false;
                                };
                                if matches.next().is_some() {
                                    return false;
                                }
                                match (&argument.value, *name) {
                                    (FilterValue::Color { components, encoding, profile }, "color") => {
                                        components.len() == 4
                                            && *encoding == crate::SampleEncoding::NonlinearProfile
                                            && profile.is_none()
                                            && components
                                                .iter()
                                                .all(|value| matches!(value, ExactNumber::Float32Bits(bits) if (0.0..=1.0).contains(&f32::from_bits(*bits))))
                                    }
                                    (FilterValue::Number(ExactNumber::Float32Bits(bits)), _) => (0.0..=1.0).contains(&f32::from_bits(*bits)),
                                    _ => false,
                                }
                            }))
                            || component_half_arguments(node)
                            || alpha_clip_half_arguments(node)
                            || color_overlay_half_arguments(node)
                            || channel_mixer_half_arguments(node)
                            || mono_mixer_half_arguments(node)
                            || red_eye_half_arguments(node)
                            || doc.sample_encoding == Some(crate::SampleEncoding::Linear) && absolute_half_arguments(node))
                            && node.opacity_f64_bits == 1.0f64.to_bits()
                            && node.source_clipping == Some(0)
                            && node.source_region == Some(0)
                            && node.composition.as_ref().is_some_and(|policy| {
                                policy.source_mode == 62
                                    && !policy.legacy
                                    && !policy.pass_through
                                    && policy.compatibility_epoch == "xcf-22-stock-point-replace-fields-v1"
                            })
                    })
            });
        if valid {
            Ok(())
        } else {
            Err(XcfValidationError::new("filter_output_precision", "Half output requires current F32 singleton qualified generated filter buffers"))
        }
    }
}
impl XcfState {
    /// Removal clears the mutable policy only after its last qualified owner disappears.
    pub fn reconcile_filter_output_precision(&mut self) {
        if self.filter_stacks.iter().all(|stack| stack.nodes.is_empty()) {
            self.current_filter_output_precision = FilterOutputPrecision::Canonical;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcf_filters::{FilterArgument, FilterNode, FilterStack, OperationIdentity};
    fn node() -> FilterNode {
        FilterNode {
            id: crate::item_identity::FilterId(1),
            operation: OperationIdentity {
                name: "gegl:color-overlay".into(),
                stored_version: "0:0".into(),
                schema_epoch: "xcf-22-stock-color-overlay-six-precision-v1".into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: vec![
                FilterArgument {
                    name: "value".into(),
                    value: FilterValue::Color {
                        components: [0.25f32, 0.5, 0.75, 0.25].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                        encoding: crate::SampleEncoding::NonlinearProfile,
                        profile: None,
                    },
                },
                FilterArgument { name: "srgb".into(), value: FilterValue::Bool(false) },
            ],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(crate::import_compositing::GimpComposition {
                compatibility_epoch: "xcf-22-stock-point-replace-fields-v1".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: 1435337297,
                stored_composite_space: 1435337297,
                stored_operator: 1435337297,
                resolved: None,
                pass_through: false,
            }),
        }
    }
    #[test]
    fn channel_mixer_half_requires_exact_typed_mutable_double_authority() {
        let mut node = node();
        node.operation.name = "gegl:channel-mixer".into();
        node.operation.schema_epoch = "xcf-22-stock-channel-mixer-precision-v1".into();
        let policy = node.composition.as_mut().unwrap();
        policy.stored_blend_space = -1150320047;
        policy.stored_composite_space = -1150320047;
        policy.stored_operator = -1150320047;
        node.arguments = ["rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| FilterArgument {
                name: name.into(),
                value: FilterValue::Number(ExactNumber::Float32Bits(if i % 4 == 0 { 1.0f32.to_bits() } else { 0.0f32.to_bits() })),
            })
            .collect();
        node.arguments.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) });
        assert!(channel_mixer_half_arguments(&node));
        node.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(1.23456789_f64.to_bits()));
        assert!(channel_mixer_half_arguments(&node));
        let state = XcfState {
            filter_stacks: vec![FilterStack { owner: ItemRef::Layer(crate::LayerId(1)), nodes: vec![node.clone()], revision: 0 }],
            ..Default::default()
        };
        let doc = Document::new("channel-mixer", photocraft_geom::Size::new(8, 6), crate::ColorMode::Rgb, SampleType::F32);
        assert!(FilterOutputPrecision::Half.validate(&state, &doc).is_ok());
        for mutation in 0..8 {
            let mut bad = node.clone();
            match mutation {
                0 => bad.operation.stored_version = "1:0".into(),
                1 => bad.operation.schema_epoch = "unknown".into(),
                2 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(f64::NAN.to_bits())),
                3 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(2.01_f64.to_bits())),
                4 => bad.arguments[0].value = FilterValue::Bool(false),
                5 => bad.composition.as_mut().unwrap().stored_operator = 0,
                6 => bad.arguments[0].name = bad.arguments[1].name.clone(),
                _ => {
                    bad.arguments.pop();
                }
            }
            assert!(!channel_mixer_half_arguments(&bad), "{mutation}");
        }
    }
    #[test]
    fn red_eye_half_requires_exact_current_schema_and_preserves_threshold_bits() {
        let mut node = node();
        node.operation.name = "gegl:red-eye-removal".into();
        node.operation.schema_epoch = "xcf-22-stock-red-eye-removal-precision-v1".into();
        let policy = node.composition.as_mut().unwrap();
        policy.stored_blend_space = 127500881;
        policy.stored_composite_space = 127500881;
        policy.stored_operator = 127500881;
        node.arguments = vec![FilterArgument { name: "threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.625f32.to_bits())) }];
        assert!(red_eye_half_arguments(&node));
        let state = XcfState {
            filter_stacks: vec![FilterStack { owner: ItemRef::Layer(crate::LayerId(1)), nodes: vec![node.clone()], revision: 0 }],
            ..Default::default()
        };
        let doc = Document::new("red-eye", photocraft_geom::Size::new(8, 6), crate::ColorMode::Rgb, SampleType::F32);
        assert!(FilterOutputPrecision::Half.validate(&state, &doc).is_ok());
        for mutation in 0..8 {
            let mut bad = node.clone();
            match mutation {
                0 => bad.operation.stored_version = "1:0".into(),
                1 => bad.operation.schema_epoch = "old".into(),
                2 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits())),
                3 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
                4 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.625f64.to_bits())),
                5 => bad.composition.as_mut().unwrap().stored_operator = 0,
                6 => bad.arguments[0].name = "unknown".into(),
                _ => bad.arguments.push(bad.arguments[0].clone()),
            }
            assert!(!red_eye_half_arguments(&bad), "{mutation}");
        }
        let mut altered = state;
        altered.filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits();
        assert!(FilterOutputPrecision::Half.validate(&altered, &doc).is_err());
    }
    #[test]
    fn mono_mixer_half_preserves_closed_typed_signed_configuration() {
        use crate::xcf_filters::FilterArgument;
        let mut node = node();
        node.operation.name = "gegl:mono-mixer".into();
        node.operation.schema_epoch = "xcf-22-stock-mono-mixer-signed-precision-v1".into();
        let policy = node.composition.as_mut().unwrap();
        policy.stored_blend_space = 1799848529;
        policy.stored_composite_space = 1799848529;
        policy.stored_operator = 1799848529;
        node.arguments = [("red", -0.5f32), ("green", 1.5), ("blue", 0.25)]
            .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) })
            .to_vec();
        node.arguments.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) });
        assert!(mono_mixer_half_arguments(&node));
        let mut double = node.clone();
        double.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits((-0.50000000001f64).to_bits()));
        assert!(mono_mixer_half_arguments(&double));
        assert_eq!(double.arguments[0].value, FilterValue::Number(ExactNumber::Float64Bits((-0.50000000001f64).to_bits())));
        for change in 0..8 {
            let mut bad = node.clone();
            match change {
                0 => bad.operation.stored_version = "1:0".into(),
                1 => bad.operation.schema_epoch = "old".into(),
                2 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(f64::NAN.to_bits())),
                3 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
                4 => bad.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(5.01f32.to_bits())),
                5 => bad.arguments[0].name = "green".into(),
                6 => bad.arguments[3].value = FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())),
                _ => bad.composition.as_mut().unwrap().stored_operator += 1,
            };
            assert!(!mono_mixer_half_arguments(&bad));
        }
    }

    #[test]
    fn color_overlay_half_requires_current_typed_shape_and_measured_fields() {
        let original = node();
        assert!(color_overlay_half_arguments(&original));
        for change in 0..11 {
            let mut bad = original.clone();
            match change {
                0 => bad.operation.schema_epoch = "xcf-22-stock-color-to-alpha-precision-v1".into(),
                1 => bad.operation.stored_version = "1:0".into(),
                2 => bad.composition.as_mut().unwrap().stored_blend_space = 0,
                3 => bad.composition = None,
                4 => {
                    let duplicate = bad.arguments[0].clone();
                    bad.arguments[1] = duplicate;
                }
                5 => bad.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
                6 => bad.arguments[0].name = "unknown".into(),
                _ => {
                    let FilterValue::Color { components, encoding, .. } = &mut bad.arguments[0].value else { panic!("own constructor") };
                    match change {
                        7 => components[0] = ExactNumber::Float64Bits(0.25f64.to_bits()),
                        8 => components[3] = ExactNumber::Float32Bits(f32::NAN.to_bits()),
                        9 => components[3] = ExactNumber::Float32Bits(1.1f32.to_bits()),
                        _ => *encoding = crate::SampleEncoding::Linear,
                    }
                }
            }
            assert!(!color_overlay_half_arguments(&bad), "change{change}");
        }
    }
    #[test]
    fn color_overlay_half_is_mutable_policy_independent_of_source_precision() {
        let mut state = XcfState::default();
        state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(crate::LayerId(1)), nodes: vec![node()], revision: 0 });
        let mut doc = Document::new("native", photocraft_geom::Size::new(8, 6), crate::ColorMode::Rgb, SampleType::F32);
        assert!(FilterOutputPrecision::Half.validate(&state, &doc).is_ok());
        state.filter_stacks[0].nodes[0].enabled = false;
        assert!(FilterOutputPrecision::Half.validate(&state, &doc).is_ok());
        doc.depth = SampleType::F64;
        assert!(FilterOutputPrecision::Half.validate(&state, &doc).is_err());
        doc.depth = SampleType::F32;
        state.filter_stacks[0].nodes[0].source_region = Some(1);
        assert!(FilterOutputPrecision::Half.validate(&state, &doc).is_err());
        assert!(FilterOutputPrecision::Canonical.validate(&state, &doc).is_ok());
    }
}
