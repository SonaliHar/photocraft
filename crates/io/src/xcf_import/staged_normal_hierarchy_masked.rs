//! Separate raw-coverage Source purpose on the actual immutable hierarchy.
use super::staged_masked_filtered_flat::{MaskClass, mask_class};
use super::*;
use crate::xcf_import::staged_filtered_flat::work;
pub(super) fn select<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<SelectedStagedFamily, IoError> {
    if super::staged_normal_hierarchy_quantized::exact_depth(f){return Ok(SelectedStagedFamily::PlainQuantizedNormalHierarchy);}
    if f.version != 22 {
        return Ok(SelectedStagedFamily::NormalHierarchy);
    }
    // This distinct permission lookup is a real repeated metadata walk.
    work(i, b, f.layers.len() as u64)?;
    if !f.layers.iter().filter_map(|d| d.plane.as_ref()).any(ExactParsedPlane::allows_normal_hierarchy_coverage) {
        return Ok(SelectedStagedFamily::NormalHierarchy);
    }
    let mut nonempty = false;
    for d in &f.layers {
        for e in &d.effects {
            work(i, b, 32)?;
            let m = e.mask.as_ref().ok_or_else(|| IoError::Unsupported("hierarchy coverage mask missing".into()))?;
            nonempty |= mask_class(m, f.sample, f.width, f.height, i, b)? == MaskClass::Nonempty;
        }
    }
    Ok(if nonempty { SelectedStagedFamily::MaskedNormalHierarchy } else { SelectedStagedFamily::NormalHierarchy })
}
pub(super) fn validate_leaf<P: ExactParsedPlane>(d: &Drawable<P>, f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
    if !d.plane.as_ref().is_some_and(ExactParsedPlane::allows_normal_hierarchy_coverage) {
        return Err(IoError::Unsupported("hierarchy coverage descriptor permission".into()));
    }
    for e in &d.effects {
        work(i, b, 512 + crate::xcf_filters::mixed_stock_source::parsing_work(&e.operation))?;
        if !crate::xcf_filters::filtered_flat_source_envelope(e) {
            return Err(IoError::Unsupported("hierarchy coverage unit Source envelope".into()));
        }
        match mask_class(e.mask.as_ref().ok_or_else(|| IoError::Unsupported("hierarchy coverage mask".into()))?, f.sample, f.width, f.height, i, b)? {
            MaskClass::ByteZero => {
                crate::xcf_filters::filtered_flat_source_family(e).ok_or_else(|| IoError::Unsupported("hierarchy zero sibling family".into()))?;
            }
            MaskClass::Nonempty => {
                crate::xcf_filters::masked_mixed_source::parameters(e).ok_or_else(|| IoError::Unsupported("hierarchy nonempty family".into()))?;
            }
        }
    }
    Ok(())
}
pub(super) fn qualify<P: ExactParsedPlane>(f: &XcfFile<P>, i: &Interrupt<'_>, b: &photocraft_xcf::Budget<'_>) -> Result<(), IoError> {
    if select(f, i, b)? != SelectedStagedFamily::MaskedNormalHierarchy {
        return Err(IoError::Unsupported("hierarchy coverage Source requires nonempty".into()));
    }
    super::staged_normal_hierarchy::validate_source_body(f, i, b, true,false)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "staged_normal_hierarchy_masked_tests.rs"]
mod tests;
