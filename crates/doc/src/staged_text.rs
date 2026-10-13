//! Bounded structural admission for cloning editable text models, not a global OOM guarantee.
use crate::{Layer, LayerContent, LayerId, TextLayer};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, backing::BackingError};
use std::collections::HashSet;

struct Census<'a, 'i> {
    bytes: u64,
    limit: u64,
    count: usize,
    work: usize,
    texts: usize,
    ids: HashSet<LayerId>,
    control: &'a Interrupt<'i>,
}
impl Census<'_, '_> {
    fn add(&mut self, bytes: u64) -> Result<(), BackingError> {
        self.bytes = self.bytes.checked_add(bytes).filter(|n| *n <= self.limit).ok_or(BackingError::Quota)?;
        Ok(())
    }
    fn work(&mut self, units: usize) -> Result<(), BackingError> {
        self.control.check().map_err(|_| BackingError::Cancelled)?;
        self.work = self.work.checked_add(units).filter(|n| *n <= 1_000_000).ok_or(BackingError::Length)?;
        Ok(())
    }
    fn owned(&mut self, bytes: usize) -> Result<(), BackingError> {
        self.add((bytes as u64).checked_mul(2).ok_or(BackingError::Length)?)
    }
    fn string(&mut self, value: &String) -> Result<(), BackingError> {
        self.work(1)?;
        if value.len() > 1 << 20 {
            return Err(BackingError::Length);
        }
        self.owned(value.capacity())
    }
    fn optional(&mut self, value: &Option<String>) -> Result<(), BackingError> {
        if let Some(value) = value {
            self.string(value)?;
        }
        Ok(())
    }
    fn vector<T>(&mut self, value: &Vec<T>) -> Result<(), BackingError> {
        if value.len() > 100_000 {
            return Err(BackingError::Length);
        }
        self.work(value.len())?;
        self.owned(value.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(BackingError::Length)?)
    }
    fn text(&mut self, text: &TextLayer) -> Result<(), BackingError> {
        let TextLayer {
            text: value,
            font_family,
            size_pt: _,
            color: _,
            transform: _,
            cache,
            psd_raw,
            runs,
            paragraphs,
            shape,
            orientation: _,
            antialias: _,
            warp,
        } = text;
        if let crate::text::TextShape::Path { path, .. } = shape {
            self.vector(&path.knots)?;
        }
        self.texts = self.texts.checked_add(1).ok_or(BackingError::Length)?;
        self.owned(std::mem::size_of::<(LayerId, TextLayer)>())?;
        self.string(value)?;
        self.string(font_family)?;
        self.vector(runs)?;
        self.vector(paragraphs)?;
        for run in runs {
            let crate::text::CharStyle {
                font_family,
                font_style,
                postscript_name,
                weight: _,
                italic: _,
                size_pt: _,
                color: _,
                tracking: _,
                leading_pt: _,
                baseline_shift_pt: _,
                horizontal_scale: _,
                vertical_scale: _,
                underline: _,
                strikethrough: _,
                faux_bold: _,
                faux_italic: _,
                kerning: _,
                kern: _,
                caps: _,
                ligatures: _,
                discretionary_ligatures: _,
                features,
                variations,
                language,
                style_sheet: _,
            } = &run.style;
            self.string(font_family)?;
            self.string(font_style)?;
            self.optional(postscript_name)?;
            self.optional(language)?;
            self.vector(features)?;
            self.vector(variations)?;
            for feature in features {
                self.string(&feature.tag)?;
            }
            for variation in variations {
                self.string(&variation.axis)?;
            }
        }
        if let Some(warp) = warp {
            self.string(&warp.style)?;
        }
        // Immutable raw TySh and native tile payloads are shared, not copied.
        if let Some(raw) = psd_raw {
            self.add(raw.capacity() as u64)?;
        }
        if let Some(surface) = cache {
            if surface.tile_count() > 100_000 {
                return Err(BackingError::Length);
            }
            self.add(surface.default_bytes().len() as u64)?;
            for (_, tile) in surface.tiles() {
                self.work(1)?;
                self.add((tile.bytes().len() as u64).checked_add(256).ok_or(BackingError::Length)?)?;
            }
            let cloned =
                (surface.tile_count() as u64).checked_mul(256).and_then(|n| n.checked_add(surface.default_bytes().len() as u64)).ok_or(BackingError::Length)?;
            self.add(cloned)?;
            surface.try_validate_tile_origins_with(self.control).map_err(|error| match error {
                photocraft_raster::RasterInputError::Cancelled => BackingError::Cancelled,
                _ => BackingError::Bounds,
            })?;
        }
        Ok(())
    }
    fn layers(&mut self, layers: &[Layer], depth: usize) -> Result<(), BackingError> {
        if depth > crate::MAX_GROUP_DEPTH {
            return Err(BackingError::Bounds);
        }
        for layer in layers {
            self.work(1)?;
            self.count = self.count.checked_add(1).filter(|n| *n <= 100_000).ok_or(BackingError::Length)?;
            self.add(64)?;
            self.ids.try_reserve(1).map_err(|_| BackingError::Allocation)?;
            if layer.id.0 == 0 || !self.ids.insert(layer.id) {
                return Err(BackingError::Integrity);
            }
            match &layer.content {
                LayerContent::Text(text) => self.text(text)?,
                LayerContent::Group(group) => self.layers(&group.children, depth + 1)?,
                _ => {}
            }
        }
        Ok(())
    }
}

/// Admit source text retention plus its staged structural COW clone, then clone only text.
/// Other layer fields are borrowed and never cloned. The cap may only lower the raster cap.
/// Standard String/Arc/map clones remain standard allocations after this size admission.
pub fn try_clone_text_layers(layers: &[Layer], limit: u64, control: &Interrupt<'_>) -> Result<Vec<(LayerId, TextLayer)>, BackingError> {
    if limit == 0 || limit > MAX_CHECKED_RASTER_BYTES {
        return Err(BackingError::Quota);
    }
    let mut census = Census { bytes: 0, limit, count: 0, work: 0, texts: 0, ids: HashSet::new(), control };
    census.layers(layers, 0)?;
    let mut staged = Vec::new();
    staged.try_reserve_exact(census.texts).map_err(|_| BackingError::Allocation)?;
    fn clone(layers: &[Layer], staged: &mut Vec<(LayerId, TextLayer)>, control: &Interrupt<'_>) -> Result<(), BackingError> {
        for layer in layers {
            control.check().map_err(|_| BackingError::Cancelled)?;
            match &layer.content {
                LayerContent::Text(text) => staged.push((layer.id, text.clone())),
                LayerContent::Group(group) => clone(&group.children, staged, control)?,
                _ => {}
            }
        }
        Ok(())
    }
    clone(layers, &mut staged, control)?;
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_identity_refuses_before_clone() {
        let mut layer = Layer::new("Text", LayerContent::Text(TextLayer::default()));
        layer.id = LayerId(1);
        let layers = vec![layer.clone(), layer];
        assert!(matches!(try_clone_text_layers(&layers, MAX_CHECKED_RASTER_BYTES, &Interrupt::NONE), Err(BackingError::Integrity)));
    }
    #[test]
    fn lower_cap_refuses_nested_run_storage() {
        let mut text = TextLayer::default();
        text.runs.push(crate::text::TextRun { len: 0, style: crate::text::CharStyle { font_family: "A".repeat(4096), ..Default::default() } });
        let layers = vec![Layer::new("Text", LayerContent::Text(text))];
        assert!(matches!(try_clone_text_layers(&layers, 1024, &Interrupt::NONE), Err(BackingError::Quota)));
    }
    #[test]
    fn stage_copies_text_only_with_exact_identity() {
        let text = TextLayer { text: "Independent staged model".into(), ..Default::default() };
        let layer = Layer::new("Text", LayerContent::Text(text.clone()));
        let staged = try_clone_text_layers(std::slice::from_ref(&layer), MAX_CHECKED_RASTER_BYTES, &Interrupt::NONE).unwrap();
        assert_eq!(staged, vec![(layer.id, text)]);
    }
}
