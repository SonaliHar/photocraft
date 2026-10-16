//! External synthetic CURRENT topology over genuine masked-flat native owners.
//! These fixtures do not assert producer-authored masked hierarchy Source facts.
#![allow(dead_code, unused_imports)]
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod hierarchy;
#[path = "accounted_masked_filtered_flat_test_support.rs"]
mod source;
pub use hierarchy::{archive, ctl, payload, same, storage};
use photocraft_doc::{
    BlendMode, Document, Layer, LayerContent, LayerId, SampleEncoding,
    accounted_document::AccountedDocument,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ImportCompositing, ResolvedGimpComposition},
    item_identity::ItemRef,
    xcf_state::ItemState,
};
use photocraft_raster::backing::{BackingError, quota::QuotaKind};
pub use source::cases;
use std::sync::Arc;
pub const OUTER: LayerId = LayerId(910001);
pub const INNER: LayerId = LayerId(910002);
pub const EMPTY: LayerId = LayerId(910003);
fn group(id: LayerId, name: &str, children: Vec<Layer>) -> Layer {
    let mut l = Layer::group(name, children);
    l.id = id;
    l.blend = BlendMode::Normal;
    l.import_compositing = Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    l
}
fn composition() -> GimpComposition {
    GimpComposition {
        compatibility_epoch: "xcf-22-normal".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: -1,
        stored_operator: -1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    }
}
fn rows(layers: &[Layer], parent: Option<ItemRef>, old: &[ItemState], template: &ItemState, out: &mut Vec<ItemState>) {
    for layer in layers {
        let item = ItemRef::Layer(layer.id);
        let mut row = old.iter().find(|r| r.item == item).unwrap_or(template).clone();
        row.item = item;
        row.parent = parent;
        row.name = layer.name.clone();
        row.visible = layer.visible;
        if layer.is_group() {
            row.raster_bounds = None;
            row.mask = None;
            row.source_tattoo = None;
            row.source_flags = 0;
            row.composition = Some(composition());
        }
        out.push(row);
        if let LayerContent::Group(g) = &layer.content {
            rows(&g.children, Some(item), old, template, out);
        }
    }
}
pub fn build(case: &serde_json::Value) -> AccountedDocument {
    let imported = source::import(case);
    let original = imported.document().clone();
    let q = original.quota().clone();
    // Conservative fees for this small external full-clone fixture, retained
    // through the factory; never use expected output buffers as owner receipts.
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = original.document().clone();
    let leaves = std::mem::take(&mut d.layers);
    assert!(!leaves.is_empty());
    let mut inner = group(INNER, "Inner masked leaves", leaves);
    inner.opacity = 0.625;
    let empty = group(EMPTY, "Empty", vec![]);
    let mut outer = group(OUTER, "Outer", vec![inner, empty]);
    outer.opacity = 0.75;
    d.layers = vec![outer];
    let state = d.xcf_state.as_mut().unwrap();
    let old = std::mem::take(&mut state.items);
    let template = old.iter().find(|r| matches!(r.item, ItemRef::Layer(_))).unwrap().clone();
    let mut canonical = Vec::new();
    rows(&d.layers, None, &old, &template, &mut canonical);
    canonical.extend(old.iter().filter(|r| matches!(r.item, ItemRef::Mask(_))).cloned());
    state.items = canonical;
    state.selection.selected = vec![ItemRef::Layer(OUTER)];
    state.selection.active = Some(ItemRef::Layer(OUTER));
    drop(template);
    drop(old);
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, Arc::new((original, index, resident)), q, &ctl()).unwrap()
}
pub fn changed(owner: &AccountedDocument, change: impl FnOnce(&mut Document)) -> Result<AccountedDocument, BackingError> {
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = owner.document().clone();
    change(&mut d);
    AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(d, Arc::new((owner.clone(), index, resident)), q, &ctl())
}
