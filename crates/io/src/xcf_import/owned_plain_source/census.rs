//! Allocation-free closed-model retained capacity census. Shared owners are
//! conservatively counted per reference; no identity table allocation or claim
//! of allocator/RSS measurement. Surface maps use the staged builder envelope.
use super::*;
use std::mem::size_of;
#[derive(Default)]
pub(super) struct Census {
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
    fn composition(&mut self, v: &Option<photocraft_doc::import_compositing::GimpComposition>) -> Result<(), IoError> {
        if let Some(v) = v {
            self.string(&v.compatibility_epoch)?;
        }
        Ok(())
    }
}
pub(super) fn measure(doc: &Document, budget: &photocraft_xcf::Budget<'_>, control: &Interrupt<'_>) -> Result<Census, IoError> {
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
    for layer in &doc.layers {
        w.visit()?;
        if layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.indexed_source.is_some()
            || !layer.effects.items.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
        {
            return Err(invalid("owned plain layer family unsupported"));
        }
        let LayerContent::Raster(surface) = &layer.content else {
            return Err(invalid("owned plain layer requires genuine staged Surface"));
        };
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
        w.add(surface.tile_count().checked_mul(256).ok_or_else(|| invalid("owned Surface map count"))?)?;
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
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
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
        if item.mask.is_some() {
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
    w.out.layers = doc.layers.len();
    control.check().map_err(|_| IoError::Cancelled)?;
    Ok(w.out)
}

pub(super) fn reports(result: &ImportResult, budget: &photocraft_xcf::Budget<'_>) -> Result<u64, IoError> {
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
