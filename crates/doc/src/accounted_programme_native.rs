//! Named native COW over actual programme-owning storage. This is structural
//! data custody only; engine must prove the full OLD and STAGED Current recipe.
use super::*;
use crate::item_identity::ItemRef;
use photocraft_raster::{
    backing::NativePlaneDescriptor,
    checked_view::{CheckedRasterView, CheckedTilePin},
};

/// Exact runtime storage metadata. Constructing this enum grants no edit,
/// Source, profile or rendering authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgrammeNativeStorageKind {
    ThresholdAlpha,
    HistogramThreshold,
    Point,
    AssignedPoint,
    GaussianGroup,
    FlatComposite,
    FilteredFlatPoint,
    MaskedFilteredFlatPoint,
    NormalHierarchy,
}
fn kind(owner: &AccountedDocument) -> Result<ProgrammeNativeStorageKind, BackingError> {
    use ProgrammeNativeStorageKind as K;
    Ok(match owner.owner._storage_class {
        StorageClass::ReadonlyThresholdAlpha => K::ThresholdAlpha,
        StorageClass::ReadonlyHistogramThreshold => K::HistogramThreshold,
        StorageClass::ReadonlyPoint => K::Point,
        StorageClass::ReadonlyAssignedPoint => K::AssignedPoint,
        StorageClass::ReadonlyGaussianGroup => K::GaussianGroup,
        StorageClass::ReadonlyFlatComposite => K::FlatComposite,
        StorageClass::ReadonlyFilteredFlatPoint => K::FilteredFlatPoint,
        StorageClass::ReadonlyMaskedFilteredFlatPoint => K::MaskedFilteredFlatPoint,
        StorageClass::ReadonlyNormalHierarchy => K::NormalHierarchy,
        StorageClass::Legacy => return Err(BackingError::Unsupported),
    })
}
fn measured(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    use ProgrammeNativeStorageKind as K;
    match kind(owner)? {
        K::ThresholdAlpha => filter_readonly::programme_measured(owner, c),
        K::HistogramThreshold => threshold_readonly::programme_measured(owner, c),
        K::Point | K::AssignedPoint => point_readonly::programme_measured(owner, c),
        K::GaussianGroup => gaussian_group_readonly::programme_measured(owner, c),
        K::FlatComposite => flat_composite_readonly::programme_measured(owner, c),
        K::FilteredFlatPoint | K::MaskedFilteredFlatPoint => filtered_flat_readonly::programme_measured(owner, c),
        K::NormalHierarchy => normal_hierarchy_readonly::programme_measured(owner, c),
    }
}
#[derive(Clone, Copy)]
struct Resolved<'a> {
    layer: &'a crate::Layer,
    ordinal: usize,
    parent: Option<(usize, LayerId)>,
    canonical: usize,
    backed: &'a crate::BackedLayer,
}
// No allocation or retained path. Complete measured class proof precedes this
// bounded actual-tree scan, including every fixed-group/root-only class.
fn resolve<'a>(doc: &'a Document, id: LayerId, c: &ReadControl<'_>) -> Result<Resolved<'a>, BackingError> {
    fn walk<'a>(
        layers: &'a [crate::Layer],
        depth: usize,
        parent: Option<(usize, LayerId)>,
        id: LayerId,
        next: &mut usize,
        found: &mut Option<(&'a crate::Layer, usize, Option<(usize, LayerId)>)>,
        c: &ReadControl<'_>,
    ) -> Result<(), BackingError> {
        if depth > 32 {
            return Err(BackingError::Length);
        }
        for layer in layers {
            c.check(1)?;
            let ordinal = *next;
            *next = next.checked_add(1).ok_or(BackingError::Length)?;
            if *next > 4096 {
                return Err(BackingError::Length);
            }
            if layer.id == id && found.replace((layer, ordinal, parent)).is_some() {
                return Err(BackingError::Integrity);
            }
            if let LayerContent::Group(group) = &layer.content {
                walk(&group.children, depth + 1, Some((ordinal, layer.id)), id, next, found, c)?;
            }
        }
        Ok(())
    }
    let mut found = None;
    walk(&doc.layers, 0, None, id, &mut 0, &mut found, c)?;
    let (layer, ordinal, parent) = found.ok_or(BackingError::Bounds)?;
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(BackingError::Unsupported);
    };
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if state.items.len() > 8192 {
        return Err(BackingError::Length);
    }
    let mut canonical = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == ItemRef::Layer(id) {
            if canonical.replace(index).is_some()
                || item.parent != parent.map(|(_, p)| ItemRef::Layer(p))
                || item.raster_bounds != Some(backed.operative_bounds())
            {
                return Err(BackingError::Integrity);
            }
        }
    }
    Ok(Resolved { layer, ordinal, parent, canonical: canonical.ok_or(BackingError::Integrity)?, backed })
}
/// Borrowed, privately constructed actual-owner witness. Engine capture may
/// copy these fixed metadata values and separately pay its own provider clone;
/// this value must not become a self-referential snapshot field.
pub struct CheckedProgrammeNativeTarget<'a> {
    kind: ProgrammeNativeStorageKind,
    resolved: Resolved<'a>,
    logical: ModelCensus,
    _scratch: QuotaLease,
}
impl CheckedProgrammeNativeTarget<'_> {
    pub fn storage_kind(&self) -> ProgrammeNativeStorageKind {
        self.kind
    }
    pub fn layer_id(&self) -> LayerId {
        self.resolved.layer.id
    }
    pub fn ordinal(&self) -> usize {
        self.resolved.ordinal
    }
    pub fn parent(&self) -> Option<(usize, LayerId)> {
        self.resolved.parent
    }
    pub fn canonical_item_index(&self) -> usize {
        self.resolved.canonical
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.resolved.backed.descriptor()
    }
    pub fn backed(&self) -> &crate::BackedLayer {
        self.resolved.backed
    }
}
#[derive(Clone, Copy)]
pub(super) struct Plan<'a> {
    kind: ProgrammeNativeStorageKind,
    layer: LayerId,
    ordinal: usize,
    parent: Option<(usize, LayerId)>,
    canonical: usize,
    descriptor: NativePlaneDescriptor,
    source: &'a crate::BackedLayer,
    rect: Rect,
    bytes: &'a [u8],
    work: u64,
}
impl Plan<'_> {
    pub(super) fn copy_work(self) -> u64 {
        self.work
    }
}
struct Prepared<'a> {
    target: CheckedProgrammeNativeTarget<'a>,
    plan: Option<Plan<'a>>,
}
fn prepare<'a>(owner: &'a AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<Prepared<'a>, BackingError> {
    c.check(1)?;
    let AccountedEdit::NativePixels { layer, rect, bytes } = edit else {
        return Err(BackingError::Unsupported);
    };
    let target = owner.checked_programme_native_target(layer, c)?;
    if bytes.len() > 16 << 20 {
        return Err(BackingError::Length);
    }
    let bpp = imported_edits::validate_native(target.resolved.backed, rect, bytes, c)?;
    let mut tiles = 0u64;
    for _ in rect.tiles() {
        c.check(16)?;
        tiles = tiles.checked_add(1).ok_or(BackingError::Length)?;
        if tiles > 4096 {
            return Err(BackingError::Length);
        }
    }
    let work = tiles
        .checked_mul(256 * 256)
        .and_then(|n| n.checked_mul(bpp as u64))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add((bytes.len() as u64).checked_mul(2)?))
        .and_then(|n| n.checked_add(4096))
        .ok_or(BackingError::Length)?;
    let plan = Plan {
        kind: target.kind,
        layer,
        ordinal: target.resolved.ordinal,
        parent: target.resolved.parent,
        canonical: target.resolved.canonical,
        descriptor: target.resolved.backed.descriptor(),
        source: target.resolved.backed,
        rect,
        bytes,
        work,
    };
    let noop = imported_edits::native_is_noop(target.resolved.layer, rect, bytes, c)?;
    c.check(1)?;
    Ok(Prepared { target, plan: if noop { None } else { Some(plan) } })
}
impl AccountedDocument {
    /// Complete structural current-class proof and actual raster target lookup.
    /// No semantic recipe, lock, Source or command authority is returned.
    pub fn checked_programme_native_target(&self, layer: LayerId, c: &ReadControl<'_>) -> Result<CheckedProgrammeNativeTarget<'_>, BackingError> {
        c.check(1)?;
        let kind = kind(self)?;
        // Covers fixed target/plan, bounded DFS frames and raw compare view/pin.
        // Each selected class additionally owns its real index/validation fees.
        let fixed = std::mem::size_of::<Prepared<'_>>()
            .checked_add(std::mem::size_of::<Plan<'_>>())
            .and_then(|n| n.checked_add(std::mem::size_of::<CheckedRasterView<'_>>()))
            .and_then(|n| n.checked_add(std::mem::size_of::<CheckedTilePin>()))
            .and_then(|n| n.checked_add(40 + 8192))
            .ok_or(BackingError::Length)?;
        let scratch = self.quota().reserve(QuotaKind::Resident, fixed as u64).map_err(|_| BackingError::Quota)?;
        c.check(fixed as u64)?;
        let logical = measured(self, c)?;
        let resolved = resolve(self.document(), layer, c)?;
        c.check(1)?;
        Ok(CheckedProgrammeNativeTarget { kind, resolved, logical, _scratch: scratch })
    }
    /// Named data-only entry; generic NativePixels and plain-hierarchy methods
    /// retain their original scopes. Only raw native region replacement is valid.
    pub fn validate_programme_preserving_native_edit(&self, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
        let _prepared = prepare(self, edit, c)?;
        c.check(1)
    }
    /// Stage a real same-q COW through the actual class's whole-model copier.
    /// Full OLD/history/STAGED operative checks remain engine responsibilities.
    pub fn try_edit_programme_preserving_native(&self, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let prepared = prepare(self, edit, c)?;
        let Some(plan) = prepared.plan else {
            c.check(1)?;
            return Ok(self.clone());
        };
        if kind(self)? != plan.kind {
            return Err(BackingError::Integrity);
        }
        use ProgrammeNativeStorageKind as K;
        let logical = prepared.target.logical;
        match plan.kind {
            K::ThresholdAlpha => filter_readonly::programme_copy(self, logical, plan, c),
            K::HistogramThreshold => threshold_readonly::programme_copy(self, logical, plan, c),
            K::Point | K::AssignedPoint => point_readonly::programme_copy(self, logical, plan, c),
            K::GaussianGroup => gaussian_group_readonly::programme_copy(self, logical, plan, c),
            K::FlatComposite => flat_composite_readonly::programme_copy(self, logical, plan, c),
            K::FilteredFlatPoint | K::MaskedFilteredFlatPoint => filtered_flat_readonly::programme_copy(self, logical, plan, c),
            K::NormalHierarchy => normal_hierarchy_readonly::programme_copy(self, logical, plan, c),
        }
    }
}
fn layer_mut<'a>(
    layers: &'a mut [crate::Layer],
    target: usize,
    next: &mut usize,
    depth: usize,
    c: &ReadControl<'_>,
) -> Result<Option<&'a mut crate::Layer>, BackingError> {
    if depth > 32 {
        return Err(BackingError::Length);
    }
    for layer in layers {
        c.check(1)?;
        let ordinal = *next;
        *next = next.checked_add(1).ok_or(BackingError::Length)?;
        if *next > 4096 {
            return Err(BackingError::Length);
        }
        if ordinal == target {
            return Ok(Some(layer));
        }
        if let LayerContent::Group(group) = &mut layer.content {
            if let Some(found) = layer_mut(&mut group.children, target, next, depth + 1, c)? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}
pub(super) fn apply(doc: &mut Document, plan: Plan<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let actual = resolve(doc, plan.layer, c)?;
    if actual.ordinal != plan.ordinal
        || actual.parent != plan.parent
        || actual.canonical != plan.canonical
        || actual.backed.descriptor() != plan.descriptor
        || actual.backed != plan.source
    {
        return Err(BackingError::Integrity);
    }
    imported_edits::validate_native(actual.backed, plan.rect, plan.bytes, c)?;
    let layer = layer_mut(&mut doc.layers, plan.ordinal, &mut 0, 0, c)?.ok_or(BackingError::Integrity)?;
    if layer.id != plan.layer {
        return Err(BackingError::Integrity);
    }
    let LayerContent::BackedRaster(backed) = &mut layer.content else {
        return Err(BackingError::Integrity);
    };
    let replacement = backed.try_edited_native(plan.rect, plan.bytes, c).map_err(|e| match e {
        crate::BackedLayerError::Backing(e) => e,
        _ => BackingError::Integrity,
    })?;
    c.check(1)?;
    layer.content = LayerContent::BackedRaster(replacement);
    Ok(())
}
