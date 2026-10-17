//! Synthetic declared-compatibility CURRENT only. Genuine prior Source owner
//! remains retained; new typed operations are not GIMP-authored mixed evidence.
#![allow(dead_code, unused_imports)]
#[path = "accounted_filtered_flat_current_test_support.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod source;
use photocraft_doc::{
    Document,
    accounted_document::AccountedDocument,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue, OperationIdentity},
};
use photocraft_raster::backing::quota::{QuotaKind, QuotaLease};
use serde_json::{Value, json};
pub use source::{ctl, payload, same, storage};
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Temperature,
    Sepia,
    AlienRgb,
}
impl Kind {
    pub fn operation(self) -> &'static str {
        match self {
            Self::Temperature => "gegl:color-temperature",
            Self::Sepia => "gegl:sepia",
            Self::AlienRgb => "gegl:alien-map",
        }
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::Temperature => photocraft_compose::COLOR_TEMPERATURE_MIXED_EPOCH,
            Self::Sepia => photocraft_compose::SEPIA_MIXED_EPOCH,
            Self::AlienRgb => photocraft_compose::ALIEN_MAP_RGB_MIXED_EPOCH,
        }
    }
}
fn f(name: &str, v: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn b(name: &str, v: bool) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Bool(v) }
}
fn arguments(kind: Kind) -> Vec<FilterArgument> {
    match kind {
        Kind::Temperature => vec![f("original-temperature", 6500.), f("intended-temperature", 7500.)],
        Kind::Sepia => vec![f("scale", 0.375), b("srgb", true)],
        Kind::AlienRgb => vec![
            FilterArgument { name: "color-model".into(), value: FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 0 } },
            f("cpn-1-frequency", 1.25),
            f("cpn-2-frequency", 1.5),
            f("cpn-3-frequency", 1.75),
            f("cpn-1-phaseshift", 15.),
            f("cpn-2-phaseshift", 30.),
            f("cpn-3-phaseshift", 45.),
            b("cpn-1-keep", false),
            b("cpn-2-keep", false),
            b("cpn-3-keep", false),
        ],
    }
}
struct Hold {
    _source: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
pub fn current(owner: &AccountedDocument, change: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut d = owner.document().clone();
    change(&mut d);
    AccountedDocument::new_readonly_filtered_flat_point_current_storage(
        d,
        Arc::new(Hold { _source: owner.clone(), _index: index, _resident: resident }),
        q,
        &ctl(),
    )
    .unwrap()
}
pub fn build() -> AccountedDocument {
    let input = source::import(&source::data::CASES[1]);
    current(input.document(), |d| {
        let x = d.xcf_state.as_mut().unwrap();
        let stack = x.filter_stacks.iter_mut().find(|s| s.nodes.len() == 3).unwrap();
        for (n, k) in stack.nodes.iter_mut().zip([Kind::Temperature, Kind::Sepia, Kind::AlienRgb]) {
            n.operation = OperationIdentity { name: k.operation().into(), stored_version: "0:0".into(), schema_epoch: k.epoch().into() };
            n.arguments = arguments(k);
            n.composition.as_mut().unwrap().compatibility_epoch = k.epoch().into();
            n.source_display_name = Some(k.operation().into());
        }
    })
}
pub fn node(d: &Document, kind: Kind) -> &FilterNode {
    d.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter()).find(|n| n.operation.schema_epoch == kind.epoch()).unwrap()
}
pub fn patch(d: &Document, kind: Kind) -> Value {
    let n = node(d, kind);
    let mut args = arguments(kind);
    match kind {
        Kind::Temperature => {
            args[0] = f("original-temperature", 6000.);
            args[1] = f("intended-temperature", 8000.);
        }
        Kind::Sepia => {
            args[0] = f("scale", 0.625);
            args[1] = b("srgb", false);
        }
        Kind::AlienRgb => {
            for a in &mut args {
                match &mut a.value {
                    FilterValue::Number(ExactNumber::Float32Bits(bits)) => *bits = (f32::from_bits(*bits) + 0.25).to_bits(),
                    FilterValue::Bool(v) => *v = true,
                    _ => {}
                }
            }
        }
    }
    json!({"filterId":n.id.0.to_string(),"arguments":args.iter().map(|a|json!({"name":a.name,"value":a.value})).collect::<Vec<_>>()})
}
pub fn sole(kind: Kind) -> AccountedDocument {
    let owner = build();
    let id = node(owner.document(), kind).id;
    current(&owner, |d| {
        let x = d.xcf_state.as_mut().unwrap();
        let target = x.filter_stacks.iter().find(|s| s.nodes.iter().any(|n| n.id == id)).unwrap().owner;
        let ItemRef::Layer(root) = target else { panic!("root") };
        d.layers.retain(|l| l.id == root);
        x.filter_stacks.retain(|s| s.owner == target);
        x.filter_stacks[0].nodes.retain(|n| n.id == id);
        let mask = x.filter_stacks[0].nodes[0].mask.unwrap();
        x.filter_masks.retain(|m| ItemRef::Mask(m.id) == mask);
        x.items.retain(|i| i.item == target || i.item == mask);
        x.opaque.retain(|r| r.owner.is_none() || r.owner == Some(target) || r.owner == Some(mask));
        x.selection.selected = vec![target];
        x.selection.active = Some(target);
    })
}
pub fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub fn unmarked_archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::unmarked_archive(owner)
}
