//! Test-only exact removal custody, no engine functions or numerical oracle.
#![allow(dead_code, unused_imports)]
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
mod native;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod observed;
pub use observed::{archive, bytes, ctl, data, encoded, import, observed, same, single_root_two, storage, workspace};
use photocraft_doc::{
    Document,
    accounted_document::AccountedDocument,
    item_identity::{FilterId, ItemRef},
};
pub fn current(owner: &AccountedDocument, f: impl FnOnce(&mut Document)) -> AccountedDocument {
    native::current(owner, f)
}
pub fn single_root_three(owner: &AccountedDocument) -> AccountedDocument {
    // Preheld wholeDoc copy retains genuine source/codec/native owner; synthetic CURRENT only.
    current(owner, |d| {
        let root = d.layers.last().unwrap().id;
        d.layers.retain(|l| l.id == root);
        let x = d.xcf_state.as_mut().unwrap();
        x.filter_stacks.retain(|s| s.owner == ItemRef::Layer(root));
        let masks = x.filter_stacks[0].nodes.iter().map(|n| n.mask.unwrap()).collect::<Vec<_>>();
        x.filter_masks.retain(|m| masks.contains(&ItemRef::Mask(m.id)));
        x.items.retain(|i| i.item == ItemRef::Layer(root) || masks.contains(&i.item));
        x.opaque.retain(|r| r.owner.is_none() || r.owner == Some(ItemRef::Layer(root)) || r.owner.is_some_and(|i| masks.contains(&i)));
        x.selection.selected.retain(|i| *i == ItemRef::Layer(root));
        x.selection.active = Some(ItemRef::Layer(root));
    })
}
pub fn preserved(a: &Document, b: &Document, removed: FilterId) {
    assert_eq!(a.layers.len(), b.layers.len());
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(native::raw(a), native::raw(b));
        let (photocraft_doc::LayerContent::BackedRaster(a), photocraft_doc::LayerContent::BackedRaster(b)) = (&a.content, &b.content) else { panic!("backed") };
        assert_eq!(a.descriptor(), b.descriptor());
        assert_eq!(a.default_bytes(), b.default_bytes());
    }
    let old = a.xcf_state.as_ref().unwrap();
    let next = b.xcf_state.as_ref().unwrap();
    assert_eq!(old.provenance, next.provenance);
    assert_eq!(old.selection, next.selection);
    let stack = old.filter_stacks.iter().find(|s| s.nodes.iter().any(|n| n.id == removed)).unwrap();
    let node = stack.nodes.iter().find(|n| n.id == removed).unwrap();
    let ItemRef::Mask(mask) = node.mask.unwrap() else { panic!("mask") };
    assert_eq!(next.filter_masks.len() + 1, old.filter_masks.len());
    assert!(!next.filter_masks.iter().any(|m| m.id == mask));
    assert!(!next.items.iter().any(|i| i.item == ItemRef::Mask(mask)));
    for m in &old.filter_masks {
        if m.id != mask {
            assert_eq!(next.filter_masks.iter().find(|n| n.id == m.id), Some(m));
        }
    }
    for i in &old.items {
        if i.item != ItemRef::Mask(mask) {
            assert_eq!(next.items.iter().find(|n| n.item == i.item), Some(i));
        }
    }
    assert_eq!(old.opaque.len(), next.opaque.len());
    for (a, b) in old.opaque.iter().zip(&next.opaque) {
        if a.owner == Some(ItemRef::Mask(mask)) {
            assert_eq!(b.owner, None);
            assert_eq!(b.identity, format!("deleted-filter-mask/{}/{}", mask.0, a.identity));
            assert_eq!(a.namespace, b.namespace);
            assert_eq!(a.bytes, b.bytes);
            assert!(std::sync::Arc::ptr_eq(&a.bytes, &b.bytes));
            assert_eq!(a.blake3, b.blake3);
            assert_eq!(a.source_version, b.source_version);
            assert_eq!(a.order, b.order);
            assert_eq!(a.flags, b.flags);
            assert_eq!(a.rendering_required, b.rendering_required);
        } else {
            assert_eq!(a, b);
        }
    }
    for st in &old.filter_stacks {
        let new = next.filter_stacks.iter().find(|s| s.owner == st.owner).unwrap();
        if st.owner == stack.owner {
            assert_eq!(new.revision, st.revision + 1);
            assert_eq!(new.nodes, st.nodes.iter().filter(|n| n.id != removed).cloned().collect::<Vec<_>>());
        } else {
            assert_eq!(st, new);
        }
    }
}
