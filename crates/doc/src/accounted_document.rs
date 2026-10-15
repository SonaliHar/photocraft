//! Persistent Session/history ownership for explicitly accounted native models.
//! Borrowed Documents remain Clone-capable; callers must use this transaction
//! API for the internally enforced lifecycle, rather than cloning the raw view.
use crate::accounted_selection::{AccountedSelection, SelectionView, SelectionViewCharges};
use crate::{BackedLayerError, Document, LayerContent, LayerId, WorkingProfileInterpretation};
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{
    any::Any,
    sync::{Arc, Weak},
};

#[path = "accounted_imported_edits.rs"]
mod imported_edits;
#[path = "accounted_selection_rehydrate.rs"]
mod selection_rehydrate;
#[path = "accounted_filter_readonly.rs"]
mod filter_readonly;
#[path = "accounted_threshold_readonly.rs"]
mod threshold_readonly;
#[path = "accounted_point_readonly.rs"]
mod point_readonly;
pub use point_readonly::{PointArgumentPatch, PointArgumentValue, PointFilterMovePatch, PointFilterPatch, PointFilterRemovePatch};
pub use point_readonly::{
    PointArgumentShape, PointStructuredArgumentPatch, PointStructuredFilterPatch,
    point_argument_shape, point_argument_shape_with, point_value_shape, point_values_equal, point_values_same_shape,
};
pub use filter_readonly::ThresholdAlphaPatch;
pub use threshold_readonly::ThresholdPatch;
#[path = "accounted_readonly_layer_name.rs"]
mod readonly_layer_name;
pub use readonly_layer_name::ReadonlyLayerNamePatch;
#[path = "accounted_readonly_layer_properties.rs"]
mod readonly_layer_properties;
pub use readonly_layer_properties::ReadonlyLayerPropertiesPatch;
#[path = "accounted_document_name.rs"]
mod document_name;
#[cfg(test)]
#[path = "accounted_constructor_requirements_tests.rs"]
mod constructor_requirements_tests;
#[path = "accounted_imported_selection.rs"]
mod imported_selection;
#[path = "accounted_document/plain_xcf_inspection.rs"]
mod plain_xcf_inspection;

struct ModelCharges {
    _index: QuotaLease,
    _payload: QuotaLease,
}
#[derive(Clone, Copy)]
struct ModelCensus {
    index: u64,
    payload: u64,
}
impl ModelCensus {
    fn total(self) -> Result<u64, BackingError> {
        self.index.checked_add(self.payload).ok_or(BackingError::Length)
    }
}
// Error paths destroy document/default/map storage before refunding view leases.
struct ModelStaging {
    document: Document,
    selection_view: Option<SelectionViewCharges>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum StorageClass {
    Legacy,
    ReadonlyThresholdAlpha,
    ReadonlyHistogramThreshold,
    ReadonlyPoint,
}
struct ModelOwner {
    _storage_class: StorageClass,
    document: Document,
    _selection: Option<AccountedSelection>,
    _selection_view: Option<SelectionViewCharges>,
    retention: Arc<dyn Any + Send + Sync>,
    quota: BackingQuota,
    _model: Option<ModelCharges>,
    _allocation: Arc<QuotaLease>,
}
/// Clone shares one physical model and its charge, suitable for undo snapshots.
#[derive(Clone)]
pub struct AccountedDocument {
    owner: Arc<ModelOwner>,
}
/// A weak cache identity releases model/source payloads when strong owners end,
/// but retains the allocation charge while the Arc control block remains alive.
#[derive(Clone, Debug)]
pub struct WeakAccountedDocument {
    owner: Weak<ModelOwner>,
    _allocation: Arc<QuotaLease>,
}
impl WeakAccountedDocument {
    pub fn upgrade(&self) -> Option<AccountedDocument> {
        self.owner.upgrade().map(|owner| AccountedDocument { owner })
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.owner, &other.owner)
    }
}
impl std::fmt::Debug for AccountedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountedDocument").field("document", &self.owner.document).finish_non_exhaustive()
    }
}
/// Edits cannot expose an unrestricted mutable model or allocate an uncharged
/// retained collection in a user callback. Incoming names/native data are borrowed.
#[derive(Clone, Copy, Debug)]
pub enum AccountedEdit<'a> {
    /// Atomic current metadata, initially restricted to the closed imported lane.
    LayerMetadata {
        layer: LayerId,
        name: Option<&'a str>,
        visible: Option<bool>,
        opacity: Option<f32>,
    },
    LayerVisibility { layer: LayerId, visible: bool },
    LayerOpacity { layer: LayerId, opacity: f32 },
    LayerName { layer: LayerId, name: &'a str },
    NativePixels { layer: LayerId, rect: Rect, bytes: &'a [u8] },
    MaskEnabled { layer: LayerId, enabled: bool },
    MaskLinked { layer: LayerId, linked: bool },
}
impl AccountedEdit<'_> {
    /// Target identity independent of the specific typed transaction.
    pub fn layer_id(self) -> LayerId {
        match self {
            Self::LayerMetadata { layer, .. }
            | Self::LayerVisibility { layer, .. }
            | Self::LayerOpacity { layer, .. }
            | Self::LayerName { layer, .. }
            | Self::NativePixels { layer, .. }
            | Self::MaskEnabled { layer, .. }
            | Self::MaskLinked { layer, .. } => layer,
        }
    }
}

/// Bounded charged history slots. Snapshots share physical model/native owners;
/// failed edits never truncate redo or publish a partially edited current model.
#[derive(Debug)]
pub struct AccountedHistory {
    states: Vec<AccountedDocument>,
    cursor: usize,
    maximum: usize,
    _lease: QuotaLease,
}
impl AccountedHistory {
    pub fn new(initial: AccountedDocument, maximum: usize, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if maximum == 0 || maximum > 256 {
            return Err(BackingError::Length);
        }
        let capacity = maximum
            .checked_mul(std::mem::size_of::<AccountedDocument>())
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(std::mem::size_of::<Self>()))
            .ok_or(BackingError::Length)?;
        let mut lease = initial.quota().reserve(QuotaKind::Index, capacity as u64).map_err(|_| BackingError::Quota)?;
        let mut states = Vec::new();
        states.try_reserve_exact(maximum).map_err(|_| BackingError::Allocation)?;
        let actual = states
            .capacity()
            .checked_mul(std::mem::size_of::<AccountedDocument>())
            .and_then(|v| v.checked_add(std::mem::size_of::<Self>()))
            .ok_or(BackingError::Length)?;
        if actual > capacity {
            return Err(BackingError::Allocation);
        }
        lease.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
        states.push(initial);
        control.check(1)?;
        Ok(Self { states, cursor: 0, maximum, _lease: lease })
    }
    pub fn current(&self) -> Result<&AccountedDocument, BackingError> {
        self.states.get(self.cursor).ok_or(BackingError::Integrity)
    }
    pub fn len(&self) -> usize {
        self.states.len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }
    pub fn can_redo(&self) -> bool {
        self.cursor.checked_add(1).is_some_and(|next| next < self.states.len())
    }
    pub fn try_commit(&mut self, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
        imported_edits::validate_owner(self.current()?, edit, control)?;
        if is_noop(self.current()?.document(), edit, control)? {
            control.check(1)?;
            return Ok(());
        }
        let next = self.cursor.checked_add(1).ok_or(BackingError::Length)?;
        if next >= self.maximum {
            return Err(BackingError::Quota);
        }
        let staged = self.current()?.try_edit(edit, control)?;
        control.check(1)?;
        self.states.truncate(next);
        self.states.push(staged);
        self.cursor = next;
        Ok(())
    }
    pub fn try_replace_selection(&mut self, selection: Option<&AccountedSelection>, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.current()?.selection_noop(selection, control)? {
            control.check(1)?;
            return Ok(());
        }
        let next = self.cursor.checked_add(1).ok_or(BackingError::Length)?;
        if next >= self.maximum {
            return Err(BackingError::Quota);
        }
        let staged = self.current()?.try_replace_selection(selection, control)?;
        control.check(1)?;
        self.states.truncate(next);
        self.states.push(staged);
        self.cursor = next;
        Ok(())
    }
    pub fn undo(&mut self, control: &ReadControl<'_>) -> Result<bool, BackingError> {
        control.check(1)?;
        if self.cursor == 0 {
            return Ok(false);
        }
        self.cursor -= 1;
        Ok(true)
    }
    pub fn redo(&mut self, control: &ReadControl<'_>) -> Result<bool, BackingError> {
        control.check(1)?;
        let next = self.cursor.checked_add(1).ok_or(BackingError::Length)?;
        if next >= self.states.len() {
            return Ok(false);
        }
        self.cursor = next;
        Ok(true)
    }
}
impl AccountedDocument {
    /// Existing `from_owned` owner/Arc index envelope. The codec's physical
    /// document/native storage guard is separate; this value grants no receipt
    /// or authority and performs no allocation or model traversal.
    pub const fn owned_model_reservation_bytes() -> u64 {
        (std::mem::size_of::<ModelOwner>() + 128) as u64
    }
    /// Move a model together with a codec-owned guard retaining its existing
    /// model reservations and immutable archive. This is an explicit ownership
    /// transfer contract: the guard must already account this physical model.
    /// This constructor charges its own owner metadata, not the model a second time.
    /// Any immutable Arc<Surface> mask must already be physically prepaid by
    /// the guard, including its Arc allocation, opaque map/default and shared
    /// tile owners. A copied model only shares that Arc and retains this guard;
    /// the census does not invent an allocation size for BTreeMap nodes.
    pub fn from_owned(document: Document, retention: Arc<dyn Any + Send + Sync>, quota: BackingQuota, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        // Field order guarantees payload destruction before its retention guard
        // even on validation/reservation failure; argument drop order is unused.
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        validate_backing(&incoming.document, &quota, control)?;
        let lease = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner { _storage_class: StorageClass::Legacy, document, _selection: None, _selection_view: None, retention, quota, _model: None, _allocation: Arc::new(lease) }),
        })
    }
    pub fn document(&self) -> &Document {
        &self.owner.document
    }
    pub fn quota(&self) -> &BackingQuota {
        &self.owner.quota
    }
    pub fn shares_model(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
    }
    pub fn downgrade(&self) -> WeakAccountedDocument {
        WeakAccountedDocument { owner: Arc::downgrade(&self.owner), _allocation: self.owner._allocation.clone() }
    }
    /// Ephemeral identity for a cache held with a live/weak owner, never authority.
    pub fn model_identity(&self) -> usize {
        Arc::as_ptr(&self.owner) as usize
    }
    fn selection_noop(&self, selection: Option<&AccountedSelection>, control: &ReadControl<'_>) -> Result<bool, BackingError> {
        let doc = self.document();
        control.check(1)?;
        if self.owner._storage_class != StorageClass::Legacy { return Err(BackingError::Unsupported); }
        if doc.selection_channel_id.is_some() {
            return Err(BackingError::Unsupported);
        }
        if imported_selection::applies(doc) {
            imported_selection::validate(doc, self.quota(), control)?;
        }
        match (&doc.selection, self.owner._selection.as_ref()) {
            (None, None) => {}
            (Some(surface), Some(guard)) if guard.matches(surface, self.quota(), control)? => {}
            _ => return Err(BackingError::Integrity),
        }
        if let Some(guard) = selection {
            if !self.quota().shares_ledger(guard.quota()) {
                return Err(BackingError::Integrity);
            }
            if !selection_format_allowed(guard.format(), doc.depth)
                || !doc.bounds().contains_rect(&guard.bounds())
            {
                return Err(BackingError::Bounds);
            }
        }
        let noop = match (selection, &doc.selection) {
            (None, None) => true,
            (Some(guard), Some(surface)) => guard.matches(surface, self.quota(), control)?,
            _ => false,
        };
        control.check(1)?;
        Ok(noop)
    }
    /// Borrowed metadata admission only. This neither allocates a replacement nor
    /// grants a proof; final typed publication independently repeats its checks.
    pub fn validate_selection_replacement(&self, control: &ReadControl<'_>) -> Result<(), BackingError> {
        self.selection_noop(None, control)?;
        selection_census(self.document(), false, self.owner._selection.as_ref(), self.quota(), control)?;
        Ok(())
    }
    /// Anonymous pixel-selection transaction. The separate imported plain-raster
    /// copier preserves retained metadata; unguarded codec selections refuse.
    /// Exact no-op shares the model, preserving the caller's redo branch.
    pub fn try_replace_selection(&self, selection: Option<&AccountedSelection>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        if self.selection_noop(selection, control)? {
            return Ok(self.clone());
        }
        let doc = self.document();
        let logical = selection_census(doc, imported_selection::applies(doc), self.owner._selection.as_ref(), self.quota(), control)?;
        let owner_bytes = (std::mem::size_of::<ModelOwner>() + 128) as u64;
        let index = logical.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        let index_envelope = index.checked_mul(2).and_then(|v| v.checked_add(4096)).ok_or(BackingError::Length)?;
        let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
        let allocation = self.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
        let mut index_lease = self.quota().reserve(QuotaKind::Index, index_envelope).map_err(|_| BackingError::Quota)?;
        let mut payload_lease = self.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
        let view = selection.map(|guard| guard.clone_view(control)).transpose()?;
        control.check(logical.total()?)?;
        let copied = if imported_selection::applies(doc) {
            imported_selection::copy_document(doc, control)?
        } else {
            clone_model_without_selection(doc)
        };
        let mut staged = ModelStaging { document: copied, selection_view: None };
        if let Some(SelectionView { surface, charges }) = view {
            staged.document.selection = Some(surface);
            staged.selection_view = Some(charges);
        }
        let retained = selection_census(&staged.document, true, selection, self.quota(), control)?;
        let retained_index = retained.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        if retained_index > index_envelope || retained.payload > payload_envelope {
            return Err(BackingError::Allocation);
        }
        index_lease.reconcile_capacity(retained_index).map_err(|_| BackingError::Quota)?;
        payload_lease.reconcile_capacity(retained.payload).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let ModelStaging { document, selection_view } = staged;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::Legacy,
                document,
                _selection: selection.cloned(),
                _selection_view: selection_view,
                retention: self.owner.retention.clone(),
                quota: self.quota().clone(),
                _model: Some(ModelCharges { _index: index_lease, _payload: payload_lease }),
                _allocation: Arc::new(allocation),
            }),
        })
    }
    /// Borrowed preflight for the closed imported current-metadata transaction.
    /// No allocation or mutation authority; final try_edit repeats these gates.
    pub fn validate_imported_metadata_edit(&self, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.owner._storage_class != StorageClass::Legacy || !imported_selection::applies(self.document())
            || !matches!(edit, AccountedEdit::LayerMetadata { .. } | AccountedEdit::LayerName { .. } | AccountedEdit::LayerVisibility { .. } | AccountedEdit::LayerOpacity { .. })
        { return Err(BackingError::Unsupported); }
        imported_edits::validate_owner(self, edit, control)?;
        accounted_layer_access(self.document(), edit.layer_id(), control)?.ok_or(BackingError::Bounds)?;
        Ok(())
    }
    /// Borrowed preflight for raw native writes in the existing closed imported
    /// plain class. Empty/outside regions and wrong native lengths refuse. This
    /// grants no mutable model access; final try_edit independently revalidates.
    pub fn validate_imported_native_edit(&self, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.owner._storage_class != StorageClass::Legacy || !imported_selection::applies(self.document())
            || !matches!(edit, AccountedEdit::NativePixels { .. })
        { return Err(BackingError::Unsupported); }
        imported_edits::validate_owner(self, edit, control)?;
        accounted_layer_access(self.document(), edit.layer_id(), control)?.ok_or(BackingError::Bounds)?;
        Ok(())
    }
    /// Stage one separately charged model; errors preserve this history snapshot.
    /// Unknown collections refuse before clone. Native COW allocation uses the
    /// same source ledger and never replaces immutable source/default samples.
    pub fn try_edit(&self, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        if self.owner._storage_class != StorageClass::Legacy { return Err(BackingError::Unsupported); }
        let imported = imported_edits::validate_owner(self, edit, control)?;
        if is_noop(self.document(), edit, control)? {
            control.check(1)?;
            return Ok(self.clone());
        }
        let doc = self.document();
        let id = edit.layer_id();
        let target = accounted_layer_access(doc, id, control)?.ok_or(BackingError::Bounds)?;
        if matches!(edit, AccountedEdit::NativePixels { .. }) && !matches!(target.layer.content, LayerContent::BackedRaster(_)) {
            return Err(BackingError::Unsupported);
        }
        let extra = match edit {
            AccountedEdit::LayerMetadata { name, .. } => name.map_or(Ok(0), |name| name.len().checked_mul(2).map(|n| n as u64).ok_or(BackingError::Length))?,
            AccountedEdit::LayerName { name, .. } => {
                if !imported && (name.len() > 1 << 20 || name.contains('\0')) {
                    return Err(BackingError::Length);
                }
                name.len().checked_mul(2).ok_or(BackingError::Length)? as u64
            }
            AccountedEdit::LayerOpacity { opacity, .. } if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) => return Err(BackingError::Bounds),
            _ => 0,
        };
        if matches!(edit, AccountedEdit::NativePixels { .. })
            && doc.xcf_state.as_ref().is_some_and(|state| state.source_items.iter().any(|item| item.owner == crate::item_identity::ItemRef::Layer(id)))
        {
            return Err(BackingError::Unsupported);
        }
        let logical = if imported {
            selection_census(doc, false, self.owner._selection.as_ref(), self.quota(), control)?
        } else {
            census(doc, false, self.owner._selection.as_ref(), self.quota(), control)?
        };
        let owner_bytes = (std::mem::size_of::<ModelOwner>() + 128) as u64;
        let index = logical.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        let index_envelope = index.checked_add(extra).and_then(|v| v.checked_mul(2)).and_then(|v| v.checked_add(4096)).ok_or(BackingError::Length)?;
        let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
        let allocation = self.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
        let mut index_lease = self.quota().reserve(QuotaKind::Index, index_envelope).map_err(|_| BackingError::Quota)?;
        let mut payload_lease = self.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
        control.check(logical.total()?)?;
        // The admitted Clone only duplicates the prepaid bounded tree containers and
        // strings below. Native tiles/ICC/source guards retain shared ownership.
        let view = self.owner._selection.as_ref().map(|guard| guard.clone_view(control)).transpose()?;
        let copied = if imported { imported_selection::copy_document(doc, control)? } else { clone_model_without_selection(doc) };
        let mut staged = ModelStaging { document: copied, selection_view: None };
        if let Some(SelectionView { surface, charges }) = view {
            staged.document.selection = Some(surface);
            staged.selection_view = Some(charges);
        }
        let layer = find_layer_mut(&mut staged.document.layers, id, 0, control)?.ok_or(BackingError::Bounds)?;
        match edit {
            AccountedEdit::LayerMetadata { name, visible, opacity, .. } => {
                if let Some(name) = name {
                    layer.name = imported_edits::copy_name(name, control)?;
                }
                if let Some(visible) = visible {
                    layer.visible = visible;
                }
                if let Some(opacity) = opacity {
                    layer.opacity = opacity;
                }
            }
            AccountedEdit::LayerVisibility { visible, .. } => layer.visible = visible,
            AccountedEdit::LayerOpacity { opacity, .. } => layer.opacity = opacity,
            AccountedEdit::MaskEnabled { enabled, .. } => layer.mask.as_mut().ok_or(BackingError::Unsupported)?.enabled = enabled,
            AccountedEdit::MaskLinked { linked, .. } => layer.mask.as_mut().ok_or(BackingError::Unsupported)?.linked = linked,
            AccountedEdit::LayerName { name, .. } => {
                if imported {
                    layer.name = imported_edits::copy_name(name, control)?;
                } else {
                    let mut owned = String::new();
                    owned.try_reserve_exact(name.len()).map_err(|_| BackingError::Allocation)?;
                    owned.push_str(name);
                    layer.name = owned;
                }
            }
            AccountedEdit::NativePixels { rect, bytes, .. } => {
                let LayerContent::BackedRaster(backed) = &layer.content else {
                    return Err(BackingError::Unsupported);
                };
                let staged = backed.try_edited_native(rect, bytes, control).map_err(|error| match error {
                    BackedLayerError::Backing(e) => e,
                    BackedLayerError::DescriptorMismatch => BackingError::Integrity,
                })?;
                layer.content = LayerContent::BackedRaster(staged);
            }
        }
        if imported {
            imported_edits::synchronize(&mut staged.document, edit, control)?;
        } else {
            sync_current_item(&mut staged.document, edit)?;
        }
        let retained = if imported {
            selection_census(&staged.document, true, self.owner._selection.as_ref(), self.quota(), control)?
        } else {
            census(&staged.document, true, self.owner._selection.as_ref(), self.quota(), control)?
        };
        let retained_index = retained.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        if retained_index > index_envelope || retained.payload > payload_envelope {
            return Err(BackingError::Allocation);
        }
        index_lease.reconcile_capacity(retained_index).map_err(|_| BackingError::Quota)?;
        payload_lease.reconcile_capacity(retained.payload).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let ModelStaging { document, selection_view } = staged;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::Legacy,
                document,
                _selection: self.owner._selection.clone(),
                _selection_view: selection_view,
                retention: self.owner.retention.clone(),
                quota: self.quota().clone(),
                _model: Some(ModelCharges { _index: index_lease, _payload: payload_lease }),
                _allocation: Arc::new(allocation),
            }),
        })
    }
}
fn is_noop(doc: &Document, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<bool, BackingError> {
    let id = edit.layer_id();
    let layer = accounted_layer_access(doc, id, control)?.ok_or(BackingError::Bounds)?.layer;
    match edit {
        AccountedEdit::LayerMetadata { name, visible, opacity, .. } => {
            if !imported_selection::applies(doc) {
                return Err(BackingError::Unsupported);
            }
            // Incoming fields were validated by validate_owner before this
            // private helper; every repeated name comparison is charged here.
            let same_name = match name { Some(name) => imported_edits::names_equal(name, &layer.name, control)?, None => true };
            if opacity.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
                return Err(BackingError::Bounds);
            }
            Ok(same_name
                && visible.is_none_or(|v| v == layer.visible)
                && opacity.is_none_or(|v| v.to_bits() == layer.opacity.to_bits()))
        }
        AccountedEdit::LayerVisibility { visible, .. } => Ok(layer.visible == visible),
        AccountedEdit::LayerOpacity { opacity, .. } => {
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                return Err(BackingError::Bounds);
            }
            Ok(layer.opacity.to_bits() == opacity.to_bits())
        }
        AccountedEdit::LayerName { name, .. } => {
            if imported_selection::applies(doc) { return imported_edits::names_equal(&layer.name, name, control); }
            if name.len() > 1 << 20 || name.contains('\0') {
                return Err(BackingError::Length);
            }
            Ok(layer.name == name)
        }
        AccountedEdit::NativePixels { rect, bytes, .. } => {
            if imported_selection::applies(doc) { imported_edits::native_is_noop(layer, rect, bytes, control) } else { Ok(false) }
        },
        AccountedEdit::MaskEnabled { enabled, .. } => {
            let mask = validate_mask_control(doc, layer, control)?;
            Ok(mask.enabled == enabled)
        }
        AccountedEdit::MaskLinked { linked, .. } => {
            let mask = validate_mask_control(doc, layer, control)?;
            Ok(mask.linked == linked)
        }
    }
}
fn validate_mask_control<'a>(doc: &Document, layer: &'a crate::Layer, control: &ReadControl<'_>) -> Result<&'a crate::LayerMask, BackingError> {
    let mask = layer.mask.as_ref().ok_or(BackingError::Unsupported)?;
    let Some(state) = &doc.xcf_state else {
        return Ok(mask);
    };
    if state.items.len() > 4096 {
        return Err(BackingError::Length);
    }
    let mut found = false;
    for (index, item) in state.items.iter().enumerate() {
        if index % 64 == 0 {
            control.check(1)?;
        }
        if let Some(current) = &item.mask
            && current.owner == crate::item_identity::ItemRef::Layer(layer.id)
        {
            if found || !matches!(item.item, crate::item_identity::ItemRef::Mask(_)) || current.enabled != mask.enabled || current.linked != mask.linked {
                return Err(BackingError::Integrity);
            }
            found = true;
        }
    }
    if !found {
        return Err(BackingError::Integrity);
    }
    control.check(1)?;
    Ok(mask)
}
/// Bounded borrowed lookup. Physical and imported locks include every ancestor.
/// Imported Layer ItemStates must match the physical parent at each visited node.
#[derive(Clone, Copy, Debug)]
pub struct AccountedLayerAccess<'a> {
    pub layer: &'a crate::Layer,
    pub locks: crate::Locks,
    pub imported_locks: crate::xcf_state::ItemLocks,
}
pub fn accounted_layer_access<'a>(doc: &'a Document, id: LayerId, control: &ReadControl<'_>) -> Result<Option<AccountedLayerAccess<'a>>, BackingError> {
    control.check(1)?;
    if doc.xcf_state.as_ref().is_some_and(|state| state.items.len() > 4096) {
        return Err(BackingError::Length);
    }
    let mut visitor = AccessVisitor { id, state: doc.xcf_state.as_ref(), count: 0, found: None, control };
    visitor.walk(&doc.layers, crate::Locks::default(), crate::xcf_state::ItemLocks::default(), None, 0)?;
    Ok(visitor.found)
}
struct AccessVisitor<'a, 'b, 'c> {
    id: LayerId,
    state: Option<&'a crate::xcf_state::XcfState>,
    count: usize,
    found: Option<AccountedLayerAccess<'a>>,
    control: &'b ReadControl<'c>,
}
impl<'a> AccessVisitor<'a, '_, '_> {
    fn walk(
        &mut self,
        layers: &'a [crate::Layer],
        locks: crate::Locks,
        imported_locks: crate::xcf_state::ItemLocks,
        parent: Option<crate::item_identity::ItemRef>,
        depth: usize,
    ) -> Result<(), BackingError> {
        if depth > crate::MAX_GROUP_DEPTH {
            return Err(BackingError::Length);
        }
        for layer in layers {
            self.control.check(1)?;
            self.count = self.count.checked_add(1).ok_or(BackingError::Length)?;
            if self.count > 4096 {
                return Err(BackingError::Length);
            }
            let locks = locks.union(layer.locks);
            let imported_locks = imported_locks.union(imported_item_locks(self.state, layer.id, parent, self.control)?);
            if layer.id == self.id {
                if self.found.is_some() {
                    return Err(BackingError::Integrity);
                }
                self.found = Some(AccountedLayerAccess { layer, locks, imported_locks });
            }
            if let LayerContent::Group(group) = &layer.content {
                self.walk(
                    &group.children,
                    locks,
                    imported_locks,
                    Some(crate::item_identity::ItemRef::Layer(layer.id)),
                    depth.checked_add(1).ok_or(BackingError::Length)?,
                )?;
            }
        }
        Ok(())
    }
}
fn imported_item_locks(
    state: Option<&crate::xcf_state::XcfState>,
    id: LayerId,
    parent: Option<crate::item_identity::ItemRef>,
    control: &ReadControl<'_>,
) -> Result<crate::xcf_state::ItemLocks, BackingError> {
    let Some(state) = state else {
        return Ok(crate::xcf_state::ItemLocks::default());
    };
    let mut found = None;
    for (index, item) in state.items.iter().enumerate() {
        if index % 64 == 0 {
            control.check(1)?;
        }
        if item.item == crate::item_identity::ItemRef::Layer(id) {
            if found.is_some() || item.parent != parent {
                return Err(BackingError::Integrity);
            }
            found = Some(item.locks);
        }
    }
    found.ok_or(BackingError::Integrity)
}
fn find_layer_mut<'a>(
    layers: &'a mut [crate::Layer],
    id: LayerId,
    depth: usize,
    control: &ReadControl<'_>,
) -> Result<Option<&'a mut crate::Layer>, BackingError> {
    if depth > crate::MAX_GROUP_DEPTH {
        return Err(BackingError::Length);
    }
    for layer in layers {
        control.check(1)?;
        if layer.id == id {
            return Ok(Some(layer));
        }
        if let LayerContent::Group(group) = &mut layer.content
            && let Some(found) = find_layer_mut(&mut group.children, id, depth.checked_add(1).ok_or(BackingError::Length)?, control)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}
fn validate_backing(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<(), BackingError> {
    control.check(1)?;
    if doc.layers.is_empty() {
        return Err(BackingError::Length);
    }
    validate_backed_layers(&doc.layers, quota, 0, &mut 0, control)
}
fn validate_backed_layers(
    layers: &[crate::Layer],
    quota: &BackingQuota,
    depth: usize,
    count: &mut usize,
    control: &ReadControl<'_>,
) -> Result<(), BackingError> {
    if depth > crate::MAX_GROUP_DEPTH {
        return Err(BackingError::Length);
    }
    for layer in layers {
        control.check(1)?;
        *count = count.checked_add(1).ok_or(BackingError::Length)?;
        if *count > 4096 {
            return Err(BackingError::Length);
        }
        match &layer.content {
            LayerContent::BackedRaster(backed) if quota.shares_ledger(backed.quota()) => {}
            LayerContent::BackedRaster(_) => return Err(BackingError::Integrity),
            LayerContent::Group(group) if group.artboard.is_none() => {
                validate_backed_layers(&group.children, quota, depth.checked_add(1).ok_or(BackingError::Length)?, count, control)?
            }
            _ => return Err(BackingError::Unsupported),
        }
    }
    Ok(())
}
// Upstream selection commands create Gray8 coverage independently of image
// depth. Typed native-wide operations may also retain the image's exact Gray
// depth; this admission never converts or reinterprets either representation.
fn selection_format_allowed(format: photocraft_color::PixelFormat, depth: crate::SampleType) -> bool {
    format == photocraft_color::PixelFormat::GRAY8 || format == photocraft_color::PixelFormat::new(crate::ColorMode::Grayscale, depth, false)
}

/// Capacity census for an intentionally explicit bounded tree model-copy domain.
/// Owned complex collections are refused, never assigned a fabricated size.
fn census(doc: &Document, actual: bool, guard: Option<&AccountedSelection>, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    census_inner(doc, actual, false, guard, quota, control, false)
}
fn selection_census(doc: &Document, actual: bool, guard: Option<&AccountedSelection>, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    census_inner(doc, actual, imported_selection::applies(doc), guard, quota, control, false)
}
fn census_inner(
    doc: &Document,
    actual: bool,
    imported_selection: bool,
    selection_guard: Option<&AccountedSelection>,
    expected_quota: &BackingQuota,
    control: &ReadControl<'_>,
    readonly_filter_masks: bool,
) -> Result<ModelCensus, BackingError> {
    census_with_mask_policy(doc, actual, imported_selection, selection_guard, expected_quota, control,
        if readonly_filter_masks { ReadonlyMaskPolicy::Threshold } else { ReadonlyMaskPolicy::None })
}
fn point_readonly_census(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    census_with_mask_policy(doc, true, true, None, quota, control, ReadonlyMaskPolicy::Point)
}
fn census_with_mask_policy(
    doc: &Document,
    actual: bool,
    imported_selection: bool,
    selection_guard: Option<&AccountedSelection>,
    expected_quota: &BackingQuota,
    control: &ReadControl<'_>,
    readonly_masks: ReadonlyMaskPolicy,
) -> Result<ModelCensus, BackingError> {
    let crate::Metadata { xmp: _, exif: _, psd_resources: _, psd_global_blocks: _, text: _ } = &doc.metadata;
    let crate::Guides { horizontal: _, vertical: _, colors: _ } = &doc.guides;
    let crate::text_styles::TextStyles { character: _, paragraph: _, basic: _, psd_raw: _ } = &doc.text_styles;
    let crate::text_styles::ParagraphStyleDef { id: _, name: _, para_attrs: _, char_attrs: _ } = &doc.text_styles.basic;
    let crate::analysis::Measurement { scale: _, count_groups: _, active_count_group: _, ruler: _ } = &doc.measurement;
    let crate::analysis::MeasurementScale { pixel_length: _, logical_length: _, units: _ } = &doc.measurement.scale;
    let crate::slices::Slices { group_name: _, list: _ } = &doc.slices;
    let crate::variables::Variables { defs: _, data_sets: _, active: _ } = &doc.variables;
    // Exhaustive field patterns make future model additions require census review.
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
    control.check(1)?;
    if !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || !doc.patterns.is_empty()
        || doc.color_table.is_some()
        || doc.duotone.is_some()
        || !doc.layer_comps.is_empty()
        || doc.last_document_state.is_some()
        || !doc.measurement.count_groups.is_empty()
        || !doc.notes.is_empty()
        || !doc.text_styles.character.is_empty()
        || !doc.text_styles.paragraph.is_empty()
        || !doc.text_styles.basic.para_attrs.is_empty()
        || !doc.text_styles.basic.char_attrs.is_empty()
        || doc.text_styles.psd_raw.is_some()
        || !doc.slices.list.is_empty()
        || !doc.variables.defs.is_empty()
        || !doc.variables.data_sets.is_empty()
        || doc.timeline.is_some()
    {
        return Err(BackingError::Unsupported);
    }
    // Each actual selection view has its own map/default charge; only immutable
    // native tiles are physically shared through this private, genuine guard.
    match (&doc.selection, selection_guard) {
        (None, None) => {}
        (Some(surface), Some(guard)) => {
            if doc.selection_channel_id.is_some()
                || (!imported_selection && (doc.xcf_state.is_some() || doc.source_color_context.is_some()))
                || !selection_format_allowed(guard.format(), doc.depth)
                || !doc.bounds().contains_rect(&guard.bounds())
                || !guard.matches(surface, expected_quota, control)?
            {
                return Err(BackingError::Unsupported);
            }
        }
        _ => return Err(BackingError::Unsupported),
    }
    let mut size = (std::mem::size_of::<ModelOwner>() + 128) as u64;
    add_string(&mut size, &doc.name, actual)?;
    add_string(&mut size, &doc.measurement.scale.units, actual)?;
    add_string(&mut size, &doc.text_styles.basic.name, actual)?;
    add_string(&mut size, &doc.slices.group_name, actual)?;
    match &doc.working_profile_interpretation {
        WorkingProfileInterpretation::Icc => {}
        WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } | WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } => {
            add_string(&mut size, icc_blake3, actual)?
        }
    }
    if let Some(resolution) = &doc.physical_resolution
        && let crate::physical_resolution::PhysicalUnit::Custom(unit) = &resolution.unit
    {
        let crate::physical_resolution::PhysicalResolution { x_dpi: _, y_dpi: _, unit: _ } = resolution;
        let crate::physical_resolution::CustomUnit { name: _, factor: _, digits: _, symbol: _, abbreviation: _ } = unit;
        add_string(&mut size, &unit.name, actual)?;
        add_string(&mut size, &unit.symbol, actual)?;
        add_string(&mut size, &unit.abbreviation, actual)?;
    }
    if let Some(xmp) = &doc.metadata.xmp {
        add_string(&mut size, xmp, actual)?;
    }
    add_vec(&mut size, &doc.layers, actual)?;
    add_vec(&mut size, &doc.guides.horizontal, actual)?;
    add_vec(&mut size, &doc.guides.vertical, actual)?;
    add_vec(&mut size, &doc.guides.colors, actual)?;
    add_vec(&mut size, &doc.metadata.text, actual)?;
    for (key, value) in &doc.metadata.text {
        control.check(1)?;
        add_string(&mut size, key, actual)?;
        add_string(&mut size, value, actual)?;
    }
    add_vec(&mut size, &doc.metadata.psd_resources, actual)?;
    add_vec(&mut size, &doc.metadata.psd_global_blocks, actual)?;
    for (_, name, _) in &doc.metadata.psd_resources {
        control.check(1)?;
        add_string(&mut size, name, actual)?;
    }
    census_layers(&doc.layers, &mut size, actual, 0, &mut 0, &mut 0, control)?;
    if doc.icc_profile.as_ref().is_some_and(|bytes| bytes.len() > 16 << 20) {
        return Err(BackingError::Length);
    }
    if let Some(crate::SourceColorContext { original_icc, source_precision: _ }) = &doc.source_color_context
        && original_icc.as_ref().is_some_and(|bytes| bytes.len() > 16 << 20)
    {
        return Err(BackingError::Length);
    }
    let mut canonical = CanonicalCensus::new(size, actual, control);
    canonical.readonly_masks = readonly_masks;
    if imported_selection {
        imported_selection::shared_payloads(doc, &mut canonical, control)?;
    }
    if let Some(state) = &doc.xcf_state {
        canonical.state(state)?;
    }
    let counted = canonical.finish()?;
    control.check(counted.total()?)?;
    Ok(counted)
}
fn census_layers(
    layers: &[crate::Layer],
    size: &mut u64,
    actual: bool,
    depth: usize,
    count: &mut usize,
    mask_tiles: &mut usize,
    control: &ReadControl<'_>,
) -> Result<(), BackingError> {
    if depth > crate::MAX_GROUP_DEPTH {
        return Err(BackingError::Length);
    }
    for layer in layers {
        control.check(1)?;
        *count = count.checked_add(1).ok_or(BackingError::Length)?;
        if *count > 4096 {
            return Err(BackingError::Length);
        }
        let crate::Effects { enabled: _, items: _, psd_raw: _, reference: _ } = &layer.effects;
        let crate::blend_if::BlendIf { ranges: _ } = &layer.blend_if;
        let crate::Layer {
            indexed_source: _,
            import_compositing: _,
            id: _,
            name: _,
            visible: _,
            locks: _,
            blend: _,
            opacity: _,
            fill_opacity: _,
            clipped: _,
            mask: _,
            vector_mask: _,
            effects: _,
            label: _,
            content: _,
            psd_blocks: _,
            psd_id: _,
            fill_cache: _,
            link_group: _,
            excluded_channels: _,
            blend_if: _,
            advanced: _,
            video: _,
        } = layer;
        if layer.indexed_source.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
        {
            return Err(BackingError::Unsupported);
        }
        if let Some(mask) = &layer.mask {
            validate_shared_mask(mask, mask_tiles, control)?;
        }
        add_string(size, &layer.name, actual)?;
        add_vec(size, &layer.psd_blocks, actual)?;
        add_vec(size, &layer.blend_if.ranges, actual)?;

        match &layer.content {
            LayerContent::BackedRaster(_) => {}
            LayerContent::Group(crate::Group { children, expanded: _, artboard: None }) => {
                add_vec(size, children, actual)?;
                census_layers(children, size, actual, depth.checked_add(1).ok_or(BackingError::Length)?, count, mask_tiles, control)?;
            }
            _ => return Err(BackingError::Unsupported),
        }
    }
    Ok(())
}
fn add_string(size: &mut u64, value: &String, actual: bool) -> Result<(), BackingError> {
    if value.len() > 1 << 20 {
        return Err(BackingError::Length);
    }
    *size = size.checked_add(if actual { value.capacity() } else { value.len() } as u64).ok_or(BackingError::Length)?;
    Ok(())
}
fn add_vec<T>(size: &mut u64, value: &Vec<T>, actual: bool) -> Result<(), BackingError> {
    if value.len() > 4096 {
        return Err(BackingError::Length);
    }
    let bytes = (if actual { value.capacity() } else { value.len() }).checked_mul(std::mem::size_of::<T>()).ok_or(BackingError::Length)?;
    *size = size.checked_add(bytes as u64).ok_or(BackingError::Length)?;
    Ok(())
}

/// Sharing an immutable mask creates no BTreeMap/default/tile allocation. The
/// codec-owned retention guard covers the original physical Surface; only the
/// inline Arc handle is copied with Layer. This admits ownership, not rendering
/// semantics or unaccounted generic mask mutation.
fn validate_shared_mask(mask: &crate::LayerMask, total_tiles: &mut usize, control: &ReadControl<'_>) -> Result<(), BackingError> {
    let crate::LayerMask { surface, enabled: _, linked: _, density: _, feather: _ } = mask;
    let format = surface.format();
    if format.mode != crate::ColorMode::Grayscale || format.alpha {
        return Err(BackingError::Unsupported);
    }
    let expected = usize::try_from(photocraft_geom::TILE_SIZE)
        .ok()
        .and_then(|side| side.checked_mul(side))
        .and_then(|pixels| pixels.checked_mul(format.bytes_per_pixel()))
        .ok_or(BackingError::Length)?;
    if surface.default_bytes().len() != format.bytes_per_pixel() || surface.tile_count() > 4096 {
        return Err(BackingError::Length);
    }
    *total_tiles = total_tiles.checked_add(surface.tile_count()).ok_or(BackingError::Length)?;
    if *total_tiles > 100_000 {
        return Err(BackingError::Length);
    }
    control.check(1)?;
    for (coordinate, tile) in surface.tiles() {
        control.check(1)?;
        for value in [coordinate.tx, coordinate.ty] {
            let origin = value.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
            let end = origin.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
            if origin < -(1 << 29) || end > 1 << 29 {
                return Err(BackingError::Bounds);
            }
        }
        if tile.bytes().len() != expected {
            return Err(BackingError::Integrity);
        }
    }
    Ok(())
}

fn sync_current_item(document: &mut Document, edit: AccountedEdit<'_>) -> Result<(), BackingError> {
    let Some(state) = &mut document.xcf_state else { return Ok(()) };
    let (id, visible, name) = match edit {
        AccountedEdit::LayerMetadata { .. } => return Err(BackingError::Unsupported),
        AccountedEdit::LayerVisibility { layer, visible } => (layer, Some(visible), None),
        AccountedEdit::LayerName { layer, name } => (layer, None, Some(name)),
        AccountedEdit::MaskEnabled { layer, enabled } => {
            let mask = state
                .items
                .iter_mut()
                .filter_map(|item| item.mask.as_mut())
                .find(|mask| mask.owner == crate::item_identity::ItemRef::Layer(layer))
                .ok_or(BackingError::Integrity)?;
            mask.enabled = enabled;
            return Ok(());
        }
        AccountedEdit::MaskLinked { layer, linked } => {
            let mask = state
                .items
                .iter_mut()
                .filter_map(|item| item.mask.as_mut())
                .find(|mask| mask.owner == crate::item_identity::ItemRef::Layer(layer))
                .ok_or(BackingError::Integrity)?;
            mask.linked = linked;
            return Ok(());
        }
        AccountedEdit::LayerOpacity { .. } | AccountedEdit::NativePixels { .. } => return Ok(()),
    };
    if let Some(item) = state.items.iter_mut().find(|item| item.item == crate::item_identity::ItemRef::Layer(id)) {
        if let Some(visible) = visible {
            item.visible = visible;
        }
        if let Some(name) = name {
            let mut owned = String::new();
            owned.try_reserve_exact(name.len()).map_err(|_| BackingError::Allocation)?;
            owned.push_str(name);
            item.name = owned;
            item.custom_name = true;
        }
    }
    Ok(())
}

/// Capacity accounting only: this neither validates relationships nor qualifies
/// an evaluator. The codec/importer retains the original source and validation
/// evidence; clones copy owned buffers and share immutable Arc payloads.
#[derive(Clone, Copy)]
enum ReadonlyMaskPolicy { None, Threshold, Point }
struct CanonicalCensus<'a, 'i> {
    readonly_masks: ReadonlyMaskPolicy,
    counted: ModelCensus,
    text: u64,
    resources: u64,
    visits: usize,
    actual: bool,
    control: &'a ReadControl<'i>,
}
impl<'a, 'i> CanonicalCensus<'a, 'i> {
    fn new(index: u64, actual: bool, control: &'a ReadControl<'i>) -> Self {
        Self { readonly_masks: ReadonlyMaskPolicy::None, counted: ModelCensus { index, payload: 0 }, text: 0, resources: 0, visits: 0, actual, control }
    }
    fn visit(&mut self) -> Result<(), BackingError> {
        self.visits = self.visits.checked_add(1).ok_or(BackingError::Length)?;
        if self.visits > 100_000 {
            return Err(BackingError::Length);
        }
        self.control.check(1)
    }
    fn string(&mut self, value: &String) -> Result<(), BackingError> {
        self.visit()?;
        if value.len() > 1 << 20 {
            return Err(BackingError::Length);
        }
        let bytes = (if self.actual { value.capacity() } else { value.len() }) as u64;
        self.text = self.text.checked_add(bytes).ok_or(BackingError::Length)?;
        self.counted.index = self.counted.index.checked_add(bytes).ok_or(BackingError::Length)?;
        if self.text > 64 << 20 {
            return Err(BackingError::Length);
        }
        Ok(())
    }
    fn optional(&mut self, value: &Option<String>) -> Result<(), BackingError> {
        if let Some(value) = value {
            self.string(value)?;
        }
        Ok(())
    }
    fn vec<T>(&mut self, value: &Vec<T>, maximum: usize) -> Result<(), BackingError> {
        self.visit()?;
        if value.len() > maximum {
            return Err(BackingError::Length);
        }
        let bytes = (if self.actual { value.capacity() } else { value.len() }).checked_mul(std::mem::size_of::<T>()).ok_or(BackingError::Length)?;
        self.counted.index = self.counted.index.checked_add(bytes as u64).ok_or(BackingError::Length)?;
        Ok(())
    }
    fn blob(&mut self, value: &Vec<u8>, resource: bool) -> Result<(), BackingError> {
        self.visit()?;
        if value.len() > 64 << 20 {
            return Err(BackingError::Length);
        }
        let bytes = (if self.actual { value.capacity() } else { value.len() }) as u64;
        self.counted.payload = self.counted.payload.checked_add(bytes).ok_or(BackingError::Length)?;
        if resource {
            self.resources = self.resources.checked_add(bytes).ok_or(BackingError::Length)?;
        }
        Ok(())
    }
    fn composition(&mut self, value: &crate::import_compositing::GimpComposition) -> Result<(), BackingError> {
        let crate::import_compositing::GimpComposition {
            compatibility_epoch,
            source_mode: _,
            legacy: _,
            stored_blend_space: _,
            stored_composite_space: _,
            stored_operator: _,
            resolved: _,
            pass_through: _,
        } = value;
        self.string(compatibility_epoch)
    }
    fn operation(&mut self, value: &crate::xcf_filters::OperationIdentity) -> Result<(), BackingError> {
        let crate::xcf_filters::OperationIdentity { name, stored_version, schema_epoch } = value;
        self.string(name)?;
        self.string(stored_version)?;
        self.string(schema_epoch)
    }
    fn arguments(&mut self, values: &Vec<crate::xcf_filters::FilterArgument>, depth: usize) -> Result<(), BackingError> {
        use crate::xcf_filters::{FilterArgument, FilterValue};
        if depth > 32 {
            return Err(BackingError::Length);
        }
        self.vec(values, 100_000)?;
        for FilterArgument { name, value } in values {
            self.string(name)?;
            match value {
                FilterValue::Number(_) | FilterValue::Bool(_) | FilterValue::Resource(_) => {}
                FilterValue::String(s) => self.string(s)?,
                FilterValue::Enum { domain, value: _ } => self.string(domain)?,
                FilterValue::Color { components, encoding: _, profile: _ } => self.vec(components, 16)?,
                FilterValue::StoredColor(crate::xcf_filters::ExactColorStorage { layout, samples, opaque_tail }) => {
                    if !crate::xcf_filters::stored_color_shape(layout, samples, opaque_tail) {
                        return Err(BackingError::Length);
                    }
                    self.string(layout)?;
                    self.vec(samples, 1024)?;
                    self.vec(opaque_tail, 1 << 20)?;
                    for chunk in samples.chunks(32 << 10).chain(opaque_tail.chunks(32 << 10)) {
                        self.control.check(chunk.len() as u64)?;
                    }
                }
                FilterValue::Curve { free: _, points_f64_bits, point_types, samples_f64_bits } => {
                    self.vec(points_f64_bits, 100_000)?;
                    self.vec(point_types, 100_000)?;
                    self.vec(samples_f64_bits, 100_000)?;
                }
                FilterValue::Config(config) => self.arguments(config, depth.checked_add(1).ok_or(BackingError::Length)?)?,
                FilterValue::Graph(graph) => self.graph(graph, depth.checked_add(1).ok_or(BackingError::Length)?)?,
            }
        }
        Ok(())
    }
    fn graph(&mut self, value: &crate::xcf_filters::FilterGraph, depth: usize) -> Result<(), BackingError> {
        use crate::xcf_filters::{FilterGraph, GraphEdge, GraphNode};
        if depth > 32 {
            return Err(BackingError::Length);
        }
        let FilterGraph { input: _, nodes, edges, output: _, original_pipeline } = value;
        self.vec(nodes, 4096)?;
        self.vec(edges, 16384)?;
        self.optional(original_pipeline)?;
        for GraphNode { id: _, operation, version_origin: _, arguments } in nodes {
            self.operation(operation)?;
            self.arguments(arguments, depth.checked_add(1).ok_or(BackingError::Length)?)?;
        }
        for GraphEdge { from: _, output_port, to: _, input_port } in edges {
            self.string(output_port)?;
            self.string(input_port)?;
        }
        Ok(())
    }
    fn state(&mut self, state: &crate::xcf_state::XcfState) -> Result<(), BackingError> {
        use crate::{xcf_editing::*, xcf_filters::*, xcf_items::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
        let XcfState {
            provenance,
            items,
            resources,
            source_items,
            editable_indexed,
            filter_stacks,
            current_filter_output_precision: _,
            filter_masks,
            floating: _,
            selection,
            item_sets,
            opaque,
        } = state;
        // These contain independent Surface/private-map payloads not yet covered
        // by this ownership contract. Refuse them before Document::clone.
        if !editable_indexed.is_empty() {
            return Err(BackingError::Unsupported);
        }
        match self.readonly_masks {
            ReadonlyMaskPolicy::None if !filter_masks.is_empty() => return Err(BackingError::Unsupported),
            ReadonlyMaskPolicy::None => {},
            ReadonlyMaskPolicy::Threshold => filter_readonly::mask_census(filter_masks, self)?,
            ReadonlyMaskPolicy::Point => point_readonly::mask_census(filter_masks, self)?,
        }
        let ProjectProvenance {
            format,
            format_version: _,
            container,
            producer,
            release_id,
            source_label,
            source_hash,
            original_locator,
            protected_source_fingerprints,
        } = provenance;
        self.string(format)?;
        self.string(container)?;
        self.optional(producer)?;
        self.optional(release_id)?;
        self.string(source_label)?;
        self.optional(source_hash)?;
        self.optional(original_locator)?;
        self.vec(protected_source_fingerprints, 4096)?;
        for value in protected_source_fingerprints {
            self.string(value)?;
        }
        self.vec(items, 4096)?;
        for ItemState {
            item: _,
            name,
            custom_name: _,
            visible: _,
            locks: _,
            parent: _,
            source_tattoo: _,
            source_flags: _,
            raster_bounds: _,
            mask: _,
            composition,
        } in items
        {
            self.string(name)?;
            if let Some(value) = composition {
                self.composition(value)?;
            }
        }
        self.vec(resources, 4096)?;
        for ResourceReference { id: _, kind, name, collection, internal: _, locator, content_hash, owned_bytes, resolution } in resources {
            self.string(name)?;
            self.optional(collection)?;
            self.optional(locator)?;
            self.optional(content_hash)?;
            if let Some(bytes) = owned_bytes {
                if matches!(kind, ResourceKind::ColorProfile) && bytes.len() > 16 << 20 {
                    return Err(BackingError::Length);
                }
                self.blob(bytes, true)?;
            }
            if let Some(ResourceResolution { fidelity: _, payload_hash }) = resolution {
                self.string(payload_hash)?;
            }
        }
        self.vec(source_items, 4096)?;
        for SourceItem { owner: _, source, transform: _, cache } in source_items {
            match source {
                EditableSource::Text { content, markup, font: _, direction: _, source_modified: _, configuration, current_origin: _ } => {
                    self.string(content)?;
                    self.optional(markup)?;
                    self.arguments(configuration, 0)?;
                }
                EditableSource::Vector { path: _, source_modified: _, configuration } => self.arguments(configuration, 0)?,
                EditableSource::Linked { resource: _, last_verified_hash, configuration, current: _ } => {
                    self.optional(last_verified_hash)?;
                    self.arguments(configuration, 0)?;
                }
            }
            if let Some(SourceCache { bytes, blake3, width: _, height: _, format: _ }) = cache {
                self.blob(bytes, false)?;
                self.string(blake3)?;
            }
        }
        self.vec(filter_stacks, 4096)?;
        for FilterStack { owner: _, nodes, revision: _ } in filter_stacks {
            self.vec(nodes, 4096)?;
            for FilterNode {
                id: _,
                operation,
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
                self.operation(operation)?;
                self.optional(source_display_name)?;
                self.optional(source_icon)?;
                self.arguments(arguments, 0)?;
                self.vec(auxiliary, 4096)?;
                for (name, _) in auxiliary {
                    self.string(name)?;
                }
                if let Some(composition) = composition {
                    self.composition(composition)?;
                }
            }
        }
        let EditingSelection { selected, active: _, selected_paths, mask_target: _ } = selection;
        self.vec(selected, 4096)?;
        self.vec(selected_paths, 4096)?;
        self.vec(item_sets, 4096)?;
        for ItemSet { id: _, name, family: _, rule, members } in item_sets {
            self.string(name)?;
            self.vec(members, 4096)?;
            match rule {
                ItemSetRule::Named => {}
                ItemSetRule::Text(s) | ItemSetRule::Regex(s) | ItemSetRule::Glob(s) => self.string(s)?,
            }
        }
        self.vec(opaque, 4096)?;
        for OpaqueRecord { owner: _, namespace, identity, source_version: _, order: _, flags: _, bytes, blake3, rendering_required: _ } in opaque {
            self.string(namespace)?;
            self.string(identity)?;
            self.blob(bytes, false)?;
            self.string(blake3)?;
        }
        Ok(())
    }
    fn finish(self) -> Result<ModelCensus, BackingError> {
        let index_maximum = if cfg!(target_arch = "wasm32") { 16u64 << 20 } else { 64u64 << 20 };
        let resource_maximum = if cfg!(target_arch = "wasm32") { 32u64 << 20 } else { 128u64 << 20 };
        let total_maximum = if cfg!(target_arch = "wasm32") { 128u64 << 20 } else { 256u64 << 20 };
        if self.counted.index > index_maximum || self.resources > resource_maximum || self.counted.total()? > total_maximum {
            return Err(BackingError::Length);
        }
        Ok(self.counted)
    }
}

// Exhaustive private clone: every other field keeps exactly its ordinary Clone
// semantics. Selection is separately installed through its charged/polled view.
fn clone_model_without_selection(doc: &Document) -> Document {
    let Document {
        physical_resolution,
        xcf_state,
        sample_encoding,
        source_color_context,
        working_profile_interpretation,
        id,
        name,
        size,
        resolution_dpi,
        mode,
        depth,
        icc_profile,
        layers,
        channels,
        guides,
        selection: _,
        selection_channel_id,
        metadata,
        global_light,
        paths,
        work_path,
        clipping_path,
        quick_mask,
        quick_mask_unselected,
        patterns,
        color_table,
        duotone,
        layer_comps,
        last_applied_comp,
        last_document_state,
        measurement,
        notes,
        text_styles,
        slices,
        variables,
        timeline,
        file_encoding,
        ruler_origin,
    } = doc;
    Document {
        physical_resolution: physical_resolution.clone(),
        xcf_state: xcf_state.clone(),
        sample_encoding: sample_encoding.clone(),
        source_color_context: source_color_context.clone(),
        working_profile_interpretation: working_profile_interpretation.clone(),
        id: id.clone(),
        name: name.clone(),
        size: size.clone(),
        resolution_dpi: resolution_dpi.clone(),
        mode: mode.clone(),
        depth: depth.clone(),
        icc_profile: icc_profile.clone(),
        layers: layers.clone(),
        channels: channels.clone(),
        guides: guides.clone(),
        selection: None,
        selection_channel_id: selection_channel_id.clone(),
        metadata: metadata.clone(),
        global_light: global_light.clone(),
        paths: paths.clone(),
        work_path: work_path.clone(),
        clipping_path: clipping_path.clone(),
        quick_mask: quick_mask.clone(),
        quick_mask_unselected: quick_mask_unselected.clone(),
        patterns: patterns.clone(),
        color_table: color_table.clone(),
        duotone: duotone.clone(),
        layer_comps: layer_comps.clone(),
        last_applied_comp: last_applied_comp.clone(),
        last_document_state: last_document_state.clone(),
        measurement: measurement.clone(),
        notes: notes.clone(),
        text_styles: text_styles.clone(),
        slices: slices.clone(),
        variables: variables.clone(),
        timeline: timeline.clone(),
        file_encoding: file_encoding.clone(),
        ruler_origin: ruler_origin.clone(),
    }
}
