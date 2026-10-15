//! Explicit CURRENT composition over actual readonly flat native custody.
//! This is no source, profile, operation-evaluation or generic edit grant.
use super::*;
use crate::{
    BlendMode, ImportCompositing, SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace, ResolvedGimpComposition},
};

/// The existing `xcf.composition.set` subset: modern exact ordinary equivalents,
/// explicit RGB transfer spaces and operators. Legacy/pass-through are false;
/// the existing canonical compatibility epoch is retained verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatCompositeCompositionPatch {
    pub layer: LayerId,
    pub source_mode: i32,
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
}
#[derive(Clone, Copy)]
pub(super) struct Plan {
    patch: FlatCompositeCompositionPatch,
    layer_index: usize,
    item_index: usize,
    ordinary: BlendMode,
    blend_word: i32,
    composite_word: i32,
    operator_word: i32,
    physical: ImportCompositing,
}
fn ordinary(mode: i32) -> Result<BlendMode, BackingError> {
    Ok(match mode {
        28 => BlendMode::Normal,
        23 => BlendMode::Overlay,
        30 => BlendMode::Multiply,
        31 => BlendMode::Screen,
        32 => BlendMode::Difference,
        33 => BlendMode::LinearDodge,
        34 => BlendMode::Subtract,
        35 => BlendMode::Darken,
        36 => BlendMode::Lighten,
        41 => BlendMode::Divide,
        42 => BlendMode::ColorDodge,
        43 => BlendMode::ColorBurn,
        44 => BlendMode::HardLight,
        45 => BlendMode::SoftLight,
        48 => BlendMode::VividLight,
        49 => BlendMode::PinLight,
        50 => BlendMode::LinearLight,
        51 => BlendMode::HardMix,
        52 => BlendMode::Exclusion,
        53 => BlendMode::LinearBurn,
        // In particular, do not invent an ordinary alias for Grain46/47.
        _ => return Err(BackingError::Unsupported),
    })
}
fn transfer(space: GimpSpace) -> Result<(i32, SampleEncoding), BackingError> {
    match space {
        GimpSpace::LinearRgb => Ok((1, SampleEncoding::Linear)),
        GimpSpace::ProfileRgb => Ok((2, SampleEncoding::NonlinearProfile)),
        GimpSpace::PerceptualRgb => Ok((4, SampleEncoding::Perceptual)),
        GimpSpace::Lab => Err(BackingError::Unsupported),
    }
}
fn prepare(owner: &AccountedDocument, patch: FlatCompositeCompositionPatch, c: &ReadControl<'_>) -> Result<(ModelCensus, Option<Plan>), BackingError> {
    c.check(32)?;
    if !owner.is_readonly_flat_composite_storage() || owner.document().mode != crate::ColorMode::Rgb {
        return Err(BackingError::Unsupported);
    }
    let ordinary = ordinary(patch.source_mode)?;
    let (blend_word, blend_encoding) = transfer(patch.blend_space)?;
    let (composite_word, composite_encoding) = transfer(patch.composite_space)?;
    let operator_word = match patch.operator {
        GimpOperator::Union => 1,
        GimpOperator::ClipToBackdrop => 2,
        GimpOperator::ClipToLayer => 3,
        GimpOperator::Intersection => 4,
    };
    let logical = measured(owner.document(), owner.quota(), c)?;
    let mut layer_index = None;
    for (index, layer) in owner.document().layers.iter().enumerate() {
        c.check(1)?;
        if layer.id == patch.layer && layer_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let layer_index = layer_index.ok_or(BackingError::Bounds)?;
    let layer = owner.document().layers.get(layer_index).ok_or(BackingError::Integrity)?;
    let state = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut item_index = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == ItemRef::Layer(patch.layer) && item_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let item_index = item_index.ok_or(BackingError::Integrity)?;
    let policy = state.items.get(item_index).and_then(|i| i.composition.as_ref()).ok_or(BackingError::Unsupported)?;
    let physical = ImportCompositing { blend_encoding, composite_encoding, pass_through: false };
    let resolved = ResolvedGimpComposition { blend_space: patch.blend_space, composite_space: patch.composite_space, operator: patch.operator };
    c.check(32)?;
    let unchanged = policy.source_mode == patch.source_mode
        && !policy.legacy
        && !policy.pass_through
        && policy.stored_blend_space == blend_word
        && policy.stored_composite_space == composite_word
        && policy.stored_operator == operator_word
        && policy.resolved.as_ref() == Some(&resolved)
        && layer.blend == ordinary
        && layer.import_compositing == Some(physical);
    Ok((logical, (!unchanged).then_some(Plan { patch, layer_index, item_index, ordinary, blend_word, composite_word, operator_word, physical })))
}
impl AccountedDocument {
    /// Structural/copy preflight only. Engine locks, captured document revision,
    /// actual current and staged profile/native evaluation are still mandatory.
    pub fn validate_flat_composite_composition_patch(&self, patch: FlatCompositeCompositionPatch, c: &ReadControl<'_>) -> Result<(), BackingError> {
        prepare(self, patch, c)?;
        Ok(())
    }
    pub fn try_with_flat_composite_composition_patch(&self, patch: FlatCompositeCompositionPatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let (logical, plan) = prepare(self, patch, c)?;
        let Some(plan) = plan else {
            return Ok(self.clone());
        };
        current_copy(self, logical, CopyKind::Composition(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(32)?;
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let item = state.items.get_mut(plan.item_index).ok_or(BackingError::Integrity)?;
    let layer = doc.layers.get_mut(plan.layer_index).ok_or(BackingError::Integrity)?;
    if item.item != ItemRef::Layer(plan.patch.layer) || layer.id != plan.patch.layer {
        return Err(BackingError::Integrity);
    }
    let policy = item.composition.as_mut().ok_or(BackingError::Integrity)?;
    // Preserve compatibility_epoch and every source/raw/inert field not listed.
    policy.source_mode = plan.patch.source_mode;
    policy.legacy = false;
    policy.stored_blend_space = plan.blend_word;
    policy.stored_composite_space = plan.composite_word;
    policy.stored_operator = plan.operator_word;
    policy.resolved =
        Some(ResolvedGimpComposition { blend_space: plan.patch.blend_space, composite_space: plan.patch.composite_space, operator: plan.patch.operator });
    policy.pass_through = false;
    layer.blend = plan.ordinary;
    layer.import_compositing = Some(plan.physical);
    c.check(1)
}

#[cfg(test)]
#[path = "accounted_flat_composite_readonly_composition_tests.rs"]
mod tests;
