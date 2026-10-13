//! Checked conversion at the explicitly lossy PSD boundary.
use crate::IoError;
use photocraft_doc::{Document, Layer, LayerContent, SampleType};
use photocraft_raster::Surface;

fn convert(surface: &mut Surface) -> Result<(), IoError> {
    if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
        *surface = surface
            .try_convert(surface.format().with_sample(SampleType::F32))
            .map_err(|e| IoError::Unsupported(format!("cannot convert wide PSD samples: {e}")))?;
    }
    Ok(())
}

fn layers(layers: &mut [Layer], depth: usize) -> Result<(), IoError> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(IoError::Unsupported("PSD layer depth exceeds limits".into()));
    }
    for layer in layers {
        match &mut layer.content {
            LayerContent::Raster(surface) => convert(surface)?,
            LayerContent::Group(group) => self::layers(&mut group.children, depth + 1)?,
            _ => return Err(IoError::Unsupported("wide PSD export requires a qualified raster or group representation".into())),
        }
        if let Some(mask) = &mut layer.mask {
            convert(std::sync::Arc::make_mut(&mut mask.surface))?;
        }
        if let Some(cache) = &mut layer.fill_cache {
            convert(&mut cache.surface)?;
        }
    }
    Ok(())
}

/// Stages conversion on a private COW document; failure leaves the source untouched.
pub(crate) fn psd_document(doc: &Document) -> Result<Option<Document>, IoError> {
    if !matches!(doc.depth, SampleType::U32 | SampleType::F64) {
        return Ok(None);
    }
    let mut staged = doc.clone();
    layers(&mut staged.layers, 0)?;
    for channel in &mut staged.channels {
        convert(&mut channel.surface)?;
    }
    if let Some(channel) = &mut staged.quick_mask {
        convert(&mut channel.surface)?;
    }
    if let Some(selection) = &mut staged.selection {
        convert(selection)?;
    }
    staged.depth = SampleType::F32;
    Ok(Some(staged))
}
