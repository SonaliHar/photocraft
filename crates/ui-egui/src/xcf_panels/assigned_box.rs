//! Bounded presentation hint for assigned Box activation only. The registered
//! command establishes complete current/profile/native semantics on every action.
use photocraft_doc::{
    LayerContent, LayerId,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};
use photocraft_engine::DocumentSnapshot;
pub(super) fn candidate(document: &DocumentSnapshot, layer: LayerId, node: &FilterNode) -> bool {
    if !document.accounted().is_some_and(|owner| owner.is_readonly_assigned_point_storage()) {
        return false;
    }
    let [owner] = document.layers.as_slice() else { return false };
    let Some(state) = &document.xcf_state else { return false };
    let [stack] = state.filter_stacks.as_slice() else { return false };
    let [retained] = stack.nodes.as_slice() else { return false };
    let [argument] = node.arguments.as_slice() else { return false };
    owner.id == layer
        && matches!(owner.content, LayerContent::BackedRaster(_))
        && stack.owner == ItemRef::Layer(layer)
        && stack.revision < u64::MAX
        && node.id.0 != 0
        && std::ptr::eq(retained, node)
        && node.operation.name == "gegl:box-blur"
        && node.operation.stored_version == "0:0"
        && node.operation.schema_epoch == "xcf22-rgb-box-replace-v1"
        && argument.name == "radius"
        && matches!(argument.value, FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 | 5 }))
}
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "assigned_box_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
