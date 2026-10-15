//! Typed metadata-only edits for the separate current Histogram Threshold class.
//! Locks and current rendering/source/profile qualification belong to the caller.
use super::*;
use crate::{
    LayerId,
    item_identity::{FilterId, ItemRef},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThresholdPatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub low_f32_bits: Option<u32>,
    pub high_f32_bits: Option<u32>,
    pub channel: Option<i32>,
    pub enabled: Option<bool>,
}
impl AccountedDocument {
    /// Copy only named exact Float32 thresholds, the existing histogram-channel
    /// enum value and activation. All source/native/mask words remain retained.
    /// This exposes no general model callback or source/profile/render authority.
    pub fn try_with_threshold(&self, patch: ThresholdPatch, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if self.owner._storage_class != StorageClass::ReadonlyHistogramThreshold {
            return unsupported();
        }
        for bits in [patch.low_f32_bits, patch.high_f32_bits].into_iter().flatten() {
            let value = f32::from_bits(bits);
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return unsupported();
            }
        }
        if patch.channel.is_some_and(|channel| !(0..=5).contains(&channel)) {
            return unsupported();
        }
        let logical = measured(self.document(), self.quota(), control)?;
        let layer = self.document().layers.first().ok_or(BackingError::Integrity)?;
        let state = self.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
        let node = stack.nodes.first().ok_or(BackingError::Integrity)?;
        control.check(1)?;
        if patch.layer != layer.id || stack.owner != ItemRef::Layer(patch.layer) || patch.filter != node.id || patch.expected_stack_revision != stack.revision {
            return Err(BackingError::Integrity);
        }
        let mut changed = patch.enabled.is_some_and(|value| value != node.enabled);
        for argument in &node.arguments {
            control.check(1)?;
            changed |= match (argument.name.as_str(), &argument.value) {
                ("low", FilterValue::Number(ExactNumber::Float32Bits(bits))) => patch.low_f32_bits.is_some_and(|value| value != *bits),
                ("high", FilterValue::Number(ExactNumber::Float32Bits(bits))) => patch.high_f32_bits.is_some_and(|value| value != *bits),
                ("channel", FilterValue::Enum { domain, value }) if domain == "GimpHistogramChannel" => patch.channel.is_some_and(|next| next != *value),
                _ => return Err(BackingError::Integrity),
            };
        }
        if !changed {
            control.check(1)?;
            return Ok(self.clone());
        }
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        current_copy(self, logical, CopyKind::Threshold { patch, revision }, control)
    }
}

// Called only for a newly owned, precharged metadata copy. Original argument
// positions, enum domain, Float32 words and unrelated fields are never rebuilt.
pub(super) fn apply(node: &mut crate::xcf_filters::FilterNode, patch: ThresholdPatch, control: &ReadControl<'_>) -> Result<(), BackingError> {
    for argument in &mut node.arguments {
        control.check(1)?;
        match (argument.name.as_str(), &mut argument.value) {
            ("low", FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                if let Some(next) = patch.low_f32_bits {
                    *bits = next;
                }
            }
            ("high", FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                if let Some(next) = patch.high_f32_bits {
                    *bits = next;
                }
            }
            ("channel", FilterValue::Enum { domain, value }) if domain == "GimpHistogramChannel" => {
                if let Some(next) = patch.channel {
                    *value = next;
                }
            }
            _ => return Err(BackingError::Integrity),
        }
    }
    if let Some(enabled) = patch.enabled {
        node.enabled = enabled;
    }
    Ok(())
}
