//! Persisted inert source evidence never carries filesystem/capability authority.
use crate::item_identity::ItemRef;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectProvenance {
    pub format: String,
    pub format_version: u32,
    pub container: String,
    pub producer: Option<String>,
    pub release_id: Option<String>,
    pub source_label: String,
    pub source_hash: Option<String>,
    pub original_locator: Option<String>,
    pub protected_source_fingerprints: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueRecord {
    pub owner: Option<ItemRef>,
    pub namespace: String,
    pub identity: String,
    pub source_version: u32,
    pub order: u32,
    pub flags: u32,
    /// Inert raw source bytes shared by snapshots; no permission to reinterpret or reread.
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub blake3: String,
    pub rendering_required: bool,
}
