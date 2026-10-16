//! Optional canonical imported state with explicit, bounded relationship validation.
use crate::{Document, LayerId, item_identity::*, xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XcfValidationError {
    pub code: &'static str,
    pub message: String,
}
impl XcfValidationError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}
impl fmt::Display for XcfValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for XcfValidationError {}
type Result<T> = std::result::Result<T, XcfValidationError>;
fn invalid(code: &'static str) -> XcfValidationError {
    XcfValidationError::new(code, "invalid or unresolved imported state")
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemLocks {
    pub content: bool,
    pub position: bool,
    pub alpha: bool,
    pub visibility: bool,
}
impl ItemLocks {
    pub fn union(self, b: Self) -> Self {
        Self {
            content: self.content || b.content,
            position: self.position || b.position,
            alpha: self.alpha || b.alpha,
            visibility: self.visibility || b.visibility,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskState {
    pub owner: ItemRef,
    pub bounds: crate::Rect,
    pub offset: [i32; 2],
    pub outside_f64_bits: u64,
    pub enabled: bool,
    pub shown: bool,
    pub editing: bool,
    pub linked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemState {
    pub item: ItemRef,
    pub name: String,
    pub custom_name: bool,
    pub visible: bool,
    pub locks: ItemLocks,
    pub parent: Option<ItemRef>,
    pub source_tattoo: Option<u32>,
    pub source_flags: u32,
    /// Exact declared drawable extent; sparse tile content is not an authority for it.
    #[serde(default)]
    pub raster_bounds: Option<crate::Rect>,
    pub mask: Option<MaskState>,
    pub composition: Option<crate::import_compositing::GimpComposition>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XcfState {
    pub provenance: ProjectProvenance,
    pub items: Vec<ItemState>,
    pub resources: Vec<ResourceReference>,
    pub source_items: Vec<SourceItem>,
    #[serde(default)]
    pub editable_indexed: Vec<crate::EditableIndexedLayer>,
    pub filter_stacks: Vec<FilterStack>,
    /// Current generated buffer storage; original source precision remains immutable provenance.
    #[serde(default, skip_serializing_if = "crate::FilterOutputPrecision::is_canonical")]
    pub current_filter_output_precision: crate::FilterOutputPrecision,
    /// Raster payloads use the native format's dedicated binary surface mapping.
    #[serde(skip)]
    pub filter_masks: Vec<crate::FilterMask>,
    pub floating: Option<FloatingAttachment>,
    pub selection: EditingSelection,
    pub item_sets: Vec<ItemSet>,
    pub opaque: Vec<OpaqueRecord>,
}
fn text(s: &str) -> Result<()> {
    if s.len() > 1 << 20 || s.contains('\0') { Err(invalid("string_limit")) } else { Ok(()) }
}
fn hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn payload(bytes: &[u8], expected: &str, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if bytes.len() > 64 << 20 || !hash(expected) || crate::xcf_items::checked_payload_hash(bytes, control)?.to_hex().as_str() != expected {
        Err(invalid("payload_hash"))
    } else {
        Ok(())
    }
}
fn resource(id: ResourceId, resources: &BTreeSet<ResourceId>) -> Result<()> {
    if resources.contains(&id) { Ok(()) } else { Err(invalid("resource_reference")) }
}
fn reference(id: ItemRef, items: &BTreeSet<ItemRef>) -> Result<()> {
    if items.contains(&id) { Ok(()) } else { Err(invalid("item_reference")) }
}
fn identity(v: &OperationIdentity) -> Result<()> {
    for s in [&v.name, &v.stored_version, &v.schema_epoch] {
        text(s)?;
        if s.is_empty() {
            return Err(invalid("operation_identity"));
        }
    }
    Ok(())
}
fn number(n: &ExactNumber) -> Result<()> {
    match n {
        ExactNumber::Signed { width, value } => {
            if ![8, 16, 32, 64].contains(width) || (*width < 64 && (*value < -(1i64 << (*width - 1)) || *value >= 1i64 << (*width - 1))) {
                return Err(invalid("integer_width"));
            }
        }
        ExactNumber::Unsigned { width, value } => {
            if ![8, 16, 32, 64].contains(width) || (*width < 64 && *value >= 1u64 << *width) {
                return Err(invalid("integer_width"));
            }
        }
        ExactNumber::Float32Bits(_) | ExactNumber::Float64Bits(_) => {}
    }
    Ok(())
}
fn cancellation(control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    if control.cancelled() { Err(invalid("cancelled")) } else { Ok(()) }
}
fn arguments(
    args: &[FilterArgument],
    resources: &BTreeSet<ResourceId>,
    depth: usize,
    visits: &mut usize,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    cancellation(control)?;
    if depth > 32 || args.len() > 100000 {
        return Err(invalid("config_limit"));
    }
    let mut names = BTreeSet::new();
    for a in args {
        cancellation(control)?;
        text(&a.name)?;
        if a.name.is_empty() || !names.insert(&a.name) {
            return Err(invalid("argument_name"));
        }
        *visits = visits.checked_add(1).ok_or_else(|| invalid("config_limit"))?;
        if *visits > 100000 {
            return Err(invalid("config_limit"));
        }
        match &a.value {
            FilterValue::Number(n) => number(n)?,
            FilterValue::Bool(_) => {}
            FilterValue::String(s) => text(s)?,
            FilterValue::Enum { domain, .. } => {
                text(domain)?;
                if domain.is_empty() {
                    return Err(invalid("enum_identity"));
                }
            }
            FilterValue::Resource(id) => resource(*id, resources)?,
            FilterValue::Config(c) => arguments(c, resources, depth + 1, visits, control)?,
            FilterValue::Graph(g) => graph(g, resources, depth + 1, visits, control)?,
            FilterValue::Color { components, profile, .. } => {
                if components.is_empty() || components.len() > 16 {
                    return Err(invalid("color_components"));
                }
                for n in components {
                    cancellation(control)?;
                    number(n)?
                }
                if let Some(id) = profile {
                    resource(*id, resources)?
                }
            }
            FilterValue::StoredColor(ExactColorStorage { layout, samples, opaque_tail }) => {
                if !stored_color_shape(layout, samples, opaque_tail) {
                    return Err(invalid("stored_color_storage"));
                }
                for _ in samples.chunks(32 << 10).chain(opaque_tail.chunks(32 << 10)) {
                    cancellation(control)?;
                }
                cancellation(control)?;
            }
            FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => {
                if points_f64_bits.len() > 100000
                    || samples_f64_bits.len() > 100000
                    || (*free && (!points_f64_bits.is_empty() || !point_types.is_empty()))
                    || (!*free && (points_f64_bits.len() != point_types.len() || !samples_f64_bits.is_empty()))
                {
                    return Err(invalid("curve_shape"));
                }
                for (index, n) in points_f64_bits.iter().flatten().chain(samples_f64_bits.iter()).enumerate() {
                    if index.is_multiple_of(256) {
                        cancellation(control)?;
                    }
                    if !f64::from_bits(*n).is_finite() {
                        return Err(invalid("curve_nonfinite"));
                    }
                }
            }
        }
    }
    Ok(())
}
fn graph(g: &FilterGraph, resources: &BTreeSet<ResourceId>, depth: usize, visits: &mut usize, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
    cancellation(control)?;
    g.charge_validation_work(depth, visits, control)?;
    let mut indegree = BTreeMap::new();
    if let Some(input) = g.input {
        if input.0 == 0 {
            return Err(invalid("graph_input"));
        }
        indegree.insert(input, 0usize);
    }
    let mut outgoing: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
    for n in &g.nodes {
        cancellation(control)?;
        if n.id.0 == 0 || indegree.insert(n.id, 0usize).is_some() {
            return Err(invalid("graph_identity"));
        }
        identity(&n.operation)?;
        arguments(&n.arguments, resources, depth + 1, visits, control)?
    }
    if !indegree.contains_key(&g.output) {
        return Err(invalid("graph_output"));
    }
    let mut ports = BTreeSet::new();
    for e in &g.edges {
        cancellation(control)?;
        text(&e.input_port)?;
        text(&e.output_port)?;
        if e.input_port.is_empty()
            || e.output_port.is_empty()
            || !indegree.contains_key(&e.from)
            || Some(e.to) == g.input
            || !ports.insert((e.to, &e.input_port))
        {
            return Err(invalid("graph_edge"));
        }
        let d = indegree.get_mut(&e.to).ok_or_else(|| invalid("graph_edge"))?;
        *d = d.checked_add(1).ok_or_else(|| invalid("graph_limit"))?;
        outgoing.entry(e.from).or_default().push(e.to)
    }
    let mut ready = Vec::new();
    for (id, n) in &indegree {
        cancellation(control)?;
        if *n == 0 {
            ready.push(*id);
        }
    }
    let mut count = 0usize;
    while let Some(id) = ready.pop() {
        cancellation(control)?;
        count += 1;
        if let Some(edges) = outgoing.get(&id) {
            for to in edges {
                cancellation(control)?;
                let n = indegree.get_mut(to).ok_or_else(|| invalid("graph_edge"))?;
                *n = n.checked_sub(1).ok_or_else(|| invalid("graph_edge"))?;
                if *n == 0 {
                    ready.push(*to)
                }
            }
        }
    }
    if count != g.nodes.len().saturating_add(usize::from(g.input.is_some())) {
        return Err(invalid("graph_cycle"));
    }
    if let Some(s) = &g.original_pipeline {
        text(s)?
    }
    Ok(())
}
impl XcfState {
    /// Check recursive/payload bounds before transaction snapshots are cloned.
    pub(crate) fn check_clone_bounds(&self) -> Result<()> {
        self.check_clone_bounds_with(&photocraft_raster::Interrupt::NONE)
    }
    pub(crate) fn check_clone_bounds_with(&self, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        if control.cancelled() {
            return Err(invalid("cancelled"));
        }
        if self.items.len() > 10000
            || self.resources.len() > 10000
            || self.source_items.len() > 10000
            || self.filter_masks.len() > 10000
            || self.editable_indexed.len() > 10000
            || self.filter_stacks.len() > 10000
            || self.opaque.len() > 100000
            || self.item_sets.len() > 10000
        {
            return Err(invalid("item_limit"));
        }
        let resources = self.resources.iter().map(|r| r.id).collect();
        let mut visits = 0;
        let mut bytes = 0usize;
        let mut add = |count: usize| -> Result<()> {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            bytes = bytes.checked_add(count).ok_or_else(|| invalid("payload_limit"))?;
            if bytes > 256 << 20 {
                return Err(invalid("payload_limit"));
            }
            Ok(())
        };
        for item in &self.items {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            text(&item.name)?;
            add(item.name.len())?;
        }
        for resource in &self.resources {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            text(&resource.name)?;
            add(resource.name.len())?;
            if let Some(resolution) = &resource.resolution {
                if !hash(&resolution.payload_hash) {
                    return Err(invalid("resource_resolution"));
                }
                add(resolution.payload_hash.len())?;
            }
            if let Some(bytes) = &resource.owned_bytes {
                add(bytes.len())?;
            }
        }
        for item in &self.editable_indexed {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            item.raster.validate_with(control)?;
            add(item.raster.indices().len())?;
            add(item.raster.alpha().map_or(0, |a| a.len()))?;
        }
        for source in &self.source_items {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            match &source.source {
                EditableSource::Text { content, markup, configuration, .. } => {
                    text(content)?;
                    add(content.len())?;
                    if let Some(markup) = markup {
                        text(markup)?;
                        add(markup.len())?;
                    }
                    arguments(configuration, &resources, 0, &mut visits, control)?;
                }
                EditableSource::Vector { configuration, .. } => arguments(configuration, &resources, 0, &mut visits, control)?,
                EditableSource::Linked { configuration, .. } => arguments(configuration, &resources, 0, &mut visits, control)?,
            }
            if let Some(cache) = &source.cache {
                if cache.bytes.len() > 64 << 20 {
                    return Err(invalid("payload_limit"));
                }
                add(cache.bytes.len())?;
            }
        }
        for mask in &self.filter_masks {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            add(usize::try_from(mask.surface.resident_payload_bytes().map_err(|_| invalid("filter_mask_limit"))?).map_err(|_| invalid("filter_mask_limit"))?)?;
        }
        for stack in &self.filter_stacks {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if stack.nodes.len() > 1024 {
                return Err(invalid("filter_limit"));
            }
            for node in &stack.nodes {
                if control.cancelled() {
                    return Err(invalid("cancelled"));
                }
                identity(&node.operation)?;
                for value in [&node.source_display_name, &node.source_icon].into_iter().flatten() {
                    if control.cancelled() {
                        return Err(invalid("cancelled"));
                    }
                    text(value)?;
                }
                arguments(&node.arguments, &resources, 0, &mut visits, control)?;
            }
        }
        for record in &self.opaque {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if record.bytes.len() > 64 << 20 {
                return Err(invalid("payload_limit"));
            }
            add(record.bytes.len())?;
        }
        Ok(())
    }
    pub fn validate(&self, doc: &Document) -> Result<()> {
        self.validate_with(doc, &photocraft_raster::Interrupt::NONE)
    }
    pub fn validate_with(&self, doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        self.validate_with_selected_filter_mask(doc, control, None)
    }
    pub(crate) fn validate_with_selected_filter_mask(
        &self,
        doc: &Document,
        control: &photocraft_raster::Interrupt<'_>,
        selected: Option<&crate::xcf_filter_masks::CheckedFilterMaskCoverage<'_>>,
    ) -> Result<()> {
        if let Some(proof) = selected {
            let mut found = 0usize;
            for mask in &self.filter_masks {
                control.check().map_err(|_| invalid("cancelled"))?;
                if proof.covers(mask) {
                    found += 1;
                }
            }
            if found != 1 {
                return Err(invalid("filter_mask_identity"));
            }
        }

        if control.cancelled() {
            return Err(invalid("cancelled"));
        }
        self.check_clone_bounds_with(control)?;
        self.current_filter_output_precision.validate(self, doc)?;
        if self.items.len() > 10000
            || self.resources.len() > 10000
            || self.source_items.len() > 10000
            || self.filter_masks.len() > 10000
            || self.editable_indexed.len() > 10000
            || self.opaque.len() > 100000
            || self.item_sets.len() > 10000
        {
            return Err(invalid("item_limit"));
        }
        // Qualify the complete cache budget before hashing any cache.
        let mut cache_bytes = 0usize;
        for source in &self.source_items {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if let Some(cache) = &source.cache {
                let len = cache.checked_byte_len()?;
                cache_bytes = cache_bytes.checked_add(len).ok_or_else(|| invalid("payload_limit"))?;
                if cache.bytes.len() != len {
                    return Err(invalid("cache_bounds"));
                }
                if cache_bytes > 256 << 20 {
                    return Err(invalid("payload_limit"));
                }
            }
        }
        let mut channel_ids = BTreeSet::new();
        if let Some(id) = doc.selection_channel_id
            && (id.0 == 0 || !channel_ids.insert(id))
        {
            return Err(invalid("channel_identity"));
        }
        if doc.channels.len() > 10000 || doc.channels.iter().any(|c| c.id.0 == 0 || !channel_ids.insert(c.id)) {
            return Err(invalid("channel_identity"));
        }
        // Bounded iterative inspection avoids recursive layer lookup on hostile trees.
        let mut layers = BTreeMap::new();
        let mut pending = vec![(doc.layers.as_slice(), 0usize)];
        while let Some((siblings, depth)) = pending.pop() {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if depth > 100 {
                return Err(invalid("hierarchy_limit"));
            }
            for layer in siblings {
                if control.cancelled() {
                    return Err(invalid("cancelled"));
                }
                if layer.id.0 == 0 || layers.insert(layer.id, layer.is_group()).is_some() || layers.len() > 10000 {
                    return Err(invalid("layer_identity"));
                }
                if let crate::LayerContent::Group(group) = &layer.content {
                    pending.push((group.children.as_slice(), depth + 1));
                }
            }
        }
        let mut mask_ids = BTreeSet::new();
        for mask in &self.filter_masks {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if !mask_ids.insert(mask.id) {
                return Err(invalid("filter_mask_identity"));
            }
            if !selected.is_some_and(|proof| proof.covers(mask)) {
                mask.validate_with(control)?;
            }
        }
        let mut indexed_owners = BTreeSet::new();
        let mut indexed_bytes = 0usize;
        for indexed in &self.editable_indexed {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if layers.get(&indexed.owner) != Some(&false) || !indexed_owners.insert(indexed.owner) {
                return Err(invalid("indexed_owner"));
            }
            indexed.raster.validate_with(control)?;
            let bytes = indexed.raster.indices().len().checked_add(indexed.raster.alpha().map_or(0, |a| a.len())).ok_or_else(|| invalid("indexed_limit"))?;
            indexed_bytes = indexed_bytes.checked_add(bytes).ok_or_else(|| invalid("indexed_limit"))?;
            if indexed_bytes > 128 << 20 {
                return Err(invalid("indexed_limit"));
            }
        }
        let mut items = BTreeSet::new();
        for i in &self.items {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if i.item.raw() == 0 || !items.insert(i.item) {
                return Err(invalid("item_identity"));
            }
            text(&i.name)?;
            if let Some(bounds) = i.raster_bounds {
                let width = i64::from(bounds.x1) - i64::from(bounds.x0);
                let height = i64::from(bounds.y1) - i64::from(bounds.y0);
                if width <= 0 || height <= 0 || width > 262144 || height > 262144 {
                    return Err(invalid("raster_bounds"));
                }
            }
            if let ItemRef::Channel(id) = i.item
                && doc.selection_channel_id != Some(id)
                && !doc.channels.iter().any(|channel| channel.id == id)
            {
                return Err(invalid("channel_reference"));
            }
            if let ItemRef::Path(id) = i.item
                && !doc.paths.iter().any(|path| path.id == id)
            {
                return Err(invalid("path_reference"));
            }
            if let ItemRef::Layer(id) = i.item
                && !layers.contains_key(&id)
            {
                return Err(invalid("layer_reference"));
            }
        }
        for i in &self.items {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if let Some(p) = i.parent {
                reference(p, &items)?;
                if !matches!(p,ItemRef::Layer(id) if layers.get(&id).copied().unwrap_or(false)) {
                    return Err(invalid("parent_type"));
                }
            }
            self.effective_locks(i.item)?;
            match (&i.mask, i.item) {
                (Some(m), ItemRef::Mask(_)) => {
                    reference(m.owner, &items)?;
                    if !(matches!(m.owner, ItemRef::Layer(_)) || (mask_ids.contains(&MaskId(i.item.raw())) && matches!(m.owner, ItemRef::Channel(_))))
                        || m.bounds.x1 < m.bounds.x0
                        || m.bounds.y1 < m.bounds.y0
                        || i64::from(m.bounds.x1) - i64::from(m.bounds.x0) > 262144
                        || i64::from(m.bounds.y1) - i64::from(m.bounds.y0) > 262144
                        || !f64::from_bits(m.outside_f64_bits).is_finite()
                    {
                        return Err(invalid("mask_bounds"));
                    }
                }
                (None, ItemRef::Mask(id)) if mask_ids.contains(&id) => {}
                (None, ItemRef::Mask(_)) | (Some(_), _) => return Err(invalid("mask_owner")),
                _ => {}
            }
        }
        let mut resources = BTreeSet::new();
        let mut resource_bytes = 0usize;
        for r in &self.resources {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if r.id.0 == 0 || !resources.insert(r.id) {
                return Err(invalid("resource_identity"));
            }
            text(&r.name)?;
            for s in [&r.locator, &r.collection].into_iter().flatten() {
                if control.cancelled() {
                    return Err(invalid("cancelled"));
                }
                text(s)?
            }
            if let Some(h) = &r.content_hash
                && !hash(h)
            {
                return Err(invalid("resource_hash"));
            }
            if let Some(b) = &r.owned_bytes {
                if r.kind == ResourceKind::Font {
                    return Err(invalid("embedded_font"));
                }
                resource_bytes = resource_bytes.checked_add(b.len()).ok_or_else(|| invalid("resource_limit"))?;
                if resource_bytes > 128 << 20 {
                    return Err(invalid("resource_limit"));
                }
                let observed = crate::source_cache_hash::checked_digest(b, control)?;
                let expected = r.resolution.as_ref().map(|resolution| resolution.payload_hash.as_str()).or(r.content_hash.as_deref());
                if expected.is_none_or(|h| !observed.to_hex().as_str().eq_ignore_ascii_case(h)) {
                    return Err(invalid("resource_hash"));
                }
            }
            if let Some(resolution) = &r.resolution {
                if !hash(&resolution.payload_hash) || r.owned_bytes.is_none() {
                    return Err(invalid("resource_resolution"));
                }
                if resolution.fidelity == crate::xcf_resources::ResourceFidelity::Exact && r.content_hash.as_deref().is_none_or(|original| !original.eq_ignore_ascii_case(&resolution.payload_hash)) {
                    return Err(invalid("resource_exact_identity"));
                }
            }
        }
        let mut visits = 0;
        let mut owners = BTreeSet::new();
        let mut filters = BTreeSet::new();
        for stack in &self.filter_stacks {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            reference(stack.owner, &items)?;
            if self.current_filter_output_precision == crate::FilterOutputPrecision::Half
                && !matches!(stack.owner, ItemRef::Layer(id) if layers.get(&id) == Some(&false))
            {
                return Err(invalid("filter_output_precision"));
            }
            if !matches!(stack.owner, ItemRef::Layer(_) | ItemRef::Channel(_)) || !owners.insert(stack.owner) || stack.nodes.len() > 1024 {
                return Err(invalid("filter_owner"));
            }
            for n in &stack.nodes {
                if control.cancelled() {
                    return Err(invalid("cancelled"));
                }
                if n.id.0 == 0 || !filters.insert(n.id) || filters.len() > 100000 {
                    return Err(invalid("filter_identity"));
                }
                identity(&n.operation)?;
                for value in [&n.source_display_name, &n.source_icon].into_iter().flatten() {
                    if control.cancelled() {
                        return Err(invalid("cancelled"));
                    }
                    text(value)?;
                }
                let opacity = f64::from_bits(n.opacity_f64_bits);
                if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                    return Err(invalid("filter_opacity"));
                }
                if let Some(m) = n.mask {
                    reference(m, &items)?;
                    if !matches!(m, ItemRef::Mask(_)) {
                        return Err(invalid("filter_mask"));
                    }
                    if let ItemRef::Mask(id) = m
                        && self.filter_masks.iter().any(|mask| mask.id == id && mask.owner != n.id)
                    {
                        return Err(invalid("filter_mask_owner"));
                    }
                }
                for (port, input) in &n.auxiliary {
                    if control.cancelled() {
                        return Err(invalid("cancelled"));
                    }
                    text(port)?;
                    reference(*input, &items)?
                }
                arguments(&n.arguments, &resources, 0, &mut visits, control)?
            }
        }
        for mask in &self.filter_masks {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            reference(ItemRef::Mask(mask.id), &items)?;
            let item = self.items.iter().find(|item| item.item == ItemRef::Mask(mask.id)).ok_or_else(|| invalid("filter_mask_owner"))?;
            let stack = self
                .filter_stacks
                .iter()
                .find(|stack| stack.nodes.iter().any(|node| node.id == mask.owner && node.mask == Some(ItemRef::Mask(mask.id))))
                .ok_or_else(|| invalid("filter_mask_owner"))?;
            if let Some(metadata) = &item.mask
                && (metadata.owner != stack.owner || metadata.bounds != mask.bounds)
            {
                return Err(invalid("filter_mask_owner"));
            }
            if item.raster_bounds.is_some_and(|bounds| bounds != mask.bounds) {
                return Err(invalid("filter_mask_bounds"));
            }
        }
        let mut sources = BTreeSet::new();
        for s in &self.source_items {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            reference(s.owner, &items)?;
            if !matches!(s.owner, ItemRef::Layer(_)) || !sources.insert(s.owner) {
                return Err(invalid("source_owner"));
            }
            match &s.source {
                EditableSource::Text { content, markup, font, configuration, current_origin, source_modified, .. } => {
                    if current_origin.is_some()
                        && (*source_modified
                            || current_origin.is_some_and(|origin| origin.iter().any(|coordinate| !(-(1 << 29)..=(1 << 29)).contains(coordinate))))
                    {
                        return Err(invalid("text_current_origin"));
                    }
                    text(content)?;
                    if let Some(m) = markup {
                        text(m)?
                    }
                    resource(*font, &resources)?;
                    if !self.resources.iter().any(|r| r.id == *font && r.kind == ResourceKind::Font) {
                        return Err(invalid("font_reference"));
                    }
                    arguments(configuration, &resources, 0, &mut visits, control)?
                }
                EditableSource::Vector { path, configuration, .. } => {
                    reference(ItemRef::Path(*path), &items)?;
                    arguments(configuration, &resources, 0, &mut visits, control)?
                }
                EditableSource::Linked { resource: id, last_verified_hash, configuration, current } => {
                    arguments(configuration, &resources, 0, &mut visits, control)?;
                    resource(*id, &resources)?;
                    if !self.resources.iter().any(|r| r.id == *id && r.kind == ResourceKind::LinkedImage) {
                        return Err(invalid("linked_resource"));
                    }
                    if last_verified_hash.as_ref().is_some_and(|h| !hash(h)) {
                        return Err(invalid("linked_hash"));
                    }
                    if let Some(current) = current {
                        if current.bounds.is_empty()
                            || [current.bounds.x0, current.bounds.y0, current.bounds.x1, current.bounds.y1]
                                .iter()
                                .any(|value| !(-(1 << 29)..=(1 << 29)).contains(value))
                            || self.items.iter().find(|item| item.item == s.owner).and_then(|item| item.raster_bounds) != Some(current.bounds)
                        {
                            return Err(invalid("linked_current_bounds"));
                        }
                        if !self.resources.iter().any(|resource| resource.id == *id && resource.owned_bytes.is_some() && resource.resolution.is_some()) {
                            return Err(invalid("linked_current_resource"));
                        }
                        if current.transform.as_ref().is_some_and(|transform| {
                            transform.matrix_f64_bits.iter().any(|value| !f64::from_bits(*value).is_finite())
                                || transform.pre_offset.iter().any(|value| !(-(1 << 29)..=(1 << 29)).contains(value))
                                || !(0..=3).contains(&transform.interpolation)
                        }) {
                            return Err(invalid("linked_current_transform"));
                        }
                    }
                }
            }
            if let Some(t) = &s.transform
                && t.matrix_f64_bits.iter().any(|b| !f64::from_bits(*b).is_finite())
            {
                return Err(invalid("transform_nonfinite"));
            }
            if let Some(c) = &s.cache {
                c.validate_with(control)?;
            }
        }
        if let Some(f) = &self.floating {
            reference(ItemRef::Layer(f.layer), &items)?;
            reference(f.target, &items)?;
            if f.target == ItemRef::Layer(f.layer) || matches!(f.target, ItemRef::Path(_)) {
                return Err(invalid("floating_target"));
            }
        }
        let mut selected = BTreeSet::new();
        for r in &self.selection.selected {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            reference(*r, &items)?;
            if !selected.insert(*r) {
                return Err(invalid("selection_duplicate"));
            }
        }
        if let Some(a) = self.selection.active
            && !selected.contains(&a)
        {
            return Err(invalid("active_selection"));
        }
        for r in &self.selection.selected_paths {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            reference(*r, &items)?;
            if !matches!(r, ItemRef::Path(_)) {
                return Err(invalid("path_selection"));
            }
        }
        if let Some(r) = self.selection.mask_target {
            reference(r, &items)?;
            if !matches!(r, ItemRef::Mask(_)) {
                return Err(invalid("mask_selection"));
            }
        }
        let mut sets = BTreeSet::new();
        for s in &self.item_sets {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            text(&s.name)?;
            if s.id.0 == 0 || !sets.insert(s.id) {
                return Err(invalid("set_identity"));
            }
            if let ItemSetRule::Text(v) | ItemSetRule::Regex(v) | ItemSetRule::Glob(v) = &s.rule {
                text(v)?;
                if !s.members.is_empty() {
                    return Err(invalid("search_membership"));
                }
            }
            let mut members = BTreeSet::new();
            for r in &s.members {
                if control.cancelled() {
                    return Err(invalid("cancelled"));
                }
                reference(*r, &items)?;
                if !matches!(
                    (s.family, r),
                    (ItemFamily::Layer, ItemRef::Layer(_)) | (ItemFamily::Channel, ItemRef::Channel(_)) | (ItemFamily::Path, ItemRef::Path(_))
                ) || !members.insert(*r)
                {
                    return Err(invalid("set_member"));
                }
            }
        }
        let mut opaque_bytes = 0usize;
        for o in &self.opaque {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            if let Some(r) = o.owner {
                reference(r, &items)?
            }
            text(&o.namespace)?;
            text(&o.identity)?;
            if o.rendering_required {
                return Err(invalid("opaque_rendering_state"));
            }
            payload(&o.bytes, &o.blake3, control)?;
            opaque_bytes = opaque_bytes.checked_add(o.bytes.len()).ok_or_else(|| invalid("opaque_limit"))?;
            if opaque_bytes > 256 << 20 {
                return Err(invalid("opaque_limit"));
            }
        }
        for s in [&self.provenance.format, &self.provenance.container, &self.provenance.source_label] {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            text(s)?
        }
        for s in [&self.provenance.producer, &self.provenance.release_id, &self.provenance.original_locator].into_iter().flatten() {
            if control.cancelled() {
                return Err(invalid("cancelled"));
            }
            text(s)?
        }
        if self.provenance.source_hash.as_ref().is_some_and(|h| !hash(h)) || self.provenance.protected_source_fingerprints.iter().any(|h| !hash(h)) {
            return Err(invalid("source_hash"));
        }
        Ok(())
    }
    pub fn effective_locks(&self, item: ItemRef) -> Result<ItemLocks> {
        let mut result = ItemLocks::default();
        let mut next = Some(item);
        let mut seen = BTreeSet::new();
        while let Some(id) = next {
            if !seen.insert(id) || seen.len() > 100 {
                return Err(invalid("hierarchy_cycle"));
            }
            let state = self.items.iter().find(|s| s.item == id).ok_or_else(|| invalid("item_reference"))?;
            result = result.union(state.locks);
            next = state.parent
        }
        Ok(result)
    }
    /// Transactionally remap channel identities, including selections and all dependencies.
    pub fn remap_channels(&mut self, mut map: impl FnMut(ChannelId) -> Option<ChannelId>) -> Result<()> {
        self.check_clone_bounds()?;
        // Borrow the reference census; channel remapping must not copy source,
        // resource or opaque payloads merely to establish transactional safety.
        let mut planned = std::collections::HashMap::<ChannelId, ChannelId>::new();
        let mut destinations = std::collections::HashSet::<ChannelId>::new();
        let mut record = |reference: ItemRef| -> Result<()> {
            let ItemRef::Channel(id) = reference else { return Ok(()) };
            if id.0 == 0 {
                return Err(invalid("channel_identity"));
            }
            if planned.contains_key(&id) {
                return Ok(());
            }
            // The existing canonical drawable ceiling bounds distinct channels,
            // while repeated references to one old identity consume no new slot.
            if planned.len() >= 10000 {
                return Err(invalid("item_limit"));
            }
            let target = map(id).ok_or_else(|| invalid("channel_reference"))?;
            if target.0 == 0 || destinations.contains(&target) {
                return Err(invalid("channel_identity"));
            }
            planned.try_reserve(1).map_err(|_| invalid("item_limit"))?;
            destinations.try_reserve(1).map_err(|_| invalid("item_limit"))?;
            planned.insert(id, target);
            destinations.insert(target);
            Ok(())
        };
        let Self {
            items,
            source_items,
            filter_stacks,
            selection,
            item_sets,
            opaque,
            floating,
            provenance: _,
            resources: _,
            editable_indexed: _,
            current_filter_output_precision: _,
            filter_masks: _,
        } = &*self;
        for item in items {
            record(item.item)?;
            if let Some(parent) = item.parent {
                record(parent)?;
            }
            if let Some(mask) = &item.mask {
                record(mask.owner)?;
            }
        }
        for source in source_items {
            record(source.owner)?;
        }
        for stack in filter_stacks {
            record(stack.owner)?;
            for node in &stack.nodes {
                if let Some(mask) = node.mask {
                    record(mask)?;
                }
                for (_, input) in &node.auxiliary {
                    record(*input)?;
                }
            }
        }
        for reference in &selection.selected {
            record(*reference)?;
        }
        if let Some(active) = selection.active {
            record(active)?;
        }
        for reference in &selection.selected_paths {
            record(*reference)?;
        }
        if let Some(target) = selection.mask_target {
            record(target)?;
        }
        for set in item_sets {
            for reference in &set.members {
                record(*reference)?;
            }
        }
        for record_value in opaque {
            if let Some(owner) = record_value.owner {
                record(owner)?;
            }
        }
        if let Some(attachment) = floating {
            record(attachment.target)?;
        }
        let mut identities = std::collections::HashSet::new();
        identities.try_reserve(items.len()).map_err(|_| invalid("item_limit"))?;
        for item in items {
            let target = match item.item {
                ItemRef::Channel(id) => ItemRef::Channel(*planned.get(&id).ok_or_else(|| invalid("channel_reference"))?),
                other => other,
            };
            if !identities.insert(target) {
                return Err(invalid("item_identity"));
            }
        }
        // Planning covered every channel-bearing field. Commit uses cached
        // results only: no user callbacks, allocation or fallible validation.
        self.remap_references(|reference| {
            if let ItemRef::Channel(id) = reference
                && let Some(target) = planned.get(id)
            {
                *id = *target;
            }
            Ok(())
        })
    }
    /// Remap saved paths transactionally, including editable vector source dependencies.
    pub fn remap_paths(&mut self, mut map: impl FnMut(PathId) -> Option<PathId>) -> Result<()> {
        self.check_clone_bounds()?;
        // Plan against immutable references, including the vector source's
        // typed path dependency; do not clone its cache or resource payloads.
        let mut planned = std::collections::HashMap::<PathId, PathId>::new();
        let mut destinations = std::collections::HashSet::<PathId>::new();
        let mut record = |reference: ItemRef| -> Result<()> {
            let ItemRef::Path(id) = reference else { return Ok(()) };
            if id.0 == 0 {
                return Err(invalid("path_identity"));
            }
            if planned.contains_key(&id) {
                return Ok(());
            }
            // Reuse the canonical item/drawable ceiling, not a smaller new cap.
            if planned.len() >= 10000 {
                return Err(invalid("item_limit"));
            }
            let target = map(id).ok_or_else(|| invalid("path_reference"))?;
            if target.0 == 0 || destinations.contains(&target) {
                return Err(invalid("path_identity"));
            }
            planned.try_reserve(1).map_err(|_| invalid("item_limit"))?;
            destinations.try_reserve(1).map_err(|_| invalid("item_limit"))?;
            planned.insert(id, target);
            destinations.insert(target);
            Ok(())
        };
        let Self {
            items,
            source_items,
            filter_stacks,
            selection,
            item_sets,
            opaque,
            floating,
            provenance: _,
            resources: _,
            editable_indexed: _,
            current_filter_output_precision: _,
            filter_masks: _,
        } = &*self;
        for item in items {
            record(item.item)?;
            if let Some(parent) = item.parent {
                record(parent)?;
            }
            if let Some(mask) = &item.mask {
                record(mask.owner)?;
            }
        }
        for source in source_items {
            record(source.owner)?;
            if let EditableSource::Vector { path, .. } = &source.source {
                record(ItemRef::Path(*path))?;
            }
        }
        for stack in filter_stacks {
            record(stack.owner)?;
            for node in &stack.nodes {
                if let Some(mask) = node.mask {
                    record(mask)?;
                }
                for (_, input) in &node.auxiliary {
                    record(*input)?;
                }
            }
        }
        for reference in &selection.selected {
            record(*reference)?;
        }
        if let Some(active) = selection.active {
            record(active)?;
        }
        for reference in &selection.selected_paths {
            record(*reference)?;
        }
        if let Some(target) = selection.mask_target {
            record(target)?;
        }
        for set in item_sets {
            for reference in &set.members {
                record(*reference)?;
            }
        }
        for record_value in opaque {
            if let Some(owner) = record_value.owner {
                record(owner)?;
            }
        }
        if let Some(attachment) = floating {
            record(attachment.target)?;
        }
        let mut identities = std::collections::HashSet::new();
        identities.try_reserve(items.len()).map_err(|_| invalid("item_limit"))?;
        for item in items {
            let target = match item.item {
                ItemRef::Path(id) => ItemRef::Path(*planned.get(&id).ok_or_else(|| invalid("path_reference"))?),
                other => other,
            };
            if !identities.insert(target) {
                return Err(invalid("item_identity"));
            }
        }
        // The complete immutable census makes commit allocation/callback-free.
        self.remap_references(|reference| {
            if let ItemRef::Path(id) = reference
                && let Some(target) = planned.get(id)
            {
                *id = *target;
            }
            Ok(())
        })
    }
    pub fn remap_layers(&mut self, map: impl FnMut(LayerId) -> Option<LayerId>) -> Result<()> {
        self.remap_layers_with_interrupt(map, &photocraft_raster::Interrupt::NONE)
    }
    /// Cancellation is admitted through planning and the final immutable commit
    /// rehearsal. After its last check, the bounded allocation-free commit is
    /// indivisible: cancellation cannot return partially remapped identities.
    pub fn remap_layers_with_interrupt(&mut self, mut map: impl FnMut(LayerId) -> Option<LayerId>, control: &photocraft_raster::Interrupt<'_>) -> Result<()> {
        fn references(state: &XcfState, control: &photocraft_raster::Interrupt<'_>, mut visit: impl FnMut(ItemRef) -> Result<()>) -> Result<usize> {
            let XcfState { items: _, resources: _, source_items: _, editable_indexed: _, filter_stacks: _, current_filter_output_precision: _, filter_masks: _, floating: _, selection: _, item_sets: _, opaque: _, provenance: _ } = state;
            let mut work = 0usize;
            let mut charge = |count: usize| -> Result<()> {
                cancellation(control)?;
                work = work.checked_add(count).filter(|n| *n <= 100_000).ok_or_else(|| invalid("item_limit"))?;
                Ok(())
            };
            // Bound loops even when optional references are absent. Repeated
            // references consume work too; distinct IDs are not a work bound.
            charge(state.items.len())?;
            for item in &state.items {
                cancellation(control)?;
                visit(item.item)?;
                if let Some(parent) = item.parent { charge(1)?; visit(parent)?; }
                if let Some(mask) = &item.mask { charge(1)?; visit(mask.owner)?; }
            }
            charge(state.editable_indexed.len())?;
            for indexed in &state.editable_indexed { cancellation(control)?; visit(ItemRef::Layer(indexed.owner))?; }
            charge(state.source_items.len())?;
            for source in &state.source_items {
                cancellation(control)?;
                visit(source.owner)?;
                if let EditableSource::Vector { path, .. } = &source.source { charge(1)?; visit(ItemRef::Path(*path))?; }
            }
            charge(state.filter_stacks.len())?;
            for stack in &state.filter_stacks {
                cancellation(control)?;
                visit(stack.owner)?;
                charge(stack.nodes.len())?;
                for node in &stack.nodes {
                    cancellation(control)?;
                    if let Some(mask) = node.mask { charge(1)?; visit(mask)?; }
                    charge(node.auxiliary.len())?;
                    for (_, input) in &node.auxiliary { cancellation(control)?; visit(*input)?; }
                }
            }
            charge(state.selection.selected.len())?;
            for reference in &state.selection.selected { cancellation(control)?; visit(*reference)?; }
            if let Some(active) = state.selection.active { charge(1)?; visit(active)?; }
            charge(state.selection.selected_paths.len())?;
            for reference in &state.selection.selected_paths { cancellation(control)?; visit(*reference)?; }
            if let Some(target) = state.selection.mask_target { charge(1)?; visit(target)?; }
            charge(state.item_sets.len())?;
            for set in &state.item_sets {
                cancellation(control)?;
                charge(set.members.len())?;
                for reference in &set.members { cancellation(control)?; visit(*reference)?; }
            }
            charge(state.opaque.len())?;
            for value in &state.opaque { cancellation(control)?; if let Some(owner) = value.owner { visit(owner)?; } }
            if let Some(attachment) = &state.floating { charge(2)?; visit(attachment.target)?; visit(ItemRef::Layer(attachment.layer))?; }
            cancellation(control)?;
            Ok(work)
        }
        let positions = references(self, control, |_| Ok(()))?;
        self.check_clone_bounds_with(control)?;
        // Plan identities through immutable borrows: even the generic mutable
        // visitor assigns floating.layer, so it cannot be used for preflight.
        // Payload-bearing state is never cloned to make this transaction atomic.
        let mut planned = std::collections::HashMap::<LayerId, LayerId>::new();
        let mut destinations = std::collections::HashSet::<LayerId>::new();
        let mut identities = std::collections::HashSet::<ItemRef>::new();
        let maximum = positions.min(10_000);
        let envelope = maximum.checked_mul(96).and_then(|n| n.checked_add(self.items.len().checked_mul(64)?)).filter(|n| *n <= 2 << 20).ok_or_else(|| invalid("item_limit"))?;
        cancellation(control)?;
        planned.try_reserve(maximum).map_err(|_| invalid("item_limit"))?;
        destinations.try_reserve(maximum).map_err(|_| invalid("item_limit"))?;
        identities.try_reserve(self.items.len()).map_err(|_| invalid("item_limit"))?;
        let actual = planned.capacity().checked_mul(32).and_then(|n| n.checked_add(destinations.capacity().checked_mul(24)?)).and_then(|n| n.checked_add(identities.capacity().checked_mul(40)?)).ok_or_else(|| invalid("item_limit"))?;
        // Hash-table control bytes/slot padding are covered conservatively.
        if actual > 2 << 20 || actual > envelope.saturating_mul(2) { return Err(invalid("item_limit")); }
        let mut record = |reference: ItemRef| -> Result<()> {
            cancellation(control)?;
            let ItemRef::Layer(id) = reference else { return Ok(()) };
            if id.0 == 0 {
                return Err(invalid("layer_identity"));
            }
            if planned.contains_key(&id) {
                return Ok(());
            }
            // Canonical documents have at most 10000 distinct drawables;
            // repetitions remain subject to the separate aggregate work bound.
            if planned.len() >= 10000 {
                return Err(invalid("item_limit"));
            }
            let target = map(id).ok_or_else(|| invalid("layer_reference"))?;
            cancellation(control)?;
            if target.0 == 0 || destinations.contains(&target) {
                return Err(invalid("layer_identity"));
            }
            planned.insert(id, target);
            destinations.insert(target);
            Ok(())
        };
        references(self, control, &mut record)?;
        for item in &self.items {
            cancellation(control)?;
            let target = match item.item {
                ItemRef::Layer(id) => ItemRef::Layer(*planned.get(&id).ok_or_else(|| invalid("layer_reference"))?),
                other => other,
            };
            if !identities.insert(target) {
                return Err(invalid("item_identity"));
            }
        }
        // Rehearse the exact mutable visitor's positions, including repeats and
        // its vector-path/non-layer positions. Once this passes, commit contains
        // no user map calls, allocation, cancellation errors or fallible lookup.
        references(self, control, |reference| {
            if let ItemRef::Layer(id) = reference {
                planned.get(&id).filter(|target| target.0 != 0).ok_or_else(|| invalid("layer_reference"))?;
            }
            Ok(())
        })?;
        cancellation(control)?;
        // Every layer reference was planned above using the same field census.
        // No callbacks, allocation or fallible validation occur after writes start.
        self.remap_references(|reference| {
            if let ItemRef::Layer(id) = reference
                && let Some(target) = planned.get(id)
            {
                *id = *target;
            }
            Ok(())
        })
    }
    fn remap_references(&mut self, mut remap: impl FnMut(&mut ItemRef) -> Result<()>) -> Result<()> {
        for i in &mut self.items {
            remap(&mut i.item)?;
            if let Some(p) = &mut i.parent {
                remap(p)?
            }
            if let Some(m) = &mut i.mask {
                remap(&mut m.owner)?
            }
        }
        for indexed in &mut self.editable_indexed {
            let mut owner = ItemRef::Layer(indexed.owner);
            remap(&mut owner)?;
            if let ItemRef::Layer(id) = owner {
                indexed.owner = id;
            }
        }
        for s in &mut self.source_items {
            remap(&mut s.owner)?;
            if let EditableSource::Vector { path, .. } = &mut s.source {
                let mut reference = ItemRef::Path(*path);
                remap(&mut reference)?;
                if let ItemRef::Path(id) = reference {
                    *path = id;
                }
            }
        }
        for s in &mut self.filter_stacks {
            remap(&mut s.owner)?;
            for n in &mut s.nodes {
                if let Some(m) = &mut n.mask {
                    remap(m)?
                }
                for (_, a) in &mut n.auxiliary {
                    remap(a)?
                }
            }
        }
        for r in &mut self.selection.selected {
            remap(r)?
        }
        if let Some(a) = &mut self.selection.active {
            remap(a)?
        }
        for r in &mut self.selection.selected_paths {
            remap(r)?
        }
        if let Some(r) = &mut self.selection.mask_target {
            remap(r)?
        }
        for s in &mut self.item_sets {
            for r in &mut s.members {
                remap(r)?
            }
        }
        for o in &mut self.opaque {
            if let Some(r) = &mut o.owner {
                remap(r)?
            }
        }
        if let Some(f) = &mut self.floating {
            remap(&mut f.target)?;
            let mut r = ItemRef::Layer(f.layer);
            remap(&mut r)?;
            if let ItemRef::Layer(id) = r {
                f.layer = id
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "xcf_layer_remap_interrupt_tests.rs"]
mod remap_layers_interrupt_tests;
