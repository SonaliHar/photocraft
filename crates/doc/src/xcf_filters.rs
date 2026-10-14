//! Lossless typed filter DTOs. These store semantics; they do not select an evaluator.
use crate::{
    item_identity::{FilterId, GraphNodeId, ItemRef, ResourceId},
    sample_encoding::SampleEncoding,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExactNumber {
    Signed { width: u8, value: i64 },
    Unsigned { width: u8, value: u64 },
    Float32Bits(u32),
    Float64Bits(u64),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactColorStorage {
    pub layout: String,
    pub samples: Vec<u8>,
    pub opaque_tail: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterValue {
    Number(ExactNumber),
    Bool(bool),
    String(String),
    Enum { domain: String, value: i32 },
    Color { components: Vec<ExactNumber>, encoding: SampleEncoding, profile: Option<ResourceId> },
    /// Exact serialized color storage, with no inferred numeric layout, transfer or profile.
    /// Semantic consumers must explicitly bind the layout and original bytes themselves.
    StoredColor(ExactColorStorage),
    Curve { free: bool, points_f64_bits: Vec<[u64; 2]>, point_types: Vec<i32>, samples_f64_bits: Vec<u64> },
    Config(Vec<FilterArgument>),
    Resource(ResourceId),
    Graph(FilterGraph),
}

/// Structural storage bounds only; this does not establish color semantics.
pub(crate) fn stored_color_shape(layout: &str, samples: &[u8], opaque_tail: &[u8]) -> bool {
    !layout.is_empty() && layout.len() <= 256 && !layout.as_bytes().contains(&0)
        && !samples.is_empty() && samples.len() <= 1024 && opaque_tail.len() <= 1 << 20
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterArgument {
    pub name: String,
    pub value: FilterValue,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIdentity {
    pub name: String,
    pub stored_version: String,
    pub schema_epoch: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub id: GraphNodeId,
    pub operation: OperationIdentity,
    /// Chain serialization may omit versions: a qualified resolver version is not source bytes.
    #[serde(default, skip_serializing_if = "GraphVersionOrigin::is_stored")]
    pub version_origin: GraphVersionOrigin,
    pub arguments: Vec<FilterArgument>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphVersionOrigin {
    #[default]
    Stored,
    SchemaResolved,
}
impl GraphVersionOrigin {
    pub fn is_stored(&self) -> bool {
        *self == Self::Stored
    }
}
impl GraphNode {
    /// None means that the original graph did not carry this operation's version.
    pub fn source_version(&self) -> Option<&str> {
        self.version_origin.is_stored().then_some(self.operation.stored_version.as_str())
    }
    /// For SchemaResolved nodes this value comes from the qualified caller schema.
    pub fn resolved_version(&self) -> &str {
        &self.operation.stored_version
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub from: GraphNodeId,
    pub output_port: String,
    pub to: GraphNodeId,
    pub input_port: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterGraph {
    /// External caller-image endpoint; not an operation and never an edge destination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<GraphNodeId>,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub output: GraphNodeId,
    pub original_pipeline: Option<String>,
}

impl FilterGraph {
    /// Charge graph structure to the caller's shared nested-configuration budget
    /// before building identity, port or cycle-validation scratch collections.
    /// Per-graph bounds alone do not bound many individually small sibling graphs.
    /// This does not choose an evaluator, interpret operation ports, or mutate the
    /// graph. Argument values are separately charged by the existing validator.
    pub(crate) fn charge_validation_work(
        &self,
        depth: usize,
        visits: &mut usize,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(), crate::XcfValidationError> {
        if control.cancelled() {
            return Err(crate::XcfValidationError::new("cancelled", "graph validation cancelled"));
        }
        let limit = || crate::XcfValidationError::new("graph_limit", "nested graph validation work exceeds bounds");
        if depth > 32 || self.nodes.len() > 4096 || self.edges.len() > 16384 {
            return Err(limit());
        }
        let total = self
            .nodes
            .len()
            .checked_add(self.edges.len())
            .and_then(|count| count.checked_add(usize::from(self.input.is_some())))
            .and_then(|count| visits.checked_add(count))
            .filter(|count| *count <= 100_000)
            .ok_or_else(limit)?;
        *visits = total;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterNode {
    pub id: FilterId,
    pub operation: OperationIdentity,
    #[serde(default)]
    pub source_display_name: Option<String>,
    #[serde(default)]
    pub source_icon: Option<String>,
    pub arguments: Vec<FilterArgument>,
    pub enabled: bool,
    pub opacity_f64_bits: u64,
    /// Exact stored outer clipping tag; absence never implies Adjust or Clip.
    #[serde(default)]
    pub source_clipping: Option<u32>,
    /// Exact stored filter region tag; absence is distinct from selection region zero.
    #[serde(default)]
    pub source_region: Option<u32>,
    pub mask: Option<ItemRef>,
    pub auxiliary: Vec<(String, ItemRef)>,
    pub composition: Option<crate::import_compositing::GimpComposition>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterStack {
    pub owner: ItemRef,
    pub nodes: Vec<FilterNode>,
    pub revision: u64,
}
