//! Imported sample interpretation, independent of storage depth.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleEncoding {
    Linear,
    NonlinearProfile,
    Perceptual,
}

/// Original source description; effective rendering uses Document::icc_profile.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceColorContext {
    pub original_icc: Option<Arc<Vec<u8>>>,
    pub source_precision: u32,
}
