//! Typed current-node metadata changes for genuine immutable-pixel storage.
//! No raw Document callback, source-proof field, profile or pixel edit is exposed.
use super::*;
use crate::{LayerId, item_identity::FilterId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThresholdAlphaPatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub value_f32_bits: Option<u32>,
    pub enabled: Option<bool>,
}
impl AccountedDocument {
    /// Copy only this closed current filter's Float32 threshold and activation.
    /// Original source bytes/provenance, native storage and all other metadata
    /// remain retained. Effective command locks and render recipes are checked
    /// by the engine; this typed storage API grants neither source nor CMS policy.
    pub fn try_with_threshold_alpha(&self, patch: ThresholdAlphaPatch, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if self.owner._storage_class != StorageClass::ReadonlyThresholdAlpha {
            return unsupported();
        }
        if let Some(bits) = patch.value_f32_bits {
            let value = f32::from_bits(bits);
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(BackingError::Unsupported);
            }
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
        let argument = node.arguments.first().ok_or(BackingError::Integrity)?;
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else { return unsupported() };
        let changed = patch.value_f32_bits.is_some_and(|next| next != bits) || patch.enabled.is_some_and(|next| next != node.enabled);
        if !changed {
            control.check(1)?;
            return Ok(self.clone());
        }
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        current_copy(self, logical, CopyKind::ThresholdAlpha { value_f32_bits: patch.value_f32_bits, enabled: patch.enabled, revision }, control)
    }
}
