//! Opt-in normal/source-over import policy. No arbitrary operators are representable.
use crate::SampleEncoding;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCompositing {
    pub blend_encoding: SampleEncoding,
    pub composite_encoding: SampleEncoding,
    pub pass_through: bool,
}
