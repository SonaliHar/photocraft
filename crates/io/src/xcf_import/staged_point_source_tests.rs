//! UNRUN bounded original route selection. This supplies no source authority.
use super::super::plane::fixture;
use super::*;
#[test]
fn oversized_alien_argument_list_is_rejected_before_new_selector_scan() {
    use photocraft_xcf::{Argument, ArgumentValue, Effect};
    let bytes = fixture::image(22, 650, 0, 8, 6, &0.5f32.to_be_bytes().repeat(48 * 4));
    let mut file = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    let arg = |name: &str, value| Argument { name: name.into(), property_index: 0, value, configuration: None, graph: None };
    let mut effect = Effect {
        name: String::new(),
        icon: String::new(),
        operation: "gegl:alien-map".into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: Default::default(),
    };
    effect.arguments.resize_with(9, || arg("ignored", ArgumentValue::Boolean(false)));
    effect.arguments.push(arg("color-model", ArgumentValue::Enum(0)));
    file.layers[0].effects.push(effect);
    assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::Point(crate::xcf_filters::PointReplaceFamily::AlienRgb));
    let effect = &mut file.layers[0].effects[0];
    effect.arguments.insert(0, arg("excess", ArgumentValue::Boolean(false)));
    assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::ThresholdAlpha);
    let effect = &mut file.layers[0].effects[0];
    effect.arguments.resize_with(100000, || arg("hostile tail", ArgumentValue::Boolean(false)));
    // A color-model present at either end cannot enroll an oversized list.
    effect.arguments[99999] = arg("color-model", ArgumentValue::Enum(1));
    for _ in 0..4 {
        assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::ThresholdAlpha);
    }
}
