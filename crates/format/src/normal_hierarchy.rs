//! Closed native hierarchy metadata admission before any Surface/blob allocation.
use crate::{
    FormatError, Result,
    manifest::{ContentM, DocM, LayerM},
};
use photocraft_raster::backing::ReadControl;
pub(crate) fn preflight(m: &DocM, c: &ReadControl<'_>) -> Result<()> {
    crate::masked_filtered_flat::preflight(m, c)?;
    fn layers(v: &[LayerM], depth: usize, count: &mut usize, groups: &mut usize, leaves: &mut usize, c: &ReadControl<'_>) -> Result<()> {
        for l in v {
            c.check(1).map_err(crate::owned_document::error)?;
            *count = count.checked_add(1).ok_or_else(|| FormatError::LimitExceeded("hierarchy layers".into()))?;
            if *count > 4096 || depth > 32 {
                return Err(FormatError::LimitExceeded("hierarchy bounds".into()));
            }
            match &l.content {
                ContentM::Group { children, artboard: None, .. } if depth < 32 => {
                    *groups = groups.checked_add(1).ok_or_else(|| FormatError::LimitExceeded("groups".into()))?;
                    layers(children, depth + 1, count, groups, leaves, c)?;
                }
                ContentM::FiniteRaster { .. } => {
                    *leaves = leaves.checked_add(1).ok_or_else(|| FormatError::LimitExceeded("leaves".into()))?;
                }
                _ => return Err(FormatError::Unsupported("hierarchy content".into())),
            }
        }
        Ok(())
    }
    let mut count = 0;
    let mut groups = 0;
    let mut leaves = 0;
    layers(&m.layers, 0, &mut count, &mut groups, &mut leaves, c)?;
    if groups == 0 || leaves == 0 {
        return Err(FormatError::Unsupported("hierarchy needs group and raster".into()));
    }
    Ok(())
}
