//! UNRUN parameter host support. Captures and expected model copies are
//! external harness data, never allocation or source-authority witnesses.
use photocraft_doc::{
    Document,
    xcf_filters::{ExactNumber, FilterValue},
};
use serde_json::{Value, json};
use std::path::PathBuf;
#[path = "canvas_gaussian_group_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, clippy::panic, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod original;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    original::ctl()
}
pub(super) fn assets() -> PathBuf {
    original::assets()
}
pub(super) fn bits(doc: &Document) -> [u32; 2] {
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    ["std-dev-x", "std-dev-y"].map(|name| {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = node.arguments.iter().find(|a| a.name == name).unwrap().value else {
            panic!("exact F32 axis")
        };
        bits
    })
}
pub(super) fn wire(id: &str, axes: [Option<u32>; 2], enabled: Option<bool>) -> Value {
    let mut value = json!({"filterId":id});
    let args: Vec<_> = ["std-dev-x", "std-dev-y"]
        .into_iter()
        .zip(axes)
        .filter_map(|(name, bits)| bits.map(|bits| json!({"name":name,"value":{"Number":{"Float32Bits":bits}}})))
        .collect();
    if !args.is_empty() {
        value.as_object_mut().unwrap().insert("arguments".into(), Value::Array(args));
    }
    if let Some(enabled) = enabled {
        value.as_object_mut().unwrap().insert("enabled".into(), json!(enabled));
    }
    value
}
pub(super) fn payload(before: &Document, current: &Document) {
    assert_eq!(original::fingerprint(before), original::fingerprint(current));
    let mut expected = before.xcf_state.as_ref().unwrap().clone();
    let target = &current.xcf_state.as_ref().unwrap().filter_stacks[0];
    let stack = &mut expected.filter_stacks[0];
    stack.revision = target.revision;
    stack.nodes[0].enabled = target.nodes[0].enabled;
    for name in ["std-dev-x", "std-dev-y"] {
        let actual = &target.nodes[0].arguments.iter().find(|a| a.name == name).unwrap().value;
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = actual else { panic!("exact F32 argument") };
        assert!(f32::from_bits(*bits).is_finite() && (0.0..=15.).contains(&f32::from_bits(*bits)));
        stack.nodes[0].arguments.iter_mut().find(|a| a.name == name).unwrap().value = actual.clone();
    }
    assert_eq!(Some(expected), current.xcf_state);
    assert_eq!(before.icc_profile, current.icc_profile);
    assert_eq!(before.source_color_context, current.source_color_context);
    assert_eq!(before.working_profile_interpretation, current.working_profile_interpretation);
    assert_eq!(before.sample_encoding, current.sample_encoding);
}
pub(super) fn matrix() -> [[u32; 2]; 6] {
    [[-0.0f32, 1.3f32], [0.8, 0.0], [0.0, -0.0], [0.0, 2.25], [f32::from_bits(0x3f7fffff), 1.0], [15.0, 15.0]].map(|v| v.map(f32::to_bits))
}
