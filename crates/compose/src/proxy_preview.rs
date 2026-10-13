//! Charged render-only ordinary proxy snapshots. Persistence metadata stays in
//! the borrowed original; this view is never used for editing or native saving.
use super::*;
use photocraft_geom::Size;
use photocraft_raster::{Interrupt, RasterInputError};
use preview_budget::{ChargedThumbnail, Lease, PreviewPreparationBudget};

/// Qualified flat raster/member domain. Other documents retain their legacy
/// preview route rather than silently dropping operative rendering state.
pub fn proxy_preview_supported(doc: &Document) -> bool {
    doc.size.width > 0
        && doc.size.height > 0
        && doc.size.width <= 262144
        && doc.size.height <= 262144
        && doc.xcf_state.is_none()
        && doc.sample_encoding.is_none()
        && doc.source_color_context.is_none()
        && doc.icc_profile.is_none()
        && doc.working_profile_interpretation == photocraft_doc::WorkingProfileInterpretation::Icc
        && matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        && matches!(doc.depth, photocraft_color::SampleType::U8 | photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32)
        && doc.layers.len() <= 128
        && doc.patterns.is_empty()
        && doc.name.len() <= 1 << 20
        && doc.layers.iter().all(|l| {
            matches!(&l.content,LayerContent::Raster(s) if s.format().mode==doc.mode && s.format().sample==doc.depth)
                && l.name.len() <= 1 << 20
                && l.vector_mask.is_none()
                && l.effects.items.is_empty()
                && l.effects.psd_raw.is_none()
                && l.advanced.is_default()
                && l.import_compositing.is_none()
                && l.indexed_source.is_none()
                && l.video.is_none()
                && l.fill_cache.is_none()
                && l.blend == BlendMode::Normal
                && !l.clipped
                && l.excluded_channels == 0
                && l.blend_if.ranges.is_empty()
        })
}
struct Snapshot<'a, 'b, 's> {
    doc: Document,
    _source: &'s Document,
    leases: Vec<Lease<'a, 'b>>,
    _metadata: Lease<'a, 'b>,
}
fn copy_string(source: &str, control: &Interrupt<'_>) -> Result<String, RasterInputError> {
    use RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    let mut out = String::new();
    out.try_reserve_exact(source.len()).map_err(|_| E::AllocationFailed)?;
    if out.capacity() != source.len() {
        return Err(E::LimitExceeded);
    }
    let mut offset = 0;
    while offset < source.len() {
        control.check().map_err(|_| E::Cancelled)?;
        let mut end = offset.saturating_add(4096).min(source.len());
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(source.get(offset..end).ok_or(E::LengthMismatch)?);
        offset = end;
    }
    Ok(out)
}
fn surface_ceiling(surface: &Surface, k: u32, extent: Option<Rect>, control: &Interrupt<'_>) -> Result<(usize, usize), RasterInputError> {
    use RasterInputError as E;
    surface.try_validate_tile_origins_with(control)?;
    let b = surface.content_bounds();
    let divisor = i64::from(k);
    let tile = 256usize.checked_mul(256).and_then(|v| v.checked_mul(surface.format().bytes_per_pixel())).ok_or(E::LimitExceeded)?;
    let mut retained = surface.default_bytes().len();
    if !b.is_empty() {
        let edge = |v: i32, ceil: bool| i64::from(v).div_euclid(divisor) + i64::from(ceil && i64::from(v).rem_euclid(divisor) != 0);
        let (x0, y0, x1, y1) = (edge(b.x0, false), edge(b.y0, false), edge(b.x1, true), edge(b.y1, true));
        let b = Rect::new(
            i32::try_from(x0).map_err(|_| E::InvalidBounds)?,
            i32::try_from(y0).map_err(|_| E::InvalidBounds)?,
            i32::try_from(x1).map_err(|_| E::InvalidBounds)?,
            i32::try_from(y1).map_err(|_| E::InvalidBounds)?,
        );
        let b = extent.map_or(b, |extent| b.intersect(&extent));
        if b.is_empty() {
            return Ok((retained, tile.checked_mul(2).ok_or(E::LimitExceeded)?));
        }
        let (x0, y0, x1, y1) = (i64::from(b.x0), i64::from(b.y0), i64::from(b.x1), i64::from(b.y1));
        let columns = (x1 - 1).div_euclid(256) - x0.div_euclid(256) + 1;
        let rows = (y1 - 1).div_euclid(256) - y0.div_euclid(256) + 1;
        let count = usize::try_from(columns.checked_mul(rows).ok_or(E::LimitExceeded)?).map_err(|_| E::LimitExceeded)?;
        retained = retained.checked_add(count.checked_mul(tile.checked_add(128).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    }
    Ok((retained, tile.checked_mul(2).ok_or(E::LimitExceeded)?))
}
impl<'a, 'b, 's> Snapshot<'a, 'b, 's> {
    fn new(source: &'s Document, k: u32, control: &Interrupt<'_>, budget: &'a PreviewPreparationBudget<'b>) -> Result<Self, RasterInputError> {
        use RasterInputError as E;
        control.check().map_err(|_| E::Cancelled)?;
        if !proxy_preview_supported(source) {
            return Err(E::UnsupportedConversion);
        }
        if !(2..=1 << 20).contains(&k) {
            return Err(E::InvalidBounds);
        }
        proxy::validate_surfaces_with_interrupt(source, control)?;
        let slots = source.layers.len().checked_mul(2).ok_or(E::LimitExceeded)?;
        let metadata = source
            .layers
            .iter()
            .try_fold(source.name.len(), |n, l| n.checked_add(l.name.len()))
            .and_then(|n| n.checked_add(source.layers.len().checked_mul(std::mem::size_of::<Layer>())?))
            .and_then(|n| n.checked_add(slots.checked_mul(std::mem::size_of::<Lease<'a, 'b>>())?))
            .and_then(|n| n.checked_add(source.layers.iter().filter(|layer| layer.mask.is_some()).count().checked_mul(std::mem::size_of::<Surface>() + 128)?))
            .and_then(|n| n.checked_add(4096))
            .ok_or(E::LimitExceeded)?;
        let metadata_lease = budget.reserve(metadata, control)?;
        let mut leases = Vec::new();
        leases.try_reserve_exact(slots).map_err(|_| E::AllocationFailed)?;
        if leases.capacity() != slots {
            return Err(E::LimitExceeded);
        }
        let name = copy_string(&source.name, control)?;
        // Exhaustive render header: adding an operative document field requires
        // an explicit review here. Persistence-only state remains in `_source`.
        let mut doc = Document {
            physical_resolution: None,
            xcf_state: None,
            sample_encoding: None,
            source_color_context: None,
            working_profile_interpretation: photocraft_doc::WorkingProfileInterpretation::Icc,
            id: source.id,
            name,
            size: Size::new(source.size.width.div_ceil(k), source.size.height.div_ceil(k)),
            resolution_dpi: source.resolution_dpi,
            mode: source.mode,
            depth: source.depth,
            icc_profile: None,
            layers: Vec::new(),
            channels: Vec::new(),
            guides: Default::default(),
            selection: None,
            selection_channel_id: None,
            metadata: Default::default(),
            global_light: source.global_light,
            paths: Vec::new(),
            work_path: None,
            clipping_path: None,
            quick_mask: None,
            quick_mask_unselected: source.quick_mask_unselected,
            patterns: Vec::new(),
            color_table: None,
            duotone: None,
            layer_comps: Vec::new(),
            last_applied_comp: None,
            last_document_state: None,
            measurement: Default::default(),
            notes: Vec::new(),
            text_styles: Default::default(),
            slices: Default::default(),
            variables: Default::default(),
            timeline: None,
            file_encoding: source.file_encoding,
        };
        doc.layers.try_reserve_exact(source.layers.len()).map_err(|_| E::AllocationFailed)?;
        if doc.layers.capacity() != source.layers.len() {
            return Err(E::LimitExceeded);
        }
        let mut snapshot = Self { doc, _source: source, leases, _metadata: metadata_lease };
        for original in &source.layers {
            control.check().map_err(|_| E::Cancelled)?;
            let LayerContent::Raster(surface) = &original.content else {
                return Err(E::UnsupportedConversion);
            };
            let pixels = snapshot.surface(surface, k, Some(snapshot.doc.bounds()), control, budget)?;
            let name = copy_string(&original.name, control)?;
            let mut layer = Layer {
                indexed_source: None,
                import_compositing: None,
                id: original.id,
                name,
                visible: original.visible,
                locks: original.locks,
                blend: original.blend,
                opacity: original.opacity,
                fill_opacity: original.fill_opacity,
                clipped: original.clipped,
                mask: None,
                vector_mask: None,
                effects: photocraft_doc::Effects { enabled: original.effects.enabled, ..Default::default() },
                label: original.label,
                content: LayerContent::Raster(pixels),
                psd_blocks: Vec::new(),
                psd_id: original.psd_id,
                fill_cache: None,
                link_group: original.link_group,
                excluded_channels: original.excluded_channels,
                blend_if: Default::default(),
                advanced: original.advanced,
                video: None,
            };
            if let Some(mask) = &original.mask {
                layer.mask = Some(photocraft_doc::LayerMask {
                    surface: std::sync::Arc::new(snapshot.surface(&mask.surface, k, None, control, budget)?),
                    enabled: mask.enabled,
                    linked: mask.linked,
                    density: mask.density,
                    feather: mask.feather,
                });
            }
            snapshot.doc.layers.push(layer);
        }
        control.check().map_err(|_| E::Cancelled)?;
        Ok(snapshot)
    }
    fn surface(
        &mut self,
        source: &Surface,
        k: u32,
        extent: Option<Rect>,
        control: &Interrupt<'_>,
        budget: &'a PreviewPreparationBudget<'b>,
    ) -> Result<Surface, RasterInputError> {
        use RasterInputError as E;
        let (retained, scratch) = surface_ceiling(source, k, extent, control)?;
        let mut lease = budget.reserve(retained.checked_add(scratch).ok_or(E::LimitExceeded)?, control)?;
        let surface = match extent {
            Some(extent) => proxy::try_downsample_render_extent(source, k, extent, control)?,
            None => proxy::try_downsample_with_interrupt(source, k, control)?,
        };
        let actual = usize::try_from(surface.resident_payload_bytes()?)
            .map_err(|_| E::LimitExceeded)?
            .checked_add(surface.tile_count().checked_mul(128).ok_or(E::LimitExceeded)?)
            .ok_or(E::LimitExceeded)?;
        lease.shrink(actual)?;
        self.leases.push(lease);
        Ok(surface)
    }
}
/// Byte-identical legacy proxy preview in the admitted domain, with snapshot,
/// preparation, and returned image leases all using the supplied live ledger.
pub fn thumbnail_with_proxy_preparation_budget<'a, 'b>(
    doc: &Document,
    side: u32,
    control: &Interrupt<'_>,
    budget: &'a PreviewPreparationBudget<'b>,
) -> Result<ChargedThumbnail<'a, 'b>, RasterInputError> {
    use RasterInputError as E;
    control.check().map_err(|_| E::Cancelled)?;
    if !proxy_preview_supported(doc) || side == 0 || side > 4096 {
        return Err(E::UnsupportedConversion);
    }
    let longest = doc.size.width.max(doc.size.height).max(1);
    let k = longest / side.saturating_mul(2);
    if k < 2 || doc.size.area() <= PROXY_THUMBNAIL_PIXELS {
        return styled_preview::thumbnail_with_styled_preparation_budget(doc, side, control, budget);
    }
    let scale = (side as f32 / longest as f32).min(1.);
    let output = ((doc.size.width as f32 * scale).round().max(1.) as usize, (doc.size.height as f32 * scale).round().max(1.) as usize);
    let snapshot = Snapshot::new(doc, k, control, budget)?;
    let result = styled_preview::thumbnail_with_styled_preparation_budget_sized(&snapshot.doc, side, control, budget, Some(output));
    drop(snapshot);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, LayerMask, SampleType};
    use preview_budget::PreparationCharge;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    fn own(depth: SampleType) -> Document {
        let mut doc = Document::with_background("Own header Ω".repeat(600), Size::new(47, 35), ColorMode::Rgb, depth, Color::rgba(0.21, 0.63, 0.17, 0.71));
        doc.layers[0].name = "Own layer λ".repeat(700);
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(-7, 3, 31, 24), &[0.73, 0.18, 0.51, 0.0]);
        let mut mask = LayerMask::reveal_all();
        std::sync::Arc::make_mut(&mut mask.surface).fill_rect(Rect::new(-3, 5, 36, 28), &[0.73]);
        mask.feather = 2.25;
        mask.density = 0.81;
        mask.linked = false;
        doc.layers[0].mask = Some(mask);
        doc.layers[0].opacity = 0.87;
        doc.layers[0].fill_opacity = 0.79;
        doc.layers[0].locks.position = false;
        doc.metadata.xmp = Some("Own unrendered metadata".repeat(10000));
        doc
    }
    #[test]
    fn render_snapshot_matches_operative_legacy_fields_and_raw_defaults() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = own(depth);
            let before = doc.clone();
            if depth == SampleType::F32 {
                let fmt = doc.pixel_format();
                let bytes = [(-0.0f32).to_ne_bytes(), 0.2f32.to_ne_bytes(), 0.4f32.to_ne_bytes(), 0.0f32.to_ne_bytes()].concat();
                doc.layers.push(Layer::new("Own default bits", LayerContent::Raster(Surface::try_with_default_bytes(fmt, &bytes).unwrap())));
            }
            let observer = |_| {};
            let budget = PreviewPreparationBudget::new(16 << 20, &observer);
            let snapshot = Snapshot::new(&doc, 3, &Interrupt::NONE, &budget).unwrap();
            let legacy = proxy::proxy_document(&doc, 3).unwrap();
            assert_eq!(render(&snapshot.doc, snapshot.doc.bounds()).unwrap(), render(&legacy, legacy.bounds()).unwrap());
            assert!(snapshot.doc.metadata.xmp.is_none());
            for (actual, expected) in snapshot.doc.layers.iter().zip(&legacy.layers) {
                assert_eq!(actual.name, expected.name);
                assert_eq!(actual.id, expected.id);
                assert_eq!(actual.locks, expected.locks);
                assert_eq!(actual.opacity, expected.opacity);
                assert_eq!(actual.fill_opacity, expected.fill_opacity);
                assert_eq!(actual.mask, expected.mask);
                let format = actual.surface().unwrap().format();
                let len = format.bytes_per_pixel();
                let (mut a, mut b) = (vec![0; len], vec![0; len]);
                for y in snapshot.doc.bounds().y0..snapshot.doc.bounds().y1 {
                    for x in snapshot.doc.bounds().x0..snapshot.doc.bounds().x1 {
                        actual.surface().unwrap().try_read_pixel_bytes(x, y, &mut a).unwrap();
                        expected.surface().unwrap().try_read_pixel_bytes(x, y, &mut b).unwrap();
                        assert_eq!(a, b);
                    }
                }
                assert_eq!(actual.surface().unwrap().default_bytes(), expected.surface().unwrap().default_bytes());
            }
            assert!(budget.usage().live_bytes > 0);
            drop(snapshot);
            assert_eq!(budget.usage().live_bytes, 0);
            assert_eq!(doc.metadata, before.metadata);
        }
    }
    #[test]
    fn cancelling_metadata_or_plane_reservation_allocates_no_published_snapshot() {
        let doc = own(SampleType::U16);
        let before = doc.clone();
        for stop in [1, 2, 3] {
            let cancel = AtomicBool::new(false);
            let reserves = AtomicUsize::new(0);
            let observer = |e| {
                if matches!(e, PreparationCharge::Reserve(_)) && reserves.fetch_add(1, Ordering::Relaxed) + 1 == stop {
                    cancel.store(true, Ordering::Relaxed);
                }
            };
            let budget = PreviewPreparationBudget::new(16 << 20, &observer);
            let cancelled = || cancel.load(Ordering::Relaxed);
            assert!(matches!(Snapshot::new(&doc, 3, &Interrupt::cancel_only(&cancelled), &budget), Err(RasterInputError::Cancelled)));
            assert_eq!(budget.usage().live_bytes, 0);
            assert_eq!(doc, before);
        }
        let observer = |_| {};
        let tiny = PreviewPreparationBudget::new(4095, &observer);
        assert!(matches!(Snapshot::new(&doc, 3, &Interrupt::NONE, &tiny), Err(RasterInputError::LimitExceeded)));
        assert_eq!(tiny.usage().live_bytes, 0);
    }
    #[test]
    fn original_target_rounding_and_two_outputs_share_proxy_ledger() {
        let mut doc = Document::with_background("Own proxy target", Size::new(4097, 4099), ColorMode::Rgb, SampleType::U8, Color::rgba(0.21, 0.63, 0.17, 0.71));
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(5, 7, 417, 311), &[0.73, 0.18, 0.51, 0.0]);
        let before = doc.clone();
        let observer = |_| {};
        let budget = PreviewPreparationBudget::new(64 << 20, &observer);
        let first = thumbnail_with_proxy_preparation_budget(&doc, 256, &Interrupt::NONE, &budget).unwrap();
        assert_eq!(first.image(), &thumbnail(&doc, 256).unwrap());
        assert_eq!(budget.usage().live_bytes, first.image().pixels.capacity());
        let second = thumbnail_with_proxy_preparation_budget(&doc, 1024, &Interrupt::NONE, &budget).unwrap();
        assert_eq!(second.image(), &thumbnail(&doc, 1024).unwrap());
        assert_eq!(budget.usage().live_bytes, first.image().pixels.capacity() + second.image().pixels.capacity());
        assert!(budget.usage().peak_bytes <= 64 << 20);
        drop(first);
        drop(second);
        assert_eq!(budget.usage().live_bytes, 0);
        assert_eq!(doc, before);
    }
}
