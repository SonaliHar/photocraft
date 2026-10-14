//! Conservative cache comparison only. Unsupported scene kinds are cache misses;
//! the general compositor damage comparator retains its existing semantics.
use crate::{EngineError, Result};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent, LayerMask, WorkingProfileInterpretation};
use photocraft_raster::{Interrupt, Surface};
use std::sync::Arc;
struct Compare<'a, 'b> {
    control: &'a Interrupt<'a>,
    charge: &'b mut dyn FnMut(u64) -> Result<()>,
    visited: usize,
}
impl Compare<'_, '_> {
    fn tick(&mut self, units: u64) -> Result<()> {
        self.control.check().map_err(|_| EngineError::Cancelled)?;
        (self.charge)(units)?;
        self.control.check().map_err(|_| EngineError::Cancelled)
    }
    fn slice<T: PartialEq>(&mut self, a: &[T], b: &[T]) -> Result<bool> {
        self.tick(1)?;
        if a.len() != b.len() {
            return Ok(false);
        }
        for (a, b) in a.chunks(256).zip(b.chunks(256)) {
            let units = a
                .len()
                .checked_mul(std::mem::size_of::<T>().max(1))
                .and_then(|n| u64::try_from(n).ok())
                .ok_or_else(|| EngineError::Other("pixel cache work overflow".into()))?;
            self.tick(units)?;
            if a != b {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn blob(&mut self, a: &Option<Arc<Vec<u8>>>, b: &Option<Arc<Vec<u8>>>) -> Result<bool> {
        self.tick(1)?;
        match (a, b) {
            (None, None) => Ok(true),
            (Some(a), Some(b)) => {
                if Arc::ptr_eq(a, b) {
                    Ok(true)
                } else {
                    self.slice(a, b)
                }
            }
            _ => Ok(false),
        }
    }
    fn surface(&mut self, a: &Surface, b: &Surface) -> Result<bool> {
        self.tick(1)?;
        let f = a.format();
        if f != b.format()
            || a.tile_count() != b.tile_count()
            || f.channels() > 32
            || a.default_bytes().len() != f.bytes_per_pixel()
            || b.default_bytes().len() != f.bytes_per_pixel()
        {
            return Ok(false);
        }
        // Preserve the old default_pixel Vec<f32> equality, including NaN and
        // signed-zero semantics, without allocating that presentation vector.
        self.tick(f.channels() as u64)?;
        for i in 0..f.channels() {
            if photocraft_color::read_sample(a.default_bytes(), f.sample, i) != photocraft_color::read_sample(b.default_bytes(), f.sample, i) {
                return Ok(false);
            }
        }
        for ((ca, ta), (cb, tb)) in a.tiles().zip(b.tiles()) {
            self.tick(1)?;
            if ca != cb || !Arc::ptr_eq(ta, tb) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn mask(&mut self, a: &Option<LayerMask>, b: &Option<LayerMask>) -> Result<bool> {
        self.tick(1)?;
        match (a, b) {
            (None, None) => Ok(true),
            (Some(a), Some(b)) => Ok(a.enabled == b.enabled
                && a.linked == b.linked
                && a.density == b.density
                && a.feather == b.feather
                && self.surface(&a.surface, &b.surface)?),
            _ => Ok(false),
        }
    }
    fn channel(&mut self, a: &AlphaChannel, b: &AlphaChannel) -> Result<bool> {
        let AlphaChannel { id: _, name: _, surface: _, spot: _, color: _, opacity: _, indicates: _ } = a;
        self.tick(1)?;
        Ok(a.spot == b.spot
            && a.color == b.color
            && a.opacity == b.opacity
            && a.indicates == b.indicates
            && self.slice(a.name.as_bytes(), b.name.as_bytes())?
            && self.surface(&a.surface, &b.surface)?)
    }
    fn layers(&mut self, a: &[Layer], b: &[Layer], depth: usize) -> Result<bool> {
        self.tick(1)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH || a.len() != b.len() {
            return Ok(false);
        }
        for (a, b) in a.iter().zip(b) {
            // As in the upstream damage comparator, a new rendering field must
            // be deliberately classified rather than silently ignored.
            let Layer {
                id: _,
                name: _,
                visible: _,
                locks: _,
                blend: _,
                opacity: _,
                fill_opacity: _,
                clipped: _,
                mask: _,
                vector_mask: _,
                effects: _,
                label: _,
                content: _,
                psd_blocks: _,
                psd_id: _,
                fill_cache: _,
                link_group: _,
                indexed_source: _,
                import_compositing: _,
                excluded_channels: _,
                blend_if: _,
                advanced: _,
                video: _,
            } = a;
            let photocraft_doc::Effects { enabled: _, items: _, psd_raw: _, reference: _ } = &a.effects;
            self.visited = self.visited.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| EngineError::Other("pixel cache layer limit".into()))?;
            self.tick(1)?;
            if a.video.is_some()
                || b.video.is_some()
                || a.vector_mask.is_some()
                || b.vector_mask.is_some()
                || !a.effects.items.is_empty()
                || !b.effects.items.is_empty()
                || a.fill_cache.is_some()
                || b.fill_cache.is_some()
                || a.id != b.id
                || a.visible != b.visible
                || a.blend != b.blend
                || a.opacity != b.opacity
                || a.fill_opacity != b.fill_opacity
                || a.clipped != b.clipped
                || a.excluded_channels != b.excluded_channels
                || a.advanced != b.advanced
                || a.import_compositing != b.import_compositing
                || a.effects.enabled != b.effects.enabled
                || a.effects.reference != b.effects.reference
            {
                return Ok(false);
            }
            if !self.blob(&a.effects.psd_raw, &b.effects.psd_raw)? || !self.slice(&a.blend_if.ranges, &b.blend_if.ranges)? || !self.mask(&a.mask, &b.mask)? {
                return Ok(false);
            }
            match (&a.content, &b.content) {
                (LayerContent::Raster(a), LayerContent::Raster(b)) => {
                    if !self.surface(a, b)? {
                        return Ok(false);
                    }
                }
                (LayerContent::Group(a), LayerContent::Group(b)) => {
                    match (&a.artboard, &b.artboard) {
                        (None, None) => {}
                        (Some(a), Some(b)) if a.rect == b.rect && a.background == b.background => {
                            if !self.slice(a.preset.as_bytes(), b.preset.as_bytes())? {
                                return Ok(false);
                            }
                        }
                        _ => return Ok(false),
                    }
                    if !self.layers(&a.children, &b.children, depth + 1)? {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }
}
pub(crate) fn same_plain_pixels_with_interrupt(a: &Document, b: &Document, control: &Interrupt<'_>, charge: &mut dyn FnMut(u64) -> Result<()>) -> Result<bool> {
    let Document {
        id: _,
        name: _,
        size: _,
        resolution_dpi: _,
        physical_resolution: _,
        xcf_state: _,
        mode: _,
        depth: _,
        icc_profile: _,
        working_profile_interpretation: _,
        layers: _,
        channels: _,
        guides: _,
        ruler_origin: _,
        selection: _,
        selection_channel_id: _,
        metadata: _,
        global_light: _,
        paths: _,
        work_path: _,
        clipping_path: _,
        quick_mask: _,
        quick_mask_unselected: _,
        patterns: _,
        color_table: _,
        duotone: _,
        layer_comps: _,
        last_applied_comp: _,
        last_document_state: _,
        measurement: _,
        notes: _,
        text_styles: _,
        slices: _,
        variables: _,
        timeline: _,
        sample_encoding: _,
        source_color_context: _,
        file_encoding: _,
    } = a;
    let mut cx = Compare { control, charge, visited: 0 };
    cx.tick(1)?;
    if a.xcf_state.is_some()
        || b.xcf_state.is_some()
        || !a.patterns.is_empty()
        || !b.patterns.is_empty()
        || a.duotone.is_some()
        || b.duotone.is_some()
        || a.size != b.size
        || a.resolution_dpi.to_bits() != b.resolution_dpi.to_bits()
        || a.mode != b.mode
        || a.depth != b.depth
        || a.sample_encoding != b.sample_encoding
        || a.selection_channel_id != b.selection_channel_id
        || a.global_light != b.global_light
    {
        return Ok(false);
    }
    match (&a.physical_resolution, &b.physical_resolution) {
        (None, None) => {}
        (Some(ar), Some(br)) => {
            if ar.x_dpi != br.x_dpi || ar.y_dpi != br.y_dpi {
                return Ok(false);
            }
            match (&ar.unit, &br.unit) {
                (photocraft_doc::PhysicalUnit::Custom(a), photocraft_doc::PhysicalUnit::Custom(b)) => {
                    if a.factor != b.factor
                        || a.digits != b.digits
                        || !cx.slice(a.name.as_bytes(), b.name.as_bytes())?
                        || !cx.slice(a.symbol.as_bytes(), b.symbol.as_bytes())?
                        || !cx.slice(a.abbreviation.as_bytes(), b.abbreviation.as_bytes())?
                    {
                        return Ok(false);
                    }
                }
                (a, b) if std::mem::discriminant(a) == std::mem::discriminant(b) => {}
                _ => return Ok(false),
            }
        }
        _ => return Ok(false),
    }
    match (&a.working_profile_interpretation, &b.working_profile_interpretation) {
        (WorkingProfileInterpretation::Icc, WorkingProfileInterpretation::Icc) => {}
        (WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: a }, WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: b })
        | (WorkingProfileInterpretation::ImplicitSGray { icc_blake3: a }, WorkingProfileInterpretation::ImplicitSGray { icc_blake3: b }) => {
            if !cx.slice(a.as_bytes(), b.as_bytes())? {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    if !cx.blob(&a.icc_profile, &b.icc_profile)? {
        return Ok(false);
    }
    match (&a.source_color_context, &b.source_color_context) {
        (None, None) => {}
        (Some(a), Some(b)) if a.source_precision == b.source_precision => {
            if !cx.blob(&a.original_icc, &b.original_icc)? {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    match (&a.color_table, &b.color_table) {
        (None, None) => {}
        (Some(a), Some(b)) if a.transparent == b.transparent => {
            if !cx.slice(&a.colors, &b.colors)? {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    if a.channels.len() != b.channels.len() {
        return Ok(false);
    }
    for (a, b) in a.channels.iter().zip(&b.channels) {
        if !cx.channel(a, b)? {
            return Ok(false);
        }
    }
    match (&a.quick_mask, &b.quick_mask) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            if !cx.channel(a, b)? {
                return Ok(false);
            }
        }
        _ => return Ok(false),
    }
    let equal = cx.layers(&a.layers, &b.layers, 0)?;
    cx.tick(1)?;
    Ok(equal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, PixelFormat, SampleType, Size};
    fn fixture() -> Document {
        let mut doc = Document::new("Own cache comparison", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("Original name", PixelFormat::RGBA8);
        layer.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.25, 0.5, 0.75, 1.]);
        doc.layers.push(layer);
        doc.icc_profile = Some(Arc::new(vec![19; 4096]));
        doc
    }
    fn compare(a: &Document, b: &Document) -> bool {
        same_plain_pixels_with_interrupt(a, b, &Interrupt::NONE, &mut |_| Ok(())).unwrap()
    }
    #[test]
    fn selection_and_nonrendering_layer_metadata_retain_old_cache_semantics() {
        let a = fixture();
        let mut b = a.clone();
        b.name = "Renamed document".into();
        b.layers[0].name = "Renamed layer".into();
        b.selection = Some(Surface::with_default(PixelFormat::GRAY8, &[1.]));
        assert!(super::super::same_pixels(&a, &b));
        assert!(compare(&a, &b));
        b.layers[0].opacity = 0.5;
        assert!(!super::super::same_pixels(&a, &b));
        assert!(!compare(&a, &b));
    }
    #[test]
    fn byte_equal_independent_icc_and_default_nan_match_old_comparison() {
        let a = fixture();
        let mut b = a.clone();
        b.icc_profile = Some(Arc::new(vec![19; 4096]));
        assert!(compare(&a, &b));
        b.icc_profile.as_mut().map(|icc| Arc::make_mut(icc).push(1));
        assert!(!compare(&a, &b));
        let mut a = fixture();
        a.layers[0].content = LayerContent::Raster(Surface::with_default(PixelFormat::RGBA8.with_sample(SampleType::F32), &[f32::NAN, 0., 0., 0.]));
        let b = a.clone();
        assert!(!super::super::same_pixels(&a, &b));
        assert!(!compare(&a, &b));
    }
    #[test]
    fn optional_two_axis_resolution_and_custom_units_keep_old_cache_key() {
        let mut a = fixture();
        let mut b = a.clone();
        a.physical_resolution = None;
        b.physical_resolution = Some(photocraft_doc::PhysicalResolution::square(72.));
        assert!(!super::super::same_pixels(&a, &b));
        assert!(!compare(&a, &b));
        a.physical_resolution = b.physical_resolution.clone();
        assert!(compare(&a, &b));
        b.physical_resolution.as_mut().unwrap().y_dpi = 144.;
        assert!(!compare(&a, &b));
        a.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 72., y_dpi: 144.,
            unit: photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
                name: "Own units".into(), factor: 2., digits: 2, symbol: "u".into(), abbreviation: "unit".into(),
            }) });
        b.physical_resolution = a.physical_resolution.clone();
        assert!(compare(&a, &b));
        if let photocraft_doc::PhysicalUnit::Custom(unit) = &mut b.physical_resolution.as_mut().unwrap().unit {
            unit.abbreviation = "changed".into();
        }
        assert!(!compare(&a, &b));
    }
    #[test]
    fn private_comparison_cancels_inside_blob_and_preserves_both_documents() {
        let a = fixture();
        let mut b = a.clone();
        b.icc_profile = Some(Arc::new(vec![19; 4096]));
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() >= 8
        };
        assert!(matches!(same_plain_pixels_with_interrupt(&a, &b, &Interrupt::cancel_only(&cancel), &mut |_| Ok(())), Err(EngineError::Cancelled)));
        let mut work = 0;
        assert!(
            same_plain_pixels_with_interrupt(&a, &b, &Interrupt::NONE, &mut |n| {
                work += n;
                if work > 100 { Err(EngineError::Other("test work limit".into())) } else { Ok(()) }
            })
            .is_err()
        );
        assert_eq!(a.icc_profile.as_ref().unwrap().len(), 4096);
        assert_eq!(b.icc_profile.as_ref().unwrap().len(), 4096);
    }
}
