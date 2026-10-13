//! Explicit current wide-sample loss at foreign export boundaries.
//! This is scalar/format preflight, not pixel conversion or an exactness test.
use crate::IoError;
use photocraft_doc::{Document, LayerContent, SampleType};
use photocraft_codecs::{CodecError, Format};

#[derive(Clone, Copy)]
pub(crate) enum Destination { Psd, Psb, Flat(Format), LayeredTiff }
fn depth(sample: SampleType) -> &'static str {
    match sample { SampleType::U8 => "U8", SampleType::U16 => "U16", SampleType::U32 => "U32", SampleType::F32 => "F32", SampleType::F64 => "F64" }
}
#[derive(Default)]
struct Wide { u32: bool, f64: bool, visits: usize }
impl Wide {
    fn sample(&mut self, sample: SampleType) { self.u32 |= sample == SampleType::U32; self.f64 |= sample == SampleType::F64; }
    fn visit(&mut self) -> Result<(), IoError> {
        self.visits = self.visits.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| IoError::Unsupported("foreign export precision census exceeds bounded item limit".into()))?;
        Ok(())
    }
}
fn current_wide(document: &Document) -> Result<Wide, IoError> {
    let mut wide = Wide::default(); wide.sample(document.depth);
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or_else(|| IoError::Unsupported("foreign export layer depth".into()))? = Some(document.layers.iter());
    let mut level = 0usize;
    loop {
        let next = frames.get_mut(level).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else { if level == 0 { break; } level -= 1; continue; };
        wide.visit()?;
        match &layer.content {
            LayerContent::Raster(surface) => wide.sample(surface.format().sample),
            LayerContent::Smart(smart) => {
                if let Some(cache) = &smart.cache { wide.sample(cache.format().sample); }
                if let Some(mask) = &smart.filter_mask { wide.sample(mask.surface.format().sample); }
            }
            LayerContent::Group(group) => {
                level = level.checked_add(1).filter(|n| *n <= photocraft_doc::MAX_GROUP_DEPTH).ok_or_else(|| IoError::Unsupported("foreign export layer depth exceeds limits".into()))?;
                *frames.get_mut(level).ok_or_else(|| IoError::Unsupported("foreign export layer depth".into()))? = Some(group.children.iter());
            }
            _ => {}
        }
        if let Some(mask) = &layer.mask { wide.sample(mask.surface.format().sample); }
        if let Some(cache) = &layer.fill_cache { wide.sample(cache.surface.format().sample); }
    }
    if document.channels.len() > 100_000 { return Err(IoError::Unsupported("foreign export channel limit".into())); }
    for channel in &document.channels { wide.visit()?; wide.sample(channel.surface.format().sample); }
    if let Some(channel) = &document.quick_mask { wide.sample(channel.surface.format().sample); }
    if let Some(selection) = &document.selection { wide.sample(selection.format().sample); }
    if let Some(state) = &document.xcf_state {
        if state.filter_masks.len() > 100_000 { return Err(IoError::Unsupported("foreign export filter mask limit".into())); }
        for mask in &state.filter_masks { wide.visit()?; wide.sample(mask.surface.format().sample); }
    }
    Ok(wide)
}

/// Reject known unwritable codecs before flattening/conversion. Existing lossy
/// defaults stay unchanged; warnings identify actual current wide depth rather
/// than claiming every pixel differs. Native export never calls this helper.
pub(crate) fn preflight(document: &Document, destination: Destination) -> Result<Option<String>, IoError> {
    let name = match destination {
        Destination::Psd => "PSD", Destination::Psb => "PSB", Destination::LayeredTiff => "Layered TIFF",
        Destination::Flat(format) => {
            if !format.caps().write {
                return Err(CodecError::Unsupported { format, reason: "encoding is not available in this build".into() }.into());
            }
            if document.size.width == 0 || document.size.height == 0 { return Err(CodecError::InvalidImage("cannot encode an empty image".into()).into()); }
            if let Some((max_width, max_height)) = format.max_dimensions()
                && (document.size.width > max_width || document.size.height > max_height)
            { return Err(CodecError::Encode { format, message: format!("dimensions exceed {max_width}x{max_height}") }.into()); }
            format.name()
        }
    };
    if matches!(destination, Destination::LayeredTiff) && matches!(document.depth, SampleType::U32 | SampleType::F64) {
        return Err(IoError::Unsupported("Layered TIFF cannot retain U32/F64 layer samples; choose .pcraft or a flat TIFF export".into()));
    }
    let wide = current_wide(document)?;
    let samples = match (wide.u32, wide.f64) { (true, true) => "U32 and F64", (true, false) => "U32", (false, true) => "F64", (false, false) => return Ok(None) };
    let document_depth = depth(document.depth);
    let warning = if matches!(document.depth, SampleType::U32 | SampleType::F64) {
        format!("{name} export: current {document_depth} document with {samples} sample storage is narrowed to F32 at the foreign rendering/storage boundary; exact native sample precision and bit identity are not guaranteed. Format-specific encoding may quantize further; save as .pcraft to retain native U32/F64 samples")
    } else {
        format!("{name} export: current {document_depth} document contains {samples} raster/cache/mask/channel sample storage; foreign rendering/storage can narrow those owners to the document/encoder depth, so exact native sample precision and bit identity are not guaranteed. Save as .pcraft to retain native U32/F64 samples")
    };
    Ok(Some(warning))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, Size};
    #[test]
    fn both_wide_depths_name_loss_for_flat_and_layered_outputs() {
        for sample in [SampleType::U32, SampleType::F64] {
            let document = Document::new("wide", Size::new(1, 1), ColorMode::Rgb, sample);
            for target in [Destination::Flat(Format::Png), Destination::Flat(Format::Tiff), Destination::Psd, Destination::Psb] {
                let warning = preflight(&document, target).unwrap().unwrap();
                assert!(warning.contains(depth(sample))); assert!(warning.contains("F32")); assert!(warning.contains("bit identity"));
            }
            assert!(matches!(preflight(&document, Destination::LayeredTiff), Err(IoError::Unsupported(_))));
        }
    }
    #[test]
    fn mixed_current_owners_are_not_hidden_by_narrow_document_depth() {
        let mut document = Document::new("mixed", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        assert!(preflight(&document, Destination::Psd).unwrap().is_none());
        document.layers.push(Layer::raster("wide owner", document.pixel_format().with_sample(SampleType::F64)));
        let warning = preflight(&document, Destination::Flat(Format::Png)).unwrap().unwrap();
        assert!(warning.contains("current U8 document")); assert!(warning.contains("F64"));
    }
    #[test]
    fn unsupported_writer_and_dimensions_fail_before_rendering() {
        let document = Document::new("wide", Size::new(257, 1), ColorMode::Rgb, SampleType::F64);
        assert!(matches!(preflight(&document, Destination::Flat(Format::Heif)), Err(IoError::Codec(CodecError::Unsupported { .. }))));
        assert!(matches!(preflight(&document, Destination::Flat(Format::Ico)), Err(IoError::Codec(CodecError::Encode { .. }))));
    }
}
