//! Typed current saved-channel -> existing selection storage replacement.
//! Only current geometry changes; original provenance, tattoos, raw records and sets stay intact.
use crate::{DocumentSnapshot, EngineError, Result};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{
    Document,
    item_identity::{ChannelId, ItemRef},
    xcf_state::ItemState,
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
const CMD: &str = "select.loadSelection";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
#[derive(Clone, Copy)]
pub(in crate::channel_cmds) struct LoadProof {
    source_document: photocraft_doc::DocId,
    target_document: photocraft_doc::DocId,
    source: ChannelId,
    target: ChannelId,
    bounds: Rect,
    old_bounds: Rect,
    format: PixelFormat,
}
fn plain_metadata(metadata: &ItemState) -> bool {
    metadata.source_flags == 0 && metadata.parent.is_none() && metadata.mask.is_none() && metadata.composition.is_none()
}
fn item<'a>(doc: &'a Document, id: ChannelId, write: bool, ctl: &Interrupt<'_>) -> Result<&'a ItemState> {
    poll(ctl)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("raw canonical transfer requires typed source and target state"))?;
    if id.0 == 0 {
        return Err(bad("zero canonical channel identity"));
    }
    let reference = ItemRef::Channel(id);
    if state.floating.is_some() {
        return Err(bad("floating canonical state is not a raw storage transfer"));
    }
    let mut count = state
        .items
        .len()
        .checked_add(state.source_items.len())
        .and_then(|n| n.checked_add(state.filter_stacks.len()))
        .and_then(|n| n.checked_add(state.filter_masks.len()))
        .and_then(|n| n.checked_add(state.opaque.len()))
        .filter(|n| *n <= 100000)
        .ok_or_else(|| bad("canonical storage count limit"))?;
    let mut found = None;
    for metadata in &state.items {
        poll(ctl)?;
        if metadata.item == reference && found.replace(metadata).is_some() {
            return Err(bad("duplicate canonical storage owner"));
        }
    }
    let metadata = found.ok_or_else(|| bad("missing canonical storage owner"))?;
    if !plain_metadata(metadata) {
        return Err(bad("canonical channel flags/relationships are not a raw storage context"));
    }
    let bounds = metadata.raster_bounds.filter(|r| !r.is_empty()).ok_or_else(|| bad("missing canonical channel extent"))?;
    photocraft_raster::checked_region_byte_len(bounds, PixelFormat::new(ColorMode::Grayscale, doc.depth, false))?;
    for source in &state.source_items {
        poll(ctl)?;
        if source.owner == reference {
            return Err(bad("immutable source owner is not a raw channel transfer"));
        }
    }
    for stack in &state.filter_stacks {
        poll(ctl)?;
        if stack.owner == reference {
            return Err(bad("filtered current channel is not a raw storage context"));
        }
        count = count.checked_add(stack.nodes.len()).filter(|n| *n <= 100000).ok_or_else(|| bad("filter dependency count limit"))?;
        for node in &stack.nodes {
            poll(ctl)?;
            count = count.checked_add(node.auxiliary.len()).filter(|n| *n <= 100000).ok_or_else(|| bad("auxiliary dependency count limit"))?;
            if write && (node.mask == Some(reference) || node.auxiliary.iter().any(|(_, owner)| *owner == reference)) {
                return Err(bad("selection is an operative filter input"));
            }
        }
    }
    for record in &state.opaque {
        poll(ctl)?;
        if record.rendering_required && (record.owner == Some(reference) || record.owner.is_none()) {
            return Err(bad("opaque rendering authority is not a raw selection context"));
        }
    }
    poll(ctl)?;
    Ok(metadata)
}
pub(super) fn prepare(source: &Document, target: &Document, source_id: ChannelId, ctl: &Interrupt<'_>) -> Result<LoadProof> {
    poll(ctl)?;
    if source.size != target.size {
        return Err(bad("raw canonical transfer canvas mismatch"));
    }
    if source.channels.len() > 10000 || target.channels.len() > 10000 {
        return Err(bad("channel identity census limit"));
    }
    let mut selected = None;
    for channel in &source.channels {
        poll(ctl)?;
        if channel.id == source_id && selected.replace(channel).is_some() {
            return Err(bad("duplicate source channel identity"));
        }
    }
    let selected = selected.ok_or_else(|| bad("missing source channel"))?;
    if source.selection_channel_id == Some(source_id) || source.quick_mask.as_ref().is_some_and(|channel| channel.id == source_id) {
        return Err(bad("source saved identity aliases another channel role"));
    }
    if selected.spot.is_some() {
        return Err(bad("spot ink is not a raw selection source"));
    }
    let target_id = target.selection_channel_id.ok_or_else(|| bad("raw canonical Load needs an existing selection identity"))?;
    for channel in &target.channels {
        poll(ctl)?;
        if channel.id == target_id {
            return Err(bad("selection identity aliases a saved channel"));
        }
    }
    if target.quick_mask.as_ref().is_some_and(|channel| channel.id == target_id) {
        return Err(bad("selection identity aliases Quick Mask"));
    }
    let current = target.selection.as_ref().ok_or_else(|| bad("raw canonical Load needs existing selection storage"))?;
    let format = selected.surface.format();
    if format.mode != ColorMode::Grayscale || format.alpha || format.sample != source.depth || format.sample != target.depth || current.format() != format {
        return Err(bad("canonical native Gray depth mismatch"));
    }
    let origin = item(source, source_id, false, ctl)?;
    let destination = item(target, target_id, true, ctl)?;
    let bounds = origin.raster_bounds.ok_or_else(|| bad("missing source extent"))?;
    let old_bounds = destination.raster_bounds.ok_or_else(|| bad("missing target extent"))?;
    let locks = crate::xcf_item_locks::effective_item_locks(target, ItemRef::Channel(target_id))?;
    locks.require(crate::xcf_item_locks::EditKind::Content)?;
    if bounds != old_bounds {
        locks.require(crate::xcf_item_locks::EditKind::Position)?;
    }
    // Color contexts remain untouched. Gray storage is never interpreted in either RGB profile.
    source.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
    target.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
    poll(ctl)?;
    Ok(LoadProof { source_document: source.id, target_document: target.id, source: source_id, target: target_id, bounds, old_bounds, format })
}
impl LoadProof {
    pub(super) fn changes_bounds(&self) -> bool {
        self.bounds != self.old_bounds
    }
    pub(in crate::channel_cmds) fn check_copy(
        &self,
        source: &DocumentSnapshot,
        target: &DocumentSnapshot,
        surface: &Surface,
        format: PixelFormat,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        poll(ctl)?;
        if source.id != self.source_document || target.id != self.target_document || format != self.format {
            return Err(bad("canonical transfer proof target mismatch"));
        }
        let mut found = None;
        for channel in &source.channels {
            poll(ctl)?;
            if channel.id == self.source && found.replace(&channel.surface).is_some() {
                return Err(bad("duplicate proof source"));
            }
        }
        if !found.is_some_and(|saved| std::ptr::eq(saved, surface)) {
            return Err(bad("canonical transfer proof storage mismatch"));
        }
        if target.selection_channel_id != Some(self.target) {
            return Err(bad("canonical transfer proof selection mismatch"));
        }
        poll(ctl)
    }
    pub(super) fn apply_bounds(self, doc: &mut Document, ctl: &Interrupt<'_>) -> Result<()> {
        poll(ctl)?;
        if doc.id != self.target_document
            || doc.selection_channel_id != Some(self.target)
            || doc.selection.as_ref().is_none_or(|surface| surface.format() != self.format)
        {
            return Err(bad("selection publication target mismatch"));
        }
        let reference = ItemRef::Channel(self.target);
        let metadata = item(doc, self.target, true, ctl)?;
        if metadata.raster_bounds != Some(self.old_bounds) {
            return Err(bad("selection geometry changed before publication"));
        }
        let locks = crate::xcf_item_locks::effective_item_locks(doc, reference)?;
        locks.require(crate::xcf_item_locks::EditKind::Content)?;
        if self.bounds != self.old_bounds {
            locks.require(crate::xcf_item_locks::EditKind::Position)?;
        }
        poll(ctl)?;
        let state = doc.xcf_state.as_mut().ok_or_else(|| bad("selection canonical state disappeared"))?;
        let metadata = state.items.iter_mut().find(|item| item.item == reference).ok_or_else(|| bad("selection metadata disappeared"))?;
        metadata.raster_bounds = Some(self.bounds);
        // No other field, relationship, source record, or original payload is rewritten.
        poll(ctl)
    }
}

fn save_error(error: EngineError) -> EngineError {
    match error {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: "select.saveSelection".into(), msg },
        other => other,
    }
}
#[derive(Clone, Copy)]
pub(in crate::channel_cmds) struct SaveProof {
    source_document: photocraft_doc::DocId,
    target_document: photocraft_doc::DocId,
    source: ChannelId,
    target: ChannelId,
    bounds: Rect,
    old_bounds: Rect,
    format: PixelFormat,
}
pub(in crate::channel_cmds) fn prepare_save(source: &Document, target: &Document, target_id: ChannelId, ctl: &Interrupt<'_>) -> Result<SaveProof> {
    (|| {
        poll(ctl)?;
        if source.size != target.size || source.channels.len() > 10000 || target.channels.len() > 10000 {
            return Err(bad("canonical raw Save canvas/identity limit"));
        }
        let source_id = source.selection_channel_id.ok_or_else(|| bad("canonical raw Save needs an existing selection identity"))?;
        if source_id.0 == 0 || source.quick_mask.as_ref().is_some_and(|channel| channel.id == source_id) {
            return Err(bad("selection identity aliases Quick Mask"));
        }
        for channel in &source.channels {
            poll(ctl)?;
            if channel.id == source_id {
                return Err(bad("source selection identity aliases saved channel"));
            }
        }
        let selection = source.selection.as_ref().ok_or_else(|| bad("canonical raw Save needs stored selection"))?;
        let mut saved = None;
        for channel in &target.channels {
            poll(ctl)?;
            if channel.id == target_id && saved.replace(channel).is_some() {
                return Err(bad("duplicate target saved channel identity"));
            }
        }
        let saved = saved.ok_or_else(|| bad("missing canonical saved target"))?;
        if saved.spot.is_some() || target.selection_channel_id == Some(target_id) || target.quick_mask.as_ref().is_some_and(|channel| channel.id == target_id) {
            return Err(bad("raw Save target is not a plain saved channel"));
        }
        let format = selection.format();
        if format.mode != ColorMode::Grayscale
            || format.alpha
            || format.sample != source.depth
            || format.sample != target.depth
            || saved.surface.format() != format
        {
            return Err(bad("canonical raw Save Gray depth mismatch"));
        }
        let origin = item(source, source_id, false, ctl)?;
        let destination = item(target, target_id, true, ctl)?;
        if destination.name != saved.name {
            return Err(bad("saved channel current name/metadata mismatch"));
        }
        let bounds = origin.raster_bounds.ok_or_else(|| bad("missing selection extent"))?;
        let old_bounds = destination.raster_bounds.ok_or_else(|| bad("missing saved channel extent"))?;
        let locks = crate::xcf_item_locks::effective_item_locks(target, ItemRef::Channel(target_id))?;
        locks.require(crate::xcf_item_locks::EditKind::Content)?;
        if bounds != old_bounds {
            locks.require(crate::xcf_item_locks::EditKind::Position)?;
        }
        source.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
        target.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
        poll(ctl)?;
        Ok(SaveProof { source_document: source.id, target_document: target.id, source: source_id, target: target_id, bounds, old_bounds, format })
    })()
    .map_err(save_error)
}
impl SaveProof {
    pub(in crate::channel_cmds) fn changes_bounds(&self) -> bool {
        self.bounds != self.old_bounds
    }
    pub(in crate::channel_cmds) fn check_copy(
        &self,
        source: &DocumentSnapshot,
        target: &DocumentSnapshot,
        surface: &Surface,
        format: PixelFormat,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        (|| {
            poll(ctl)?;
            if source.id != self.source_document
                || target.id != self.target_document
                || format != self.format
                || source.selection_channel_id != Some(self.source)
                || !source.selection.as_ref().is_some_and(|stored| std::ptr::eq(stored, surface))
            {
                return Err(bad("canonical raw Save proof source mismatch"));
            }
            let mut matches = 0usize;
            for channel in &target.channels {
                poll(ctl)?;
                if channel.id == self.target {
                    matches += 1;
                }
            }
            if matches != 1 {
                return Err(bad("canonical raw Save proof target mismatch"));
            }
            poll(ctl)
        })()
        .map_err(save_error)
    }
    pub(in crate::channel_cmds) fn apply_bounds(self, doc: &mut Document, ctl: &Interrupt<'_>) -> Result<()> {
        (|| {
            poll(ctl)?;
            if doc.id != self.target_document {
                return Err(bad("saved channel publication document mismatch"));
            }
            let mut saved = None;
            for channel in &doc.channels {
                poll(ctl)?;
                if channel.id == self.target && saved.replace(channel).is_some() {
                    return Err(bad("duplicate published saved channel"));
                }
            }
            if saved.is_none_or(|channel| channel.surface.format() != self.format || channel.spot.is_some()) {
                return Err(bad("saved channel publication storage mismatch"));
            }
            let reference = ItemRef::Channel(self.target);
            let metadata = item(doc, self.target, true, ctl)?;
            if metadata.raster_bounds != Some(self.old_bounds) {
                return Err(bad("saved channel geometry changed before publication"));
            }
            let locks = crate::xcf_item_locks::effective_item_locks(doc, reference)?;
            locks.require(crate::xcf_item_locks::EditKind::Content)?;
            if self.bounds != self.old_bounds {
                locks.require(crate::xcf_item_locks::EditKind::Position)?;
            }
            poll(ctl)?;
            let state = doc.xcf_state.as_mut().ok_or_else(|| bad("saved channel canonical state disappeared"))?;
            let metadata = state.items.iter_mut().find(|item| item.item == reference).ok_or_else(|| bad("saved channel metadata disappeared"))?;
            metadata.raster_bounds = Some(self.bounds);
            poll(ctl)
        })()
        .map_err(save_error)
    }
}

fn new_error(error: EngineError) -> EngineError {
    match error {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: "channel.new".into(), msg },
        other => other,
    }
}
#[derive(Clone, Copy)]
pub(in crate::channel_cmds) struct NewProof {
    document: photocraft_doc::DocId,
    source: ChannelId,
    bounds: Rect,
    format: PixelFormat,
}
fn channel_namespace(doc: &Document, ctl: &Interrupt<'_>) -> Result<std::collections::HashSet<ChannelId>> {
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing canonical channel namespace"))?;
    let mut visits = doc
        .channels
        .len()
        .checked_add(state.items.len())
        .and_then(|n| n.checked_add(state.source_items.len()))
        .and_then(|n| n.checked_add(state.filter_stacks.len()))
        .and_then(|n| n.checked_add(state.item_sets.len()))
        .and_then(|n| n.checked_add(state.opaque.len()))
        .and_then(|n| n.checked_add(state.selection.selected.len()))
        .and_then(|n| n.checked_add(state.selection.selected_paths.len()))
        .filter(|n| *n <= 100000)
        .ok_or_else(|| bad("channel namespace container limit"))?;
    let mut identities = std::collections::HashSet::new();
    identities.try_reserve(10000).map_err(|_| EngineError::Other("channel.new: channel namespace allocation failed".into()))?;
    let mut record = |visits: &mut usize, reference: ItemRef| -> Result<()> {
        poll(ctl)?;
        *visits = visits.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad("channel namespace work limit"))?;
        if let ItemRef::Channel(id) = reference {
            if id.0 == 0 {
                return Err(bad("zero canonical channel reference"));
            }
            if !identities.contains(&id) && identities.len() >= 10000 {
                return Err(bad("channel namespace identity limit"));
            }
            identities.insert(id);
        }
        Ok(())
    };
    for channel in &doc.channels {
        record(&mut visits, ItemRef::Channel(channel.id))?;
    }
    if let Some(id) = doc.selection_channel_id {
        record(&mut visits, ItemRef::Channel(id))?;
    }
    if let Some(channel) = &doc.quick_mask {
        record(&mut visits, ItemRef::Channel(channel.id))?;
    }
    for metadata in &state.items {
        record(&mut visits, metadata.item)?;
    }
    for source in &state.source_items {
        record(&mut visits, source.owner)?;
    }
    for stack in &state.filter_stacks {
        record(&mut visits, stack.owner)?;
        visits = visits.checked_add(stack.nodes.len()).filter(|n| *n <= 100000).ok_or_else(|| bad("channel namespace node limit"))?;
        for node in &stack.nodes {
            poll(ctl)?;
            if let Some(mask) = node.mask {
                record(&mut visits, mask)?;
            }
            for (_, reference) in &node.auxiliary {
                record(&mut visits, *reference)?;
            }
        }
    }
    for set in &state.item_sets {
        poll(ctl)?;
        for reference in &set.members {
            record(&mut visits, *reference)?;
        }
    }
    for reference in state.selection.selected.iter().chain(&state.selection.selected_paths) {
        record(&mut visits, *reference)?;
    }
    for reference in [state.selection.active, state.selection.mask_target].into_iter().flatten() {
        record(&mut visits, reference)?;
    }
    if let Some(floating) = &state.floating {
        record(&mut visits, ItemRef::Layer(floating.layer))?;
        record(&mut visits, floating.target)?;
    }
    for opaque in &state.opaque {
        poll(ctl)?;
        if let Some(owner) = opaque.owner {
            record(&mut visits, owner)?;
        }
    }
    poll(ctl)?;
    Ok(identities)
}
pub(in crate::channel_cmds) fn prepare_new(doc: &Document, ctl: &Interrupt<'_>) -> Result<NewProof> {
    (|| {
        poll(ctl)?;
        if doc.channels.len() >= 10000 || doc.mode == ColorMode::Indexed || doc.color_table.is_some() {
            return Err(bad("raw canonical new channel context/count limit"));
        }
        let id = doc.selection_channel_id.ok_or_else(|| bad("raw canonical New needs a current selection identity"))?;
        if doc.quick_mask.as_ref().is_some_and(|channel| channel.id == id) {
            return Err(bad("selection identity aliases Quick Mask"));
        }
        for channel in &doc.channels {
            poll(ctl)?;
            if channel.id == id {
                return Err(bad("selection identity aliases saved channel"));
            }
        }
        let selection = doc.selection.as_ref().ok_or_else(|| bad("raw canonical New needs stored selection"))?;
        let format = selection.format();
        if format.mode != ColorMode::Grayscale || format.alpha || format.sample != doc.depth {
            return Err(bad("raw canonical New Gray depth mismatch"));
        }
        let source = item(doc, id, false, ctl)?;
        let bounds = source.raster_bounds.ok_or_else(|| bad("missing selection extent"))?;
        if channel_namespace(doc, ctl)?.len() >= 10000 {
            return Err(bad("new channel namespace headroom"));
        }
        let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing canonical New state"))?;
        state
            .items
            .len()
            .checked_add(state.source_items.len())
            .and_then(|n| n.checked_add(state.filter_stacks.len()))
            .and_then(|n| n.checked_add(state.filter_masks.len()))
            .and_then(|n| n.checked_add(state.opaque.len()))
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= 100000)
            .ok_or_else(|| bad("new canonical item headroom"))?;
        doc.validate_xcf_state_with(ctl).map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
        poll(ctl)?;
        Ok(NewProof { document: doc.id, source: id, bounds, format })
    })()
    .map_err(new_error)
}
impl NewProof {
    pub(in crate::channel_cmds) fn check_copy(&self, doc: &DocumentSnapshot, surface: &Surface, format: PixelFormat, ctl: &Interrupt<'_>) -> Result<()> {
        (|| {
            poll(ctl)?;
            if doc.id != self.document
                || doc.selection_channel_id != Some(self.source)
                || format != self.format
                || !doc.selection.as_ref().is_some_and(|stored| std::ptr::eq(stored, surface))
            {
                return Err(bad("canonical New proof source mismatch"));
            }
            poll(ctl)
        })()
        .map_err(new_error)
    }
    pub(in crate::channel_cmds) fn fresh_id(&self, doc: &Document, ctl: &Interrupt<'_>) -> Result<ChannelId> {
        (|| {
            poll(ctl)?;
            if doc.id != self.document {
                return Err(bad("canonical New identity document mismatch"));
            }
            let occupied = channel_namespace(doc, ctl)?;
            for _ in 0..occupied.len() + 4 {
                poll(ctl)?;
                let id = ChannelId::fresh();
                if id.0 != 0 && !occupied.contains(&id) {
                    return Ok(id);
                }
            }
            Err(bad("canonical new channel identity exhausted"))
        })()
        .map_err(new_error)
    }
    /// Reserves all new containers/name privately, then appends only a fresh ordinary item.
    pub(in crate::channel_cmds) fn install(self, doc: &mut Document, channel: photocraft_doc::AlphaChannel, ctl: &Interrupt<'_>) -> Result<usize> {
        (|| {
            poll(ctl)?;
            if doc.id != self.document
                || doc.selection_channel_id != Some(self.source)
                || channel.surface.format() != self.format
                || channel.spot.is_some()
                || channel.name.len() > 2048
            {
                return Err(bad("canonical New publication mismatch"));
            }
            let source = item(doc, self.source, false, ctl)?;
            if source.raster_bounds != Some(self.bounds) {
                return Err(bad("selection geometry changed before New publication"));
            }
            if channel_namespace(doc, ctl)?.contains(&channel.id) {
                return Err(bad("new channel identity collision"));
            }
            let mut name = String::new();
            name.try_reserve_exact(channel.name.len()).map_err(|_| bad("new channel metadata name allocation"))?;
            name.push_str(&channel.name);
            let metadata = ItemState {
                item: ItemRef::Channel(channel.id),
                name,
                custom_name: true,
                visible: false,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(self.bounds),
                mask: None,
                composition: None,
            };
            doc.channels.try_reserve_exact(1).map_err(|_| bad("new channel allocation"))?;
            let state = doc.xcf_state.as_mut().ok_or_else(|| bad("new channel canonical state disappeared"))?;
            state.items.try_reserve_exact(1).map_err(|_| bad("new channel metadata allocation"))?;
            poll(ctl)?;
            let index = doc.channels.len();
            state.items.push(metadata);
            doc.channels.push(channel);
            poll(ctl)?;
            Ok(index)
        })()
        .map_err(new_error)
    }
}

/// A once-per-dialog metadata hint, not source/storage validation or a new ID allocation.
pub(in crate::channel_cmds) fn new_hint(doc: &DocumentSnapshot, ctl: &Interrupt<'_>) -> Result<bool> {
    (|| {
        poll(ctl)?;
        if doc.channels.len() >= 10000 || doc.accounted().is_some() || doc.mode == ColorMode::Indexed || doc.color_table.is_some() {
            return Ok(false);
        }
        let Some(selection) = &doc.selection else { return Ok(false) };
        let format = selection.format();
        if format.mode != ColorMode::Grayscale || format.alpha || format.sample != doc.depth {
            return Ok(false);
        }
        // Mirrors the existing creation context and conservatively excludes resident-ineligible owners.
        match photocraft_doc::resident_structure::visit_layers_with(doc, ctl, |layer| {
            if layer.indexed_source.is_some() || matches!(layer.content, photocraft_doc::LayerContent::BackedRaster(_)) {
                return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
            }
            Ok(true)
        }) {
            Ok(()) => {}
            Err(photocraft_raster::RasterInputError::Cancelled) => return Err(EngineError::Cancelled),
            Err(photocraft_raster::RasterInputError::AllocationFailed) => {
                return Err(EngineError::Other("channel.new: raw storage hint allocation failed".into()));
            }
            Err(_) => return Ok(false),
        }
        let Some(state) = &doc.xcf_state else {
            poll(ctl)?;
            return Ok(doc.source_color_context.is_none() && doc.selection_channel_id.is_none());
        };
        let Some(id) = doc.selection_channel_id else { return Ok(false) };
        if id.0 == 0 || state.floating.is_some() || doc.quick_mask.as_ref().is_some_and(|channel| channel.id == id) {
            return Ok(false);
        }
        for channel in &doc.channels {
            poll(ctl)?;
            if channel.id == id {
                return Ok(false);
            }
        }
        if state
            .items
            .len()
            .checked_add(state.source_items.len())
            .and_then(|n| n.checked_add(state.filter_stacks.len()))
            .and_then(|n| n.checked_add(state.filter_masks.len()))
            .and_then(|n| n.checked_add(state.opaque.len()))
            .and_then(|n| n.checked_add(1))
            .is_none_or(|n| n > 100000)
        {
            return Ok(false);
        }
        // Read-only owner predicates do not treat selection content/position locks as a write grant.
        match item(doc, id, false, ctl) {
            Ok(_) => {}
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(_) => return Ok(false),
        }
        let occupied = match channel_namespace(doc, ctl) {
            Ok(ids) => ids,
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(error @ EngineError::Other(_)) => return Err(error),
            Err(_) => return Ok(false),
        };
        poll(ctl)?;
        Ok(occupied.len() < 10000)
    })()
    .map_err(new_error)
}

/// Metadata-only, same-document presentation hints. Never scans/hash/renders sample storage.
/// This is deliberately conservative and does not replace `prepare` at dispatch/commit.
pub(in crate::channel_cmds) fn hints(doc: &DocumentSnapshot, ctl: &Interrupt<'_>) -> Result<Vec<ChannelId>> {
    hints_for(doc, ctl, false)
}
/// Same bounded index, with the selection as read-only source and saved channels as targets.
/// Returned identities permit no creation, renaming, source admission or mutation by themselves.
pub(in crate::channel_cmds) fn save_hints(doc: &DocumentSnapshot, ctl: &Interrupt<'_>) -> Result<Vec<ChannelId>> {
    hints_for(doc, ctl, true).map_err(save_error)
}
fn hints_for(doc: &DocumentSnapshot, ctl: &Interrupt<'_>, save: bool) -> Result<Vec<ChannelId>> {
    use std::collections::{HashMap, HashSet};
    fn empty(ctl: &Interrupt<'_>) -> Result<Vec<ChannelId>> {
        poll(ctl)?;
        Ok(Vec::new())
    }
    fn allocation() -> EngineError {
        bad("raw storage hint allocation limit")
    }
    poll(ctl)?;
    let Some(state) = &doc.xcf_state else { return empty(ctl) };
    let Some(target_id) = doc.selection_channel_id else { return empty(ctl) };
    let Some(selection) = &doc.selection else { return empty(ctl) };
    if state.floating.is_some() || doc.accounted().is_some() || target_id.0 == 0 || doc.channels.len() > 10000 || state.items.len() > 10000 {
        return empty(ctl);
    }
    let format = selection.format();
    if format.mode != ColorMode::Grayscale || format.alpha || format.sample != doc.depth {
        return empty(ctl);
    }
    let mut visits = doc
        .channels
        .len()
        .checked_add(state.items.len())
        .and_then(|n| n.checked_add(state.source_items.len()))
        .and_then(|n| n.checked_add(state.filter_stacks.len()))
        .and_then(|n| n.checked_add(state.filter_masks.len()))
        .and_then(|n| n.checked_add(state.opaque.len()))
        .filter(|n| *n <= 100000)
        .ok_or_else(|| bad("raw storage hint census limit"))?;
    let mut index = HashMap::new();
    index.try_reserve(state.items.len()).map_err(|_| allocation())?;
    for metadata in &state.items {
        poll(ctl)?;
        if index.insert(metadata.item, metadata).is_some() {
            return empty(ctl);
        }
    }
    let target_ref = ItemRef::Channel(target_id);
    let Some(target) = index.get(&target_ref).copied() else { return empty(ctl) };
    // This is the selection record: source for Save, target for Load.
    if !plain_metadata(target) || (!save && target.locks.content) {
        return empty(ctl);
    }
    let Some(target_bounds) = target.raster_bounds.filter(|r| !r.is_empty()) else { return empty(ctl) };
    if photocraft_raster::checked_region_byte_len(target_bounds, format).is_err() {
        return empty(ctl);
    }
    let mut native_ids = HashSet::new();
    native_ids.try_reserve(doc.channels.len() + 2).map_err(|_| allocation())?;
    if !native_ids.insert(target_id) {
        return empty(ctl);
    }
    if let Some(quick) = &doc.quick_mask
        && (quick.id.0 == 0 || !native_ids.insert(quick.id))
    {
        return empty(ctl);
    }
    for channel in &doc.channels {
        poll(ctl)?;
        if channel.id.0 == 0 || !native_ids.insert(channel.id) {
            return empty(ctl);
        }
    }
    let mut forbidden = HashSet::new();
    forbidden.try_reserve(index.len()).map_err(|_| allocation())?;
    let mut dependencies = HashSet::new();
    dependencies.try_reserve(index.len()).map_err(|_| allocation())?;
    for source in &state.source_items {
        poll(ctl)?;
        if !index.contains_key(&source.owner) {
            return empty(ctl);
        }
        forbidden.insert(source.owner);
    }
    for stack in &state.filter_stacks {
        poll(ctl)?;
        if !index.contains_key(&stack.owner) {
            return empty(ctl);
        }
        forbidden.insert(stack.owner);
        visits = visits.checked_add(stack.nodes.len()).filter(|n| *n <= 100000).ok_or_else(|| bad("raw storage hint work limit"))?;
        for node in &stack.nodes {
            poll(ctl)?;
            visits = visits.checked_add(node.auxiliary.len()).filter(|n| *n <= 100000).ok_or_else(|| bad("raw storage hint work limit"))?;
            if let Some(mask) = node.mask {
                if !index.contains_key(&mask) {
                    return empty(ctl);
                }
                dependencies.insert(mask);
            }
            for (_, owner) in &node.auxiliary {
                poll(ctl)?;
                if !index.contains_key(owner) {
                    return empty(ctl);
                }
                dependencies.insert(*owner);
            }
        }
    }
    for record in &state.opaque {
        poll(ctl)?;
        if record.rendering_required {
            let Some(owner) = record.owner else { return empty(ctl) };
            if !index.contains_key(&owner) {
                return empty(ctl);
            }
            forbidden.insert(owner);
        }
    }
    if forbidden.contains(&target_ref) || (!save && dependencies.contains(&target_ref)) {
        return empty(ctl);
    }
    let mut result = Vec::new();
    result.try_reserve_exact(doc.channels.len()).map_err(|_| allocation())?;
    if result.capacity() > 10000 {
        return Err(allocation());
    }
    for channel in &doc.channels {
        poll(ctl)?;
        let reference = ItemRef::Channel(channel.id);
        if channel.spot.is_some() || channel.surface.format() != format || forbidden.contains(&reference) || (save && dependencies.contains(&reference)) {
            continue;
        }
        let Some(metadata) = index.get(&reference).copied() else { continue };
        if !plain_metadata(metadata) || (save && (metadata.locks.content || metadata.name != channel.name)) {
            continue;
        }
        let Some(bounds) = metadata.raster_bounds.filter(|r| !r.is_empty()) else { continue };
        let position_locked = if save { metadata.locks.position } else { target.locks.position };
        if photocraft_raster::checked_region_byte_len(bounds, format).is_err() || (position_locked && bounds != target_bounds) {
            continue;
        }
        result.push(channel.id);
    }
    poll(ctl)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::SampleType;
    use photocraft_doc::{
        AlphaChannel, XcfState,
        xcf_state::{ItemLocks, ItemState},
    };
    fn metadata(id: ChannelId, bounds: Rect) -> ItemState {
        ItemState {
            item: ItemRef::Channel(id),
            name: "Current".into(),
            custom_name: false,
            visible: false,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(17),
            source_flags: 0,
            raster_bounds: Some(bounds),
            mask: None,
            composition: None,
        }
    }
    fn fixture() -> (Document, ChannelId) {
        let mut doc = Document::new("Canonical raw draft", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::F64);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let channel = AlphaChannel::new("Saved", Surface::try_with_default_bytes(format, &(-0.0f64).to_ne_bytes()).unwrap());
        let id = channel.id;
        let selected = ChannelId::fresh();
        doc.selection_channel_id = Some(selected);
        doc.selection = Some(Surface::new(format));
        let mut state = XcfState::default();
        let mut saved_metadata = metadata(id, Rect::new(-7, 9, -5, 11));
        saved_metadata.name = "Saved".into();
        state.items.push(saved_metadata);
        state.items.push(metadata(selected, doc.bounds()));
        doc.channels.push(channel);
        doc.xcf_state = Some(state);
        (doc, id)
    }
    #[test]
    fn only_current_extent_changes_and_position_lock_refuses() {
        let (mut doc, id) = fixture();
        let proof = prepare(&doc, &doc, id, &Interrupt::NONE).unwrap();
        let original = doc.xcf_state.clone().unwrap();
        let selected = doc.selection_channel_id.unwrap();
        proof.apply_bounds(&mut doc, &Interrupt::NONE).unwrap();
        let mut expected = original.clone();
        expected.items.iter_mut().find(|item| item.item == ItemRef::Channel(selected)).unwrap().raster_bounds = Some(Rect::new(-7, 9, -5, 11));
        assert_eq!(doc.xcf_state.as_ref(), Some(&expected));
        assert_eq!(doc.selection_channel_id, Some(selected));
        let (mut doc, id) = fixture();
        doc.xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|item| item.item == ItemRef::Channel(doc.selection_channel_id.unwrap()))
            .unwrap()
            .locks
            .position = true;
        let before = doc.clone();
        assert!(prepare(&doc, &doc, id, &Interrupt::NONE).is_err());
        assert_eq!(doc, before);
        assert!(matches!(prepare(&doc, &doc, id, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }

    #[test]
    fn hints_are_metadata_only_and_lock_cancel_controls_stay_closed() {
        let (mut doc, id) = fixture();
        let format = doc.channels[0].surface.format();
        doc.channels[0].surface = Surface::try_with_default_bytes(format, &0x7ff8000000000042u64.to_ne_bytes()).unwrap();
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(doc.clone())).unwrap();
        assert_eq!(hints(&snapshot, &Interrupt::NONE).unwrap(), vec![id]);
        assert!(matches!(hints(&snapshot, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        let target = doc.selection_channel_id.unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(target)).unwrap().locks.position = true;
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(doc)).unwrap();
        assert!(hints(&snapshot, &Interrupt::NONE).unwrap().is_empty());
    }

    #[test]
    fn canonical_save_updates_only_target_extent_and_keeps_owner_fields() {
        let (mut doc, id) = fixture();
        let before = doc.xcf_state.clone().unwrap();
        let proof = prepare_save(&doc, &doc, id, &Interrupt::NONE).unwrap();
        assert!(proof.changes_bounds());
        proof.apply_bounds(&mut doc, &Interrupt::NONE).unwrap();
        let mut expected = before;
        let bounds = doc.bounds();
        expected.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().raster_bounds = Some(bounds);
        assert_eq!(doc.xcf_state.as_ref(), Some(&expected));
        assert_eq!(doc.channels[0].name, "Saved");
        let (mut doc, id) = fixture();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().locks.content = true;
        let before = doc.clone();
        assert!(prepare_save(&doc, &doc, id, &Interrupt::NONE).is_err());
        assert_eq!(doc, before);
        assert!(matches!(prepare_save(&doc, &doc, id, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }

    #[test]
    fn save_hints_read_selection_locks_but_fence_saved_target_locks_and_names() {
        let (mut doc, id) = fixture();
        let selected = doc.selection_channel_id.unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(selected)).unwrap().locks.content = true;
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(doc.clone())).unwrap();
        assert_eq!(save_hints(&snapshot, &Interrupt::NONE).unwrap(), vec![id]);
        assert!(hints(&snapshot, &Interrupt::NONE).unwrap().is_empty());
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().locks.position = true;
        let snapshot = DocumentSnapshot::from_resident(std::sync::Arc::new(doc)).unwrap();
        assert!(save_hints(&snapshot, &Interrupt::NONE).unwrap().is_empty());
        assert!(matches!(save_hints(&snapshot, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }

    #[test]
    fn canonical_new_is_fresh_ordinary_metadata_and_preserves_existing_state() {
        let (mut doc, _) = fixture();
        let old = doc.xcf_state.clone().unwrap();
        let proof = prepare_new(&doc, &Interrupt::NONE).unwrap();
        let id = proof.fresh_id(&doc, &Interrupt::NONE).unwrap();
        let mut channel = AlphaChannel::new("Independent new", doc.selection.clone().unwrap());
        channel.id = id;
        let index = proof.install(&mut doc, channel, &Interrupt::NONE).unwrap();
        assert_eq!(doc.channels[index].id, id);
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(&state.items[..old.items.len()], old.items.as_slice());
        let created = state.items.last().unwrap();
        assert_eq!(created.item, ItemRef::Channel(id));
        assert_eq!(created.name, "Independent new");
        assert_eq!(created.raster_bounds, Some(doc.bounds()));
        assert!(created.source_tattoo.is_none());
        assert_eq!(created.source_flags, 0);
        assert_eq!(created.locks, Default::default());
        assert!(created.parent.is_none() && created.mask.is_none() && created.composition.is_none());
        assert_eq!(state.provenance, old.provenance);
        assert_eq!(state.item_sets, old.item_sets);
        assert_eq!(state.selection, old.selection);
        assert!(matches!(prepare_new(&doc, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }
}
