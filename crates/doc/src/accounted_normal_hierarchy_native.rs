//! Named structural plain-tree native COW data path. Generic NativePixels stays
//! Legacy-only; complete OLD/STAGED operative proof and locks belong to engine.
use super::*;
use photocraft_raster::{
    backing::NativePlaneDescriptor,
    checked_view::{CheckedRasterView, CheckedTilePin},
};
#[derive(Clone, Copy)]
pub(super) struct Plan<'a> {
    layer: LayerId,
    ordinal: usize,
    parent: Option<(usize, LayerId)>,
    canonical: usize,
    descriptor: NativePlaneDescriptor,
    source: &'a crate::BackedLayer,
    rect: Rect,
    bytes: &'a [u8],
    copy_work: u64,
}
impl Plan<'_> {
    pub(super) fn copy_work(self) -> u64 {
        self.copy_work
    }
}
struct Prepared<'a> {
    logical: ModelCensus,
    plan: Option<Plan<'a>>,
    _scratch: QuotaLease,
}
/// No semantic/source authority: actual stored programme emptiness, including
/// disabled nodes, before any selected index or model-copy allocation.
fn plain(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if !accepts(owner) {
        return refused();
    }
    plain_document(owner.document(), c)
}
fn plain_document(doc: &Document, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    if state.filter_stacks.len() > 4096 {
        return Err(BackingError::Length);
    }
    if !state.filter_masks.is_empty() {
        return refused();
    }
    for stack in &state.filter_stacks {
        c.check(1)?;
        if !stack.nodes.is_empty() {
            return refused();
        }
    }
    Ok(())
}
fn prepare<'a>(owner: &'a AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<Prepared<'a>, BackingError> {
    c.check(1)?;
    let AccountedEdit::NativePixels { layer, rect, bytes } = edit else { return refused() };
    plain(owner, c)?;
    // Separate even for raw noops. Later whole-copy slack is never scratch credit.
    let fixed = std::mem::size_of::<Prepared<'_>>()
        .checked_add(std::mem::size_of::<CheckedRasterView<'_>>())
        .and_then(|n| n.checked_add(std::mem::size_of::<CheckedTilePin>()))
        .and_then(|n| n.checked_add(40 + 4096))
        .ok_or(BackingError::Length)?;
    let scratch = owner.quota().reserve(QuotaKind::Resident, fixed as u64).map_err(|_| BackingError::Quota)?;
    c.check(fixed as u64)?;
    let logical = measured_owner(owner, c)?;
    let index = owner.checked_readonly_normal_hierarchy_current_index(c)?;
    let mut selected = None;
    for row in index.rows() {
        c.check(1)?;
        if row.id == layer {
            if selected.replace(row).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let row = selected.ok_or(BackingError::Bounds)?;
    let actual_layer: &'a crate::Layer = row.layer;
    let LayerContent::BackedRaster(backed) = &actual_layer.content else { return refused() };
    if bytes.len() > 16 << 20 {
        return Err(BackingError::Length);
    }
    let bpp = imported_edits::validate_native(backed, rect, bytes, c)?;
    let parent = row.parent.map(|p| index.row(p).map(|r| (p, r.id)).ok_or(BackingError::Integrity)).transpose()?;
    let descriptor = backed.descriptor();
    let mut count = 0u64;
    for _ in rect.tiles() {
        c.check(16)?;
        count = count.checked_add(1).ok_or(BackingError::Length)?;
        if count > 4096 {
            return Err(BackingError::Length);
        }
    }
    let work = count
        .checked_mul(256 * 256)
        .and_then(|n| n.checked_mul(bpp as u64))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add((bytes.len() as u64).checked_mul(2)?))
        .and_then(|n| n.checked_add(4096))
        .ok_or(BackingError::Length)?;
    let plan = Plan { layer, ordinal: row.ordinal, parent, canonical: row.canonical_item_index, descriptor, source: backed, rect, bytes, copy_work: work };
    let noop = imported_edits::native_is_noop(row.layer, rect, bytes, c)?;
    c.check(1)?;
    Ok(Prepared { logical, plan: if noop { None } else { Some(plan) }, _scratch: scratch })
}
impl AccountedDocument {
    /// Structural native-region preflight for the exact actual plain hierarchy.
    /// This does not authorize rendering, locks, Source, selection or other edits.
    pub fn validate_readonly_normal_hierarchy_native_edit(&self, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
        let _prepared = prepare(self, edit, c)?;
        c.check(1)
    }
    /// Same-q fixed-root native COW through the complete paid tree copier. Full
    /// target/canonical/custody proof precedes every exact raw-byte no-op.
    pub fn try_edit_readonly_normal_hierarchy_native(&self, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let prepared = prepare(self, edit, c)?;
        let Some(plan) = prepared.plan else {
            c.check(1)?;
            return Ok(self.clone());
        };
        current_copy(self, prepared.logical, CopyKind::Native(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    plain_document(doc, c)?;
    if let Some((ordinal, id)) = plan.parent {
        if layer_at(doc, ordinal, c)?.id != id {
            return Err(BackingError::Integrity);
        }
    }
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let item = state.items.get(plan.canonical).ok_or(BackingError::Integrity)?;
    if item.item != ItemRef::Layer(plan.layer)
        || item.parent != plan.parent.map(|(_, id)| ItemRef::Layer(id))
        || item.raster_bounds != Some(plan.descriptor.bounds())
        || item.mask.is_some()
    {
        return Err(BackingError::Integrity);
    }
    let layer = layer_at_mut(doc, plan.ordinal, c)?;
    if layer.id != plan.layer {
        return Err(BackingError::Integrity);
    }
    let LayerContent::BackedRaster(backed) = &mut layer.content else { return refused() };
    // Conservative immutable provider/COW identity, not descriptor-only inference.
    if backed.descriptor() != plan.descriptor || &*backed != plan.source {
        return Err(BackingError::Integrity);
    }
    imported_edits::validate_native(backed, plan.rect, plan.bytes, c)?;
    let replacement = backed.try_edited_native(plan.rect, plan.bytes, c).map_err(|e| match e {
        crate::BackedLayerError::Backing(e) => e,
        _ => BackingError::Integrity,
    })?;
    c.check(1)?;
    *backed = replacement;
    Ok(())
}
