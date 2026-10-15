//! Explicit closed Histogram Threshold storage. No source, profile, renderer,
//! generic metadata/pixel or selection editing authority is granted.
use super::filter_readonly::MaskCopyPlan;
use super::*;
use crate::{
    FilterMask,
    xcf_filters::{ExactNumber, FilterStack, FilterValue},
};
#[path = "accounted_threshold_readonly_patch.rs"]
mod patch;
pub use patch::ThresholdPatch;
const EPOCH: &str = "xcf-22-stock-threshold-precision-v1";
const OPAQUE: i32 = -1_565_818_287;
fn unsupported<T>() -> Result<T, BackingError> {
    Err(BackingError::Unsupported)
}
fn node_shape(node: &crate::xcf_filters::FilterNode) -> Result<(), BackingError> {
    if node.id.0 == 0
        || node.operation.name != "gimp:threshold"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
        || node.arguments.len() != 3
        || !node.auxiliary.is_empty()
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
    {
        return unsupported();
    }
    let outer = node.composition.as_ref().ok_or(BackingError::Unsupported)?;
    if outer.compatibility_epoch != EPOCH
        || outer.source_mode != 62
        || outer.legacy
        || outer.pass_through
        || outer.resolved.is_some()
        || [outer.stored_blend_space, outer.stored_composite_space, outer.stored_operator] != [OPAQUE; 3]
    {
        return unsupported();
    }
    let mut seen = 0u8;
    for argument in &node.arguments {
        let flag = match (argument.name.as_str(), &argument.value) {
            ("channel", FilterValue::Enum { domain, value }) if domain == "GimpHistogramChannel" && (0..=5).contains(value) => 1,
            (name @ ("low" | "high"), FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                let value = f32::from_bits(*bits);
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return unsupported();
                }
                if name == "low" { 2 } else { 4 }
            }
            _ => return unsupported(),
        };
        if seen & flag != 0 {
            return Err(BackingError::Integrity);
        }
        seen |= flag;
    }
    if seen != 7 {
        return Err(BackingError::Integrity);
    }
    Ok(())
}
fn measured(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    control.check(1)?;
    if doc.size != crate::Size::new(8, 6)
        || doc.mode != crate::ColorMode::Rgb
        || doc.sample_encoding != Some(crate::SampleEncoding::NonlinearProfile)
        || !matches!(
            (doc.source_color_context.as_ref().map(|v| v.source_precision), doc.depth),
            (Some(250), crate::SampleType::U16) | (Some(650), crate::SampleType::F32) | (Some(750), crate::SampleType::F64)
        )
        || doc.xcf_state.as_ref().is_none_or(|state| state.filter_masks.len() != 1)
    {
        return unsupported();
    }
    filter_readonly::physical_shape(doc, quota, control, |node| {
        control.check(3)?;
        node_shape(node)
    })?;
    // Original codec ownership retains all existing raw/profile/tile payloads.
    // The census measures actual current capacities, including three arguments.
    census_inner(doc, true, true, None, quota, control, true)
}
impl AccountedDocument {
    /// Additional owner-header Index admission only. The caller must retain all
    /// actual existing model/native/default/map/source charges on this quota.
    /// Performs complete class/census validation; no source or render proof.
    pub fn readonly_threshold_storage_requirements(document: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(document, quota, control)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    /// Trusted codec-owned storage adoption only. Retention must already own
    /// every existing model, native, source, mask-map and default allocation on
    /// this actual quota. No original-source, profile or renderer proof follows.
    pub fn new_readonly_threshold_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        let required = Self::readonly_threshold_storage_requirements(&incoming.document, &quota, control)?;
        let allocation = quota.reserve(QuotaKind::Index, required).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyHistogramThreshold,
                document,
                _selection: None,
                _selection_view: None,
                retention,
                quota,
                _model: None,
                _allocation: Arc::new(allocation),
            }),
        })
    }
    /// Runtime storage routing only; validation remains mandatory.
    pub fn is_readonly_threshold_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyHistogramThreshold
    }
    pub fn validate_readonly_threshold_storage(&self, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_threshold_storage() {
            return unsupported();
        }
        measured(self.document(), self.quota(), control)?;
        Ok(())
    }
}

fn copy_node(node: &crate::xcf_filters::FilterNode, c: &ReadControl<'_>) -> Result<crate::xcf_filters::FilterNode, BackingError> {
    use crate::xcf_filters::{FilterArgument, FilterNode, OperationIdentity};
    let FilterNode {
        id,
        operation,
        source_display_name,
        source_icon,
        arguments,
        enabled,
        opacity_f64_bits,
        source_clipping,
        source_region,
        mask,
        auxiliary: _,
        composition,
    } = node;
    let mut copied = Vec::new();
    copied.try_reserve_exact(3).map_err(|_| BackingError::Allocation)?;
    if copied.capacity() > 6 {
        return Err(BackingError::Allocation);
    }
    for argument in arguments {
        c.check(1)?;
        let value = match &argument.value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) => FilterValue::Number(ExactNumber::Float32Bits(*bits)),
            FilterValue::Enum { domain, value } => FilterValue::Enum { domain: imported_edits::copy_name(domain, c)?, value: *value },
            _ => return Err(BackingError::Unsupported),
        };
        copied.push(FilterArgument { name: imported_edits::copy_name(&argument.name, c)?, value });
    }
    let composition = composition
        .as_ref()
        .map(|v| {
            let crate::import_compositing::GimpComposition {
                compatibility_epoch,
                source_mode,
                legacy,
                stored_blend_space,
                stored_composite_space,
                stored_operator,
                resolved,
                pass_through,
            } = v;
            Ok::<_, BackingError>(crate::import_compositing::GimpComposition {
                compatibility_epoch: imported_edits::copy_name(compatibility_epoch, c)?,
                source_mode: *source_mode,
                legacy: *legacy,
                stored_blend_space: *stored_blend_space,
                stored_composite_space: *stored_composite_space,
                stored_operator: *stored_operator,
                resolved: resolved.clone(),
                pass_through: *pass_through,
            })
        })
        .transpose()?;
    Ok(FilterNode {
        id: *id,
        operation: OperationIdentity {
            name: imported_edits::copy_name(&operation.name, c)?,
            stored_version: imported_edits::copy_name(&operation.stored_version, c)?,
            schema_epoch: imported_edits::copy_name(&operation.schema_epoch, c)?,
        },
        source_display_name: source_display_name.as_deref().map(|v| imported_edits::copy_name(v, c)).transpose()?,
        source_icon: source_icon.as_deref().map(|v| imported_edits::copy_name(v, c)).transpose()?,
        arguments: copied,
        enabled: *enabled,
        opacity_f64_bits: *opacity_f64_bits,
        source_clipping: *source_clipping,
        source_region: *source_region,
        mask: *mask,
        auxiliary: Vec::new(),
        composition,
    })
}

struct OriginalRetention {
    original: AccountedDocument,
}
#[derive(Clone, Copy)]
enum CopyKind<'a> {
    Rename(&'a str),
    LayerName(ReadonlyLayerNamePatch<'a>),
    Threshold { patch: ThresholdPatch, revision: u64 },
}
pub(super) fn layer_named(owner: &AccountedDocument, patch: ReadonlyLayerNamePatch<'_>, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    imported_edits::validate_name(patch.name, control)?;
    let logical = measured(owner.document(), owner.quota(), control)?;
    if !readonly_layer_name::changed(owner.document(), patch, control)? {
        control.check(1)?;
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::LayerName(patch), control)
}
pub(super) fn renamed(owner: &AccountedDocument, name: &str, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    imported_edits::validate_name(name, control)?;
    if name.len() > 4096 {
        return Err(BackingError::Length);
    }
    let logical = measured(owner.document(), owner.quota(), control)?;
    if imported_edits::names_equal(&owner.document().name, name, control)? {
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::Rename(name), control)
}
fn current_copy(owner: &AccountedDocument, logical: ModelCensus, kind: CopyKind<'_>, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    let replacement_name_bytes = match kind {
        CopyKind::Rename(name) => name.len() as u64,
        // Two replacement Strings may each reserve up to twice their length.
        CopyKind::LayerName(patch) => (patch.name.len() as u64).checked_mul(2).ok_or(BackingError::Length)?,
        CopyKind::Threshold { .. } => 0,
    };
    let owner_bytes = AccountedDocument::owned_model_reservation_bytes();
    let envelope = logical
        .index
        .checked_mul(2)
        .and_then(|n| n.checked_add(replacement_name_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(4096 + std::mem::size_of::<OriginalRetention>() as u64 + 128))
        .ok_or(BackingError::Length)?;
    let allocation = owner.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
    let mut index = owner.quota().reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let payload = owner.quota().reserve(QuotaKind::Resident, 0).map_err(|_| BackingError::Quota)?;
    control.check(logical.total()?)?;
    let mut document = imported_selection::copy_document(owner.document(), control)?;
    let source = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let destination = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    destination.filter_stacks.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    let stack = source.filter_stacks.first().ok_or(BackingError::Integrity)?;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    if nodes.capacity() > 2 || destination.filter_stacks.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    nodes.push(copy_node(stack.nodes.first().ok_or(BackingError::Integrity)?, control)?);
    destination.filter_stacks.push(FilterStack { owner: stack.owner, nodes, revision: stack.revision });
    destination.filter_masks.try_reserve_exact(source.filter_masks.len()).map_err(|_| BackingError::Allocation)?;
    if destination.filter_masks.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    for mask in &source.filter_masks {
        let plan = MaskCopyPlan::new(&mask.surface, mask.bounds, control)?;
        destination.filter_masks.push(FilterMask { id: mask.id, owner: mask.owner, bounds: mask.bounds, surface: plan.copy(&mask.surface, control)? });
    }
    match kind {
        CopyKind::Rename(name) => document.name = imported_edits::copy_name(name, control)?,
        CopyKind::LayerName(patch) => readonly_layer_name::apply(&mut document, patch, control)?,
        CopyKind::Threshold { patch, revision } => {
            let state = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
            let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
            let node = stack.nodes.first_mut().ok_or(BackingError::Integrity)?;
            patch::apply(node, patch, control)?;
            stack.revision = revision;
        }
    }
    let actual =
        measured(&document, owner.quota(), control)?.index.checked_add(std::mem::size_of::<OriginalRetention>() as u64 + 128).ok_or(BackingError::Length)?;
    if actual > envelope {
        return Err(BackingError::Allocation);
    }
    index.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
    control.check(1)?;
    // Flatten repeated title/current-node changes to the original real owner rather than form
    // an unbounded recursive retention/drop chain. Native tile Arc identity is
    // unchanged; no second tile payload receipt is fabricated for those Arcs.
    let original = owner.owner.retention.downcast_ref::<OriginalRetention>().map_or_else(|| owner.clone(), |r| r.original.clone());
    let retention = Arc::new(OriginalRetention { original });
    Ok(AccountedDocument {
        owner: Arc::new(ModelOwner {
            _storage_class: StorageClass::ReadonlyHistogramThreshold,
            document,
            _selection: None,
            _selection_view: None,
            retention,
            quota: owner.quota().clone(),
            _model: Some(ModelCharges { _index: index, _payload: payload }),
            _allocation: Arc::new(allocation),
        }),
    })
}
#[cfg(test)]
#[path = "accounted_threshold_readonly_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "accounted_threshold_readonly_patch_tests.rs"]
mod patch_tests;
