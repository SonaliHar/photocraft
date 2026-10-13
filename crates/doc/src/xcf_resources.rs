//! Inert references only. Loading this data never reads a locator or resolves a font.
use crate::item_identity::ResourceId;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceKind {
    Font,
    Pattern,
    Gradient,
    Brush,
    Palette,
    ColorProfile,
    LinkedImage,
}
/// Current document data, never permission to reread a locator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceFidelity {
    Exact,
    Substituted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceResolution {
    pub fidelity: ResourceFidelity,
    pub payload_hash: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReference {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub name: String,
    pub collection: Option<String>,
    pub internal: bool,
    pub locator: Option<String>,
    pub content_hash: Option<String>,
    /// Owned inert bytes shared across snapshots; this grants no locator access.
    pub owned_bytes: Option<std::sync::Arc<Vec<u8>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ResourceResolution>,
}
