//! Synthetic HSL CURRENT wrappers over a genuinely retained prior ordered Source.
//! No new producer HSL mixed packet or Source admission is claimed.
#![allow(dead_code, unused_imports)]
#[path = "accounted_filtered_flat_stock_rgb_test_support.rs"]
mod rgb;
use photocraft_doc::{
    Document,
    accounted_document::AccountedDocument,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};
pub use rgb::{archive, ctl, payload, same, storage, unmarked_archive};
use serde_json::{Value, json};
fn convert(owner: &AccountedDocument) -> AccountedDocument {
    let id = rgb::node(owner.document(), rgb::Kind::AlienRgb).id;
    rgb::current(owner, |d| {
        let n = d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().flat_map(|s| s.nodes.iter_mut()).find(|n| n.id == id).unwrap();
        n.operation.schema_epoch = photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH.into();
        n.composition.as_mut().unwrap().compatibility_epoch = photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH.into();
        let model = n.arguments.iter_mut().find(|a| a.name == "color-model").unwrap();
        let FilterValue::Enum { domain, value } = &mut model.value else { panic!("enum") };
        assert_eq!(domain, "gegl:alien-map/color-model");
        assert_eq!(*value, 0);
        *value = 1;
    })
}
pub fn build() -> AccountedDocument {
    convert(&rgb::build())
}
pub fn sole() -> AccountedDocument {
    convert(&rgb::sole(rgb::Kind::AlienRgb))
}
pub fn node(d: &Document) -> &FilterNode {
    d.xcf_state
        .as_ref()
        .unwrap()
        .filter_stacks
        .iter()
        .flat_map(|s| s.nodes.iter())
        .find(|n| n.operation.schema_epoch == photocraft_compose::ALIEN_MAP_HSL_MIXED_EPOCH)
        .unwrap()
}
pub fn patch(d: &Document) -> Value {
    let n = node(d);
    let mut args = n.arguments.clone(); // External incoming request, not a claimed model-copy receipt.
    for a in &mut args {
        match &mut a.value {
            FilterValue::Number(ExactNumber::Float32Bits(v)) => *v = (f32::from_bits(*v) + 0.25).to_bits(),
            FilterValue::Bool(v) => *v = true,
            FilterValue::Enum { value, .. } => assert_eq!(*value, 1),
            _ => panic!("exact primitive"),
        }
    }
    json!({"filterId":n.id.0.to_string(),"arguments":args.iter().map(|a|json!({"name":a.name,"value":a.value})).collect::<Vec<_>>()})
}
