//! Saved semantic selections and floating attachments are independent of UI focus.
use crate::{
    LayerId,
    item_identity::{ItemRef, ItemSetId},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloatingAttachment {
    pub layer: LayerId,
    pub target: ItemRef,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditingSelection {
    pub selected: Vec<ItemRef>,
    pub active: Option<ItemRef>,
    pub selected_paths: Vec<ItemRef>,
    pub mask_target: Option<ItemRef>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemFamily {
    Layer,
    Channel,
    Path,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemSetRule {
    Named,
    Text(String),
    Regex(String),
    Glob(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemSet {
    pub id: ItemSetId,
    pub name: String,
    pub family: ItemFamily,
    pub rule: ItemSetRule,
    pub members: Vec<ItemRef>,
}
