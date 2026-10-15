//! One paid CURRENT layer-properties copy for genuine readonly native storage.
//! No generic metadata, pixel, profile, source or renderer authority follows.
use super::*;
use crate::item_identity::ItemRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadonlyLayerPropertiesPatch<'a> {
    pub layer: LayerId,
    pub name: Option<&'a str>,
    pub visible: Option<bool>,
    pub opacity_f32_bits: Option<u32>,
}
impl AccountedDocument {
    /// Apply the supported fields together, then synchronize canonical item
    /// name/visibility as ordinary layer.setProps reconciliation does. None
    /// preserves the Layer field; exact current no-ops share the original owner.
    /// Effective command locks, source/view fences and history remain external.
    pub fn try_with_readonly_layer_properties(&self, patch: ReadonlyLayerPropertiesPatch<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        match self.owner._storage_class {
            StorageClass::ReadonlyThresholdAlpha => filter_readonly::layer_properties(self, patch, control),
            StorageClass::ReadonlyHistogramThreshold => threshold_readonly::layer_properties(self, patch, control),
            StorageClass::Legacy | StorageClass::ReadonlyPoint => Err(BackingError::Unsupported),
        }
    }
}

pub(super) fn validate(patch: ReadonlyLayerPropertiesPatch<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
    if let Some(bits) = patch.opacity_f32_bits {
        let value = f32::from_bits(bits);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(BackingError::Bounds);
        }
    }
    if let Some(name) = patch.name {
        imported_edits::validate_name(name, control)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct Plan<'a> {
    layer: LayerId,
    name: Option<ReadonlyLayerNamePatch<'a>>,
    visible: bool,
    opacity_bits: u32,
}
impl Plan<'_> {
    pub(super) fn replacement_name_bytes(self) -> Result<u64, BackingError> {
        // The enclosing unchanged envelope doubles this value: two new names,
        // each with capacity at most twice its length, precede old-name drops.
        self.name.map_or(Ok(0), |p| (p.name.len() as u64).checked_mul(2).ok_or(BackingError::Length))
    }
}

pub(super) fn plan<'a>(doc: &'a Document, patch: ReadonlyLayerPropertiesPatch<'a>, control: &ReadControl<'_>) -> Result<Option<Plan<'a>>, BackingError> {
    let layer = doc.layers.first().filter(|layer| layer.id == patch.layer).ok_or(BackingError::Bounds)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut item = None;
    for candidate in &state.items {
        control.check(1)?;
        if candidate.item == ItemRef::Layer(patch.layer) {
            if item.is_some() {
                return Err(BackingError::Integrity);
            }
            item = Some(candidate);
        }
    }
    let item = item.ok_or(BackingError::Integrity)?;
    let name = patch.name.unwrap_or(&layer.name);
    let name_equal = imported_edits::names_equal(&layer.name, name, control)?;
    let item_name_equal = imported_edits::names_equal(&item.name, name, control)?;
    let visible = patch.visible.unwrap_or(layer.visible);
    let opacity_bits = patch.opacity_f32_bits.unwrap_or(layer.opacity.to_bits());
    if name_equal && item_name_equal && layer.visible == visible && item.visible == visible && layer.opacity.to_bits() == opacity_bits {
        return Ok(None);
    }
    Ok(Some(Plan {
        layer: patch.layer,
        name: (!name_equal || !item_name_equal).then_some(ReadonlyLayerNamePatch { layer: patch.layer, name }),
        visible,
        opacity_bits,
    }))
}

pub(super) fn apply(doc: &mut Document, plan: Plan<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
    if let Some(name) = plan.name {
        readonly_layer_name::apply(doc, name, control)?;
    }
    control.check(1)?;
    let layer = doc.layers.first_mut().filter(|layer| layer.id == plan.layer).ok_or(BackingError::Bounds)?;
    layer.visible = plan.visible;
    layer.opacity = f32::from_bits(plan.opacity_bits);
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let mut item = None;
    for candidate in &mut state.items {
        control.check(1)?;
        if candidate.item == ItemRef::Layer(plan.layer) {
            if item.is_some() {
                return Err(BackingError::Integrity);
            }
            item = Some(candidate);
        }
    }
    item.ok_or(BackingError::Integrity)?.visible = plan.visible;
    Ok(())
}
