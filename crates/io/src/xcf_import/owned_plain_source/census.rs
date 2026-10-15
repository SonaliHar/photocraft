//! Allocation-free closed-model retained capacity census. Shared owners are
//! conservatively counted per reference; no identity table allocation or claim
//! of allocator/RSS measurement. Surface maps use the staged builder envelope.
use super::*;
use std::mem::size_of;
#[derive(Default)]
pub(in crate::xcf_import) struct Census {
    pub model: u64,
    pub payload: u64,
    pub layers: usize,
}
struct Walk<'b, 'a> {
    out: Census,
    budget: &'b photocraft_xcf::Budget<'a>,
    visits: usize,
}
impl Walk<'_, '_> {
    fn add(&mut self, n: usize) -> Result<(), IoError> {
        self.out.model = add(self.out.model, n as u64)?;
        Ok(())
    }
    fn visit(&mut self) -> Result<(), IoError> {
        self.visits = self.visits.checked_add(1).ok_or_else(|| invalid("owned model visit overflow"))?;
        if self.visits > 100_000 {
            return Err(invalid("owned model visit ceiling"));
        }
        self.budget.consume_work(1, "owned XCF model census").map_err(accounting_error)
    }
    fn vector<T>(&mut self, v: &Vec<T>) -> Result<(), IoError> {
        self.visit()?;
        if v.len() > 100_000 {
            return Err(invalid("owned model collection ceiling"));
        }
        self.add(v.capacity().checked_mul(size_of::<T>()).ok_or_else(|| invalid("owned vector capacity"))?)
    }
    fn string(&mut self, v: &String) -> Result<(), IoError> {
        self.visit()?;
        if v.len() > 1 << 20 {
            return Err(invalid("owned string ceiling"));
        }
        self.add(v.capacity())?;
        for part in v.as_bytes().chunks(4096) {
            self.budget.consume_work(part.len() as u64, "owned XCF text census").map_err(accounting_error)?;
        }
        Ok(())
    }
    fn optional(&mut self, v: &Option<String>) -> Result<(), IoError> {
        if let Some(v) = v {
            self.string(v)?;
        }
        Ok(())
    }
    fn blob(&mut self, v: &Arc<Vec<u8>>) -> Result<(), IoError> {
        self.visit()?;
        self.out.payload = add(self.out.payload, add(v.capacity() as u64, (size_of::<Vec<u8>>() + 128) as u64)?)?;
        for part in v.chunks(4096) {
            self.budget.consume_work(part.len() as u64, "owned XCF raw owner census").map_err(accounting_error)?;
        }
        Ok(())
    }
    fn arguments(&mut self, arguments: &Vec<photocraft_doc::xcf_filters::FilterArgument>, depth: usize) -> Result<(), IoError> {
        use photocraft_doc::xcf_filters::{FilterArgument, FilterValue};
        if depth > 2 {
            return Err(invalid("owned Point argument depth"));
        }
        self.vector(arguments)?;
        for FilterArgument { name, value } in arguments {
            self.string(name)?;
            match value {
                FilterValue::Enum { domain, .. } => self.string(domain)?,
                FilterValue::Color { components, .. } => self.vector(components)?,
                FilterValue::Config(fields) => self.arguments(fields, depth + 1)?,
                FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } => {
                    self.vector(points_f64_bits)?;
                    self.vector(point_types)?;
                    self.vector(samples_f64_bits)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn composition(&mut self, v: &Option<photocraft_doc::import_compositing::GimpComposition>) -> Result<(), IoError> {
        if let Some(v) = v {
            self.string(&v.compatibility_epoch)?;
        }
        Ok(())
    }
}
pub(super) fn measure(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<Census, IoError> {
    measure_inner(doc, budget, control, false, false, false)
}
/// Only the sealed staged Threshold Alpha producer may call this sibling seam.
/// Its original source/Surface receipts remain mandatory; this measures storage.
pub(in crate::xcf_import) fn measure_threshold(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<Census, IoError> {
    threshold_shape(doc)?;
    measure_inner(doc, budget, control, true, false, false)
}
/// Closed Histogram Threshold source promotion only; not a caller source proof.
pub(in crate::xcf_import) fn measure_histogram_threshold(
    doc: &Document,
    budget: &photocraft_xcf::Budget<'_>,
    control: &Interrupt<'_>,
) -> Result<Census, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.consume_work(32, "histogram threshold census shape").map_err(accounting_error)?;
    histogram_threshold_shape(doc)?;
    measure_inner(doc, budget, control, true, false, false)
}
/// Storage-only shared walker, reached after the sealed Point shape gate.
pub(in crate::xcf_import) fn measure_point_storage(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<Census, IoError> {
    measure_inner(doc, budget, control, true, false, false)
}
fn measure_inner(
    doc: &Document,
    budget: &photocraft_xcf::Budget<'_>,
    control: &Interrupt<'_>,
    threshold: bool,
    geometry: bool,
    isolated_group: bool,
) -> Result<Census, IoError> {
    use photocraft_doc::{WorkingProfileInterpretation, physical_resolution::PhysicalUnit, xcf_editing::ItemSetRule};
    control.check().map_err(|_| IoError::Cancelled)?;
    let Document {
        physical_resolution: _,
        xcf_state: _,
        sample_encoding: _,
        source_color_context: _,
        working_profile_interpretation: _,
        id: _,
        name: _,
        size: _,
        resolution_dpi: _,
        mode: _,
        depth: _,
        icc_profile: _,
        layers: _,
        channels: _,
        guides: _,
        selection: _,
        selection_channel_id: _,
        metadata: _,
        global_light: _,
        paths: _,
        work_path: _,
        clipping_path: _,
        quick_mask: _,
        quick_mask_unselected: _,
        patterns: _,
        color_table: _,
        duotone: _,
        layer_comps: _,
        last_applied_comp: _,
        last_document_state: _,
        measurement: _,
        notes: _,
        text_styles: _,
        slices: _,
        variables: _,
        timeline: _,
        file_encoding: _,
        ruler_origin: _,
    } = doc;
    if doc.layers.is_empty()
        || doc.layers.len() > 4096
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || !doc.patterns.is_empty()
        || doc.color_table.is_some()
        || doc.duotone.is_some()
        || !doc.layer_comps.is_empty()
        || doc.last_document_state.is_some()
        || !doc.notes.is_empty()
        || doc.timeline.is_some()
        || !doc.measurement.count_groups.is_empty()
        || !doc.text_styles.character.is_empty()
        || !doc.text_styles.paragraph.is_empty()
        || !doc.text_styles.basic.char_attrs.is_empty()
        || !doc.text_styles.basic.para_attrs.is_empty()
        || doc.text_styles.psd_raw.is_some()
        || !doc.slices.list.is_empty()
        || !doc.variables.defs.is_empty()
        || !doc.variables.data_sets.is_empty()
    {
        return Err(invalid("owned plain XCF model family is unsupported"));
    }
    let mut w = Walk { out: Census::default(), budget, visits: 0 };
    w.add(size_of::<Document>())?;
    w.string(&doc.name)?;
    w.vector(&doc.layers)?;
    w.vector(&doc.channels)?;
    w.vector(&doc.paths)?;
    w.vector(&doc.patterns)?;
    w.vector(&doc.layer_comps)?;
    w.vector(&doc.notes)?;
    w.vector(&doc.measurement.count_groups)?;
    w.string(&doc.measurement.scale.units)?;
    w.vector(&doc.text_styles.character)?;
    w.vector(&doc.text_styles.paragraph)?;
    w.string(&doc.text_styles.basic.name)?;
    w.vector(&doc.slices.list)?;
    w.string(&doc.slices.group_name)?;
    w.vector(&doc.variables.defs)?;
    w.vector(&doc.variables.data_sets)?;
    if let Some(p) = &doc.physical_resolution {
        if let PhysicalUnit::Custom(v) = &p.unit {
            w.string(&v.name)?;
            w.string(&v.symbol)?;
            w.string(&v.abbreviation)?;
        }
    }
    match &doc.working_profile_interpretation {
        WorkingProfileInterpretation::Icc => {}
        WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } | WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } => w.string(icc_blake3)?,
    }
    if let Some(v) = &doc.icc_profile {
        w.blob(v)?;
    }
    if let Some(v) = doc.source_color_context.as_ref().and_then(|v| v.original_icc.as_ref()) {
        w.blob(v)?;
    }
    w.optional(&doc.metadata.xmp)?;
    if let Some(v) = &doc.metadata.exif {
        w.blob(v)?;
    }
    w.vector(&doc.metadata.text)?;
    for (k, v) in &doc.metadata.text {
        w.string(k)?;
        w.string(v)?;
    }
    w.vector(&doc.metadata.psd_resources)?;
    for (_, name, v) in &doc.metadata.psd_resources {
        w.string(name)?;
        w.blob(v)?;
    }
    w.vector(&doc.metadata.psd_global_blocks)?;
    for (_, _, v) in &doc.metadata.psd_global_blocks {
        w.blob(v)?;
    }
    w.vector(&doc.guides.horizontal)?;
    w.vector(&doc.guides.vertical)?;
    w.vector(&doc.guides.colors)?;
    // Only the dedicated shape-checked group entry supplies these two children.
    // This is a bounded one-level walk, not recursive group enrollment.
    let children = if isolated_group {
        let LayerContent::Group(group) = &doc.layers.get(1).ok_or_else(|| invalid("group census root"))?.content else {
            return Err(invalid("group census content"));
        };
        w.vector(&group.children)?;
        group.children.as_slice()
    } else {
        &[]
    };
    for layer in doc.layers.iter().chain(children.iter()) {
        if isolated_group {
            control.check().map_err(|_| IoError::Cancelled)?;
        }
        w.visit()?;
        if layer.mask.is_some() && !(geometry || isolated_group)
            || layer.vector_mask.is_some()
            || layer.indexed_source.is_some()
            || !layer.effects.items.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
        {
            return Err(invalid("owned plain layer family unsupported"));
        }
        let surface = match &layer.content {
            LayerContent::Raster(surface) => Some(surface),
            LayerContent::Group(_) if isolated_group && layer.id == doc.layers.get(1).ok_or_else(|| invalid("group census root"))?.id => None,
            _ => return Err(invalid("owned plain layer requires genuine staged Surface")),
        };
        if geometry || isolated_group {
            if let Some(mask) = &layer.mask {
                w.visit()?;
                if isolated_group {
                    // Native payload stays in the original Sink receipts. Visit
                    // every retained map entry, including padded/default tiles.
                    for _ in mask.surface.tiles() {
                        control.check().map_err(|_| IoError::Cancelled)?;
                        w.visit()?;
                    }
                }
                w.add(
                    size_of::<LayerMask>()
                        .checked_add(size_of::<Surface>() + 128)
                        .and_then(|n| mask.surface.tile_count().checked_mul(256).and_then(|m| n.checked_add(m)))
                        .and_then(|n| mask.surface.default_bytes().len().checked_mul(2).and_then(|d| n.checked_add(d)))
                        .ok_or_else(|| invalid("geometry mask metadata"))?,
                )?;
            }
        }
        w.string(&layer.name)?;
        w.vector(&layer.effects.items)?;
        if let Some(v) = &layer.effects.psd_raw {
            w.blob(v)?;
        }
        w.vector(&layer.psd_blocks)?;
        for (_, v) in &layer.psd_blocks {
            w.blob(v)?;
        }
        w.vector(&layer.blend_if.ranges)?;
        // Native payload remains covered by each genuine staged Surface lease;
        // additionally retain its real map/header envelope in the index subcap.
        if let Some(surface) = surface {
            w.add(surface.tile_count().checked_mul(256).ok_or_else(|| invalid("owned Surface map count"))?)?;
            w.out.layers = w.out.layers.checked_add(1).ok_or_else(|| invalid("owned native store count"))?;
        }
    }
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("missing owned XCF state"))?;
    let photocraft_doc::XcfState {
        provenance: _,
        items: _,
        resources: _,
        source_items: _,
        editable_indexed: _,
        filter_stacks: _,
        current_filter_output_precision: _,
        filter_masks: _,
        floating: _,
        selection: _,
        item_sets: _,
        opaque: _,
    } = state;
    if !state.source_items.is_empty()
        || (!threshold && !state.filter_stacks.is_empty())
        || (!threshold && !state.filter_masks.is_empty())
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
    {
        return Err(invalid("owned plain source state family unsupported"));
    }
    let p = &state.provenance;
    w.string(&p.format)?;
    w.string(&p.container)?;
    w.optional(&p.producer)?;
    w.optional(&p.release_id)?;
    w.string(&p.source_label)?;
    w.optional(&p.source_hash)?;
    w.optional(&p.original_locator)?;
    w.vector(&p.protected_source_fingerprints)?;
    for v in &p.protected_source_fingerprints {
        w.string(v)?;
    }
    w.vector(&state.items)?;
    for item in &state.items {
        w.string(&item.name)?;
        w.composition(&item.composition)?;
        if !threshold && !geometry && !isolated_group && item.mask.is_some() {
            return Err(invalid("owned plain item mask unsupported"));
        }
    }
    w.vector(&state.resources)?;
    for r in &state.resources {
        w.string(&r.name)?;
        w.optional(&r.collection)?;
        w.optional(&r.locator)?;
        w.optional(&r.content_hash)?;
        if let Some(v) = &r.owned_bytes {
            w.blob(v)?;
        }
        if let Some(r) = &r.resolution {
            w.string(&r.payload_hash)?;
        }
    }
    w.vector(&state.source_items)?;
    w.vector(&state.filter_stacks)?;
    w.vector(&state.filter_masks)?;
    if threshold {
        threshold_storage(state, &mut w, control)?;
    }
    w.vector(&state.editable_indexed)?;
    w.vector(&state.selection.selected)?;
    w.vector(&state.selection.selected_paths)?;
    w.vector(&state.item_sets)?;
    for set in &state.item_sets {
        w.string(&set.name)?;
        w.vector(&set.members)?;
        match &set.rule {
            ItemSetRule::Named => {}
            ItemSetRule::Text(v) | ItemSetRule::Regex(v) | ItemSetRule::Glob(v) => w.string(v)?,
        }
    }
    w.vector(&state.opaque)?;
    for raw in &state.opaque {
        w.string(&raw.namespace)?;
        w.string(&raw.identity)?;
        w.string(&raw.blake3)?;
        w.blob(&raw.bytes)?;
    }
    control.check().map_err(|_| IoError::Cancelled)?;
    Ok(w.out)
}

pub(in crate::xcf_import) fn reports(result: &ImportResult, budget: &photocraft_xcf::Budget<'_>) -> Result<u64, IoError> {
    let mut w = Walk { out: Census::default(), budget, visits: 0 };
    w.vector(&result.warnings)?;
    for v in &result.warnings {
        w.string(v)?;
    }
    if let Some(a) = &result.assessment {
        w.string(&a.container)?;
        w.vector(&a.dependencies)?;
        w.vector(&a.diagnostics)?;
        for v in &a.dependencies {
            w.string(&v.resource_id)?;
        }
        for v in &a.diagnostics {
            w.string(&v.resource_id)?;
            w.vector(&v.recovery_actions)?;
        }
    }
    Ok(w.out.model)
}

fn threshold_shape(doc: &Document) -> Result<(), IoError> {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("threshold census state"))?;
    if doc.layers.len() != 1
        || doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 262144
        || doc.size.height > 262144
        || u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).is_none_or(|n| n > 1 << 30)
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() > 1
        || state.items.len() != 1 + state.filter_masks.len()
        || state.filter_masks.iter().any(|mask| mask.bounds != doc.bounds())
    {
        return Err(invalid("threshold census closed storage shape"));
    }
    let stack = state.filter_stacks.first().ok_or_else(|| invalid("threshold census stack"))?;
    let node = stack.nodes.first().ok_or_else(|| invalid("threshold census node"))?;
    if stack.nodes.len() != 1
        || node.operation.name != "gimp:threshold-alpha"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf-22-stock-threshold-alpha-precision-v1"
        || node.arguments.len() != 1
        || !node.auxiliary.is_empty()
    {
        return Err(invalid("threshold census canonical node"));
    }
    let argument = node.arguments.first().ok_or_else(|| invalid("threshold census argument"))?;
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else {
        return Err(invalid("threshold census scalar storage"));
    };
    if argument.name != "value" || !f32::from_bits(bits).is_finite() || !(0.0..=1.0).contains(&f32::from_bits(bits)) {
        return Err(invalid("threshold census scalar domain"));
    }
    Ok(())
}
fn histogram_threshold_shape(doc: &Document) -> Result<(), IoError> {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("histogram threshold census state"))?;
    if doc.size != Size::new(8, 6)
        || doc.mode != ColorMode::Rgb
        || doc.sample_encoding != Some(photocraft_doc::SampleEncoding::NonlinearProfile)
        || !matches!(
            (doc.source_color_context.as_ref().map(|v| v.source_precision), doc.depth),
            (Some(250), SampleType::U16) | (Some(650), SampleType::F32) | (Some(750), SampleType::F64)
        )
        || doc.layers.len() != 1
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() != 1
        || state.items.len() != 2
        || state.filter_masks.iter().any(|mask| mask.bounds != doc.bounds())
    {
        return Err(invalid("histogram threshold census closed storage shape"));
    }
    let node = state
        .filter_stacks
        .first()
        .and_then(|s| (s.nodes.len() == 1).then(|| s.nodes.first()).flatten())
        .ok_or_else(|| invalid("histogram threshold census node"))?;
    if node.operation.name != "gimp:threshold"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != "xcf-22-stock-threshold-precision-v1"
        || node.arguments.len() != 3
        || !node.auxiliary.is_empty()
    {
        return Err(invalid("histogram threshold census canonical node"));
    }
    let mut seen = 0u8;
    for a in &node.arguments {
        let flag = match (a.name.as_str(), &a.value) {
            ("channel", FilterValue::Enum { domain, value }) if domain == "GimpHistogramChannel" && (0..=5).contains(value) => 1,
            (name @ ("low" | "high"), FilterValue::Number(ExactNumber::Float32Bits(bits)))
                if f32::from_bits(*bits).is_finite() && (0.0..=1.0).contains(&f32::from_bits(*bits)) =>
            {
                if name == "low" {
                    2
                } else {
                    4
                }
            }
            _ => return Err(invalid("histogram threshold census argument")),
        };
        if seen & flag != 0 {
            return Err(invalid("histogram threshold census duplicate argument"));
        }
        seen |= flag;
    }
    if seen != 7 {
        return Err(invalid("histogram threshold census missing argument"));
    }
    Ok(())
}
fn threshold_storage(state: &photocraft_doc::XcfState, w: &mut Walk<'_, '_>, control: &Interrupt<'_>) -> Result<(), IoError> {
    use photocraft_doc::xcf_filters::{FilterNode, FilterStack, OperationIdentity};
    for FilterStack { owner: _, nodes, revision: _ } in &state.filter_stacks {
        w.vector(nodes)?;
        for FilterNode {
            id: _,
            operation: OperationIdentity { name, stored_version, schema_epoch },
            source_display_name,
            source_icon,
            arguments,
            enabled: _,
            opacity_f64_bits: _,
            source_clipping: _,
            source_region: _,
            mask: _,
            auxiliary,
            composition,
        } in nodes
        {
            w.string(name)?;
            w.string(stored_version)?;
            w.string(schema_epoch)?;
            w.optional(source_display_name)?;
            w.optional(source_icon)?;
            // The separate Point shape validates the shared bounded tree.
            // Threshold shapes still only admit their original inline values.
            w.arguments(arguments, 0)?;
            w.vector(auxiliary)?;
            w.composition(composition)?;
        }
    }
    for photocraft_doc::FilterMask { id: _, owner: _, surface, bounds } in &state.filter_masks {
        control.check().map_err(|_| IoError::Cancelled)?;
        w.visit()?;
        if bounds.x0 != 0 || bounds.y0 != 0 || bounds.is_empty() || bounds.width() > 262144 || bounds.height() > 262144 {
            return Err(invalid("threshold census mask bounds"));
        }
        let pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(|| invalid("threshold mask pixels"))?;
        let columns = u64::from(bounds.width()).checked_add(255).ok_or_else(|| invalid("threshold mask columns"))? / 256;
        let rows = u64::from(bounds.height()).checked_add(255).ok_or_else(|| invalid("threshold mask rows"))? / 256;
        let grid = columns.checked_mul(rows).ok_or_else(|| invalid("threshold mask grid"))?;
        if pixels > 1 << 30 || u64::try_from(surface.tile_count()).ok().is_none_or(|n| n > grid) {
            return Err(invalid("threshold census mask tile count"));
        }
        let tile_bytes = 65536usize.checked_mul(surface.format().bytes_per_pixel()).ok_or_else(|| invalid("threshold mask tile bytes"))?;
        for (coordinate, tile) in surface.tiles() {
            control.check().map_err(|_| IoError::Cancelled)?;
            w.visit()?;
            if coordinate.tx < 0 || coordinate.ty < 0 || coordinate.tx as u64 >= columns || coordinate.ty as u64 >= rows {
                return Err(invalid("threshold mask sparse coordinate"));
            }
            let x = coordinate.tx.checked_mul(256).ok_or_else(|| invalid("threshold mask tile origin"))?;
            let y = coordinate.ty.checked_mul(256).ok_or_else(|| invalid("threshold mask tile origin"))?;
            let right = x.checked_add(256).ok_or_else(|| invalid("threshold mask tile edge"))?;
            let bottom = y.checked_add(256).ok_or_else(|| invalid("threshold mask tile edge"))?;
            if [x, y, right, bottom].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v)) || tile.bytes().len() != tile_bytes {
                return Err(invalid("threshold mask native tile structure"));
            }
        }
        // Inline Surface is counted by FilterMask Vec capacity. Actual native
        // Tile/Box/default storage also remains with the original Sink receipts.
        w.add(surface.default_bytes().len())?;
        w.add(surface.tile_count().checked_mul(256).ok_or_else(|| invalid("threshold mask map envelope"))?)?;
    }
    // MaskState has only inline scalar/identity fields and is included in the
    // actual ItemState Vec capacity. Destructure it to detect future heap fields.
    for item in &state.items {
        if let Some(photocraft_doc::xcf_state::MaskState { owner: _, bounds: _, offset: _, outside_f64_bits: _, enabled: _, shown: _, editing: _, linked: _ }) =
            &item.mask
        {}
    }
    Ok(())
}

/// Only the sealed original analytic geometry promotion reaches this walker.
pub(in crate::xcf_import) fn measure_analytic_geometry(
    doc: &Document,
    budget: &photocraft_xcf::Budget<'_>,
    control: &Interrupt<'_>,
) -> Result<Census, IoError> {
    if doc.layers.len() != 2
        || doc.xcf_state.as_ref().is_none_or(|s| s.items.len() != 3 || !s.filter_stacks.is_empty() || !s.filter_masks.is_empty())
        || doc.layers.iter().filter(|l| l.mask.is_some()).count() != 1
    {
        return Err(invalid("analytic geometry census shape"));
    }
    measure_inner(doc, budget, control, false, true, false)
}

/// Exact one-level isolated Normal group storage census. The sealed source
/// producer and original decode/Surface receipts remain mandatory; this is not
/// a group source, profile or rendering qualification.
pub(in crate::xcf_import) fn measure_isolated_normal_group(
    doc: &Document,
    budget: &photocraft_xcf::Budget<'_>,
    control: &Interrupt<'_>,
) -> Result<Census, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.consume_work(16, "isolated group census shape").map_err(accounting_error)?;
    let bottom = doc.layers.first().ok_or_else(|| invalid("group census bottom"))?;
    let top = doc.layers.get(1).ok_or_else(|| invalid("group census top"))?;
    let LayerContent::Group(group) = &top.content else {
        return Err(invalid("group census root content"));
    };
    if doc.layers.len() != 2
        || !matches!(bottom.content, LayerContent::Raster(_))
        || bottom.mask.is_some()
        || group.children.len() != 2
        || group.artboard.is_some()
        || doc
            .xcf_state
            .as_ref()
            .is_none_or(|state| state.items.len() != 4 + usize::from(top.mask.is_some()) || !state.filter_stacks.is_empty() || !state.filter_masks.is_empty())
    {
        return Err(invalid("isolated group census shape"));
    }
    for child in &group.children {
        control.check().map_err(|_| IoError::Cancelled)?;
        budget.consume_work(1, "isolated group child census shape").map_err(accounting_error)?;
        if !matches!(child.content, LayerContent::Raster(_)) || child.mask.is_some() {
            return Err(invalid("isolated group census child"));
        }
    }
    measure_inner(doc, budget, control, false, false, true)
}

#[cfg(test)]
#[path = "census_group_tests.rs"]
mod group_tests;

/// Fixed one-level Gaussian group storage only. Eligibility belongs to the
/// sealed original-source selector and the independent CURRENT renderer.
pub(in crate::xcf_import) fn measure_isolated_gaussian_group(
    doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>,
) -> Result<Census, IoError> {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    control.check().map_err(|_| IoError::Cancelled)?;
    budget.consume_work(32, "Gaussian group census shape").map_err(accounting_error)?;
    let bottom = doc.layers.first().ok_or_else(|| invalid("Gaussian group census bottom"))?;
    let top = doc.layers.get(1).ok_or_else(|| invalid("Gaussian group census top"))?;
    let LayerContent::Group(group) = &top.content else { return Err(invalid("Gaussian group census tree")) };
    let state = doc.xcf_state.as_ref().ok_or_else(|| invalid("Gaussian group census state"))?;
    if doc.layers.len() != 2 || doc.size != Size::new(16, 12) || doc.mode != ColorMode::Rgb || doc.depth != SampleType::F32
        || !matches!(bottom.content, LayerContent::Raster(_)) || bottom.mask.is_some() || top.mask.is_none()
        || group.children.len() != 2 || group.artboard.is_some()
        || state.items.len() != 6 || state.filter_stacks.len() != 1 || state.filter_masks.len() != 1
    { return Err(invalid("Gaussian group census closed shape")); }
    for child in &group.children {
        control.check().map_err(|_| IoError::Cancelled)?;
        budget.consume_work(1, "Gaussian group child census").map_err(accounting_error)?;
        if !matches!(child.content, LayerContent::Raster(_)) || child.mask.is_some() {
            return Err(invalid("Gaussian group census leaf"));
        }
    }
    let stack = state.filter_stacks.first().ok_or_else(|| invalid("Gaussian group census stack"))?;
    let node = stack.nodes.first().ok_or_else(|| invalid("Gaussian group census node"))?;
    if stack.owner != photocraft_doc::item_identity::ItemRef::Layer(top.id) || stack.nodes.len() != 1
        || node.arguments.len() != 5 || !node.auxiliary.is_empty()
        || state.filter_masks.iter().any(|mask| mask.bounds != doc.bounds())
    { return Err(invalid("Gaussian group census filter shape")); }
    for argument in &node.arguments {
        control.check().map_err(|_| IoError::Cancelled)?;
        budget.consume_work(1, "Gaussian group argument census").map_err(accounting_error)?;
        if !matches!(&argument.value, FilterValue::Number(ExactNumber::Float32Bits(_)) | FilterValue::Bool(_) | FilterValue::Enum { .. }) {
            return Err(invalid("Gaussian group census flat argument storage"));
        }
    }
    // Existing threshold storage is the bounded flat-node/filter-mask census;
    // existing isolated storage is the four-header/three-store tree census.
    // Their original entry points keep their original flags and shape gates.
    measure_inner(doc, budget, control, true, false, true)
}
