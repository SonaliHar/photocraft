//! Narrow current-state preflight for caller-accounted, plain Normal preparation.
use super::PreparationBudget;
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{
    ColorMode, Document, LayerContent, SampleEncoding, SampleType, WorkingProfileInterpretation,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::ItemRef,
};
use photocraft_raster::{Interrupt, RasterInputError};
fn unsupported() -> CompositionError {
    CompositionError::Unsupported("budgeted single plain Normal current state")
}
/// Cheap membership only: malformed samples/current ICC still enter fallible validation.
pub(crate) fn supports(doc: &Document) -> bool {
    if doc.layers.len() != 1
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(doc.sample_encoding, Some(SampleEncoding::NonlinearProfile | SampleEncoding::Linear | SampleEncoding::Perceptual))
        || !matches!(doc.depth, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        || !matches!(
            (&doc.working_profile_interpretation, doc.mode),
            (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) | (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale)
        )
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.quick_mask.is_some()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
    {
        return false;
    }
    let Some(layer) = doc.layers.first() else {
        return false;
    };
    // Existing hidden/partial-opacity and finite-alpha families keep their
    // compatibility route. Malformed opacity remains fallible scoped input.
    if !layer.visible || (layer.opacity.is_finite() && layer.opacity != 1.0) || (layer.fill_opacity.is_finite() && layer.fill_opacity != 1.0) {
        return false;
    }
    if !matches!(layer.content, LayerContent::Raster(_))
        || layer.blend != photocraft_color::blend::BlendMode::Normal
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || crate::effects::has_effects(layer)
        || layer.clipped
        || layer.video.is_some()
        || layer.fill_cache.is_some()
        || layer.indexed_source.is_some()
        || layer.excluded_channels != 0
        || layer.advanced != photocraft_doc::AdvancedBlending::default()
        || layer.blend_if != photocraft_doc::BlendIf::default()
    {
        return false;
    }
    let Some(state) = &doc.xcf_state else {
        return layer.import_compositing.is_none();
    };
    if state.items.len() != 1
        || !state.resources.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.filter_stacks.is_empty()
        || !state.filter_masks.is_empty()
        || state.floating.is_some()
        || !state.item_sets.is_empty()
    {
        return false;
    }
    let Some(item) = state.items.first() else {
        return false;
    };
    let Some(policy) = &item.composition else {
        return false;
    };
    let Some(resolved) = &policy.resolved else {
        return false;
    };
    item.item == ItemRef::Layer(layer.id)
        && item.parent.is_none()
        && item.mask.is_none()
        && policy.compatibility_epoch != photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH
        && policy.source_mode == 28
        && !policy.legacy
        && !policy.pass_through
        && resolved.operator == GimpOperator::Union
        && matches!(resolved.composite_space, GimpSpace::LinearRgb | GimpSpace::ProfileRgb)
        && matches!(resolved.blend_space, GimpSpace::LinearRgb | GimpSpace::ProfileRgb | GimpSpace::PerceptualRgb)
}
pub(crate) fn validate(doc: &Document, interrupt: &Interrupt, budget: &mut impl PreparationBudget) -> Result<()> {
    if !supports(doc) {
        return Err(unsupported());
    }
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let count = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(unsupported)?;
    if count == 0 || count > 1_u64 << 30 || doc.size.width > 262144 || doc.size.height > 262144 {
        return Err(unsupported());
    }
    let layer = doc.layers.first().ok_or_else(unsupported)?;
    if !layer.visible || layer.opacity != 1.0 || layer.fill_opacity != 1.0 {
        return Err(unsupported());
    }
    if let Some(state) = &doc.xcf_state {
        let item = state.items.first().ok_or_else(unsupported)?;
        let extent = item.raster_bounds.ok_or_else(unsupported)?;
        let width = i64::from(extent.x1).checked_sub(i64::from(extent.x0)).ok_or_else(unsupported)?;
        let height = i64::from(extent.y1).checked_sub(i64::from(extent.y0)).ok_or_else(unsupported)?;
        if !item.visible || width <= 0 || height <= 0 || width > 262144 || height > 262144 {
            return Err(unsupported());
        }
    }
    let surface = layer.surface().ok_or_else(unsupported)?;
    let format = surface.format();
    if format != doc.pixel_format() || !format.alpha {
        return Err(unsupported());
    }
    let channels = format.channels();
    let color_channels = format.mode.color_channels();
    let pixel_bytes = format.bytes_per_pixel();
    let tile_bytes = 65536_usize.checked_mul(pixel_bytes).ok_or_else(unsupported)?;
    let check = |bytes: &[u8], pixels: usize, budget: &mut dyn FnMut(u64) -> Result<()>| -> Result<()> {
        if bytes.len() != pixels.checked_mul(pixel_bytes).ok_or_else(unsupported)? {
            return Err(unsupported());
        }
        for start in (0..pixels).step_by(64) {
            interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
            let units = (pixels - start).min(64).checked_mul(channels).ok_or_else(unsupported)?;
            budget(u64::try_from(units).map_err(|_| unsupported())?)?;
            let sample_start = start.checked_mul(channels).ok_or_else(unsupported)?;
            for index in sample_start..sample_start.checked_add(units).ok_or_else(unsupported)? {
                let value = photocraft_color::try_read_sample_f64(bytes, format.sample, index).map_err(|_| unsupported())?;
                // Color is finite HDR data; coverage remains a strict unit domain.
                // Preserve all current sample bits, including hidden/offbounds color.
                if !value.is_finite() || (index % channels == color_channels && !(0.0..=1.0).contains(&value)) {
                    return Err(unsupported());
                }
            }
        }
        Ok(())
    };
    check(surface.default_bytes(), 1, &mut |units| budget.consume_preparation_work(units))?;
    for (coordinate, tile) in surface.tiles() {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        for axis in [coordinate.tx, coordinate.ty] {
            let origin = axis.checked_mul(photocraft_geom::TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
            origin.checked_add(photocraft_geom::TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        }
        if tile.bytes().len() != tile_bytes {
            return Err(unsupported());
        }
        check(tile.bytes(), 65536, &mut |units| budget.consume_preparation_work(units))?;
    }
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::super::{PreparationBudget, PreparedRenderer};
    use crate::gimp_composite::{CompositionError, Result};
    use photocraft_doc::{ColorMode, Document, Layer, SampleEncoding, SampleType, Size, WorkingProfileInterpretation};
    use photocraft_raster::Interrupt;
    use std::{cell::Cell, rc::Rc};
    struct Lease(Rc<Cell<u64>>, u64);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.set(self.0.get() - self.1);
        }
    }
    struct Budget {
        live: Rc<Cell<u64>>,
        limit: u64,
        work: u64,
    }
    impl Budget {
        fn new(limit: u64) -> Self {
            Self { live: Rc::new(Cell::new(0)), limit, work: 0 }
        }
    }
    impl PreparationBudget for Budget {
        type Lease = Lease;
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<Lease> {
            let next = self.live.get().checked_add(bytes).ok_or(CompositionError::Invalid("test overflow"))?;
            if next > self.limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            self.live.set(next);
            Ok(Lease(self.live.clone(), bytes))
        }
        fn consume_preparation_work(&mut self, units: u64) -> Result<()> {
            self.work += units;
            Ok(())
        }
    }
    fn doc() -> Document {
        let mut doc = Document::new("Plain Normal", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let bytes = photocraft_cms::Builtin::Srgb.profile().to_bytes();
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&bytes);
        doc.icc_profile = Some(bytes);
        let mut layer = Layer::raster("Literal", doc.pixel_format());
        layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.125, 0.5, 0.875, 0.75]).unwrap();
        doc.layers.push(layer);
        doc
    }
    #[test]
    fn plain_normal_preparation_retains_and_refunds_caller_leases() {
        let doc = doc();
        let mut budget = Budget::new(8 << 20);
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let renderer = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        assert!(budget.live.get() > 123);
        let mut work = 0;
        let output = renderer
            .renderer()
            .render_reduced_with_interrupt(
                2,
                2,
                None,
                &mut |event| {
                    if let photocraft_cms::WideWork::SampleUnits(units) = event {
                        work += units;
                    }
                    Ok(())
                },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(output.px.len(), 4);
        assert_eq!(work, 128 * 8 * 6);
        drop(renderer);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn invalid_default_offbounds_and_current_binding_do_not_escape_selection() {
        for kind in 0..5 {
            let mut doc = doc();
            match kind {
                0 => {
                    let bytes: Vec<_> = [0.1_f64, f64::NAN, 0.3, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect();
                    *doc.layers[0].surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(doc.pixel_format(), &bytes).unwrap();
                }
                1 => {
                    doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &[0.1, f64::NAN, 0.3, 1.0]).unwrap();
                }
                2 => {
                    doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &[0.1, 0.2, 0.3, 1.0001]).unwrap();
                }
                3 => {
                    doc.icc_profile = Some(std::sync::Arc::new(vec![0; 200]));
                }
                _ => {
                    doc.icc_profile = None;
                }
            }
            assert!(PreparedRenderer::supports_plain_normal_budgeted(&doc));
            let surface = doc.layers[0].surface().unwrap().clone();
            let mut budget = Budget::new(8 << 20);
            let unrelated = budget.reserve_prepared_bytes(117).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            assert!(PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
            assert_eq!(budget.live.get(), 117);
            assert_eq!(doc.layers[0].surface().unwrap(), &surface);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn preparation_denial_and_mid_tile_cancel_refund_only_owned_leases() {
        let doc = doc();
        let mut budget = Budget::new(124);
        let source = budget.reserve_prepared_bytes(123).unwrap();
        assert!(PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
        assert_eq!(budget.work, 0);
        assert_eq!(budget.live.get(), 0);
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || {
            let n = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            n >= 10
        };
        let mut budget = Budget::new(8 << 20);
        let unrelated = budget.reserve_prepared_bytes(17).unwrap();
        let source = budget.reserve_prepared_bytes(123).unwrap();
        assert!(matches!(
            PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::cancel_only(&cancel), &mut budget, source),
            Err(CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))
        ));
        assert!(budget.work > 4 && budget.work < 65536 * 4);
        assert_eq!(budget.live.get(), 17);
        drop(unrelated);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn damage_reduction_work_and_failure_release_are_exact() {
        let doc = doc();
        let mut budget = Budget::new(8 << 20);
        let renderer = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
        let mut work = 0;
        let damage = photocraft_geom::Rect::new(0, 0, 1, 1);
        let output = renderer
            .renderer()
            .render_reduced_with_interrupt(
                2,
                2,
                Some(damage),
                &mut |event| {
                    if let photocraft_cms::WideWork::SampleUnits(units) = event {
                        work += units;
                    }
                    Ok(())
                },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(output.px.len(), 1);
        assert_eq!(work, 128 * 4 * 3);
        let mut sampled = 0;
        let mut live = 0;
        assert!(
            renderer
                .renderer()
                .render_reduced_with_interrupt(
                    2,
                    2,
                    None,
                    &mut |event| {
                        match event {
                            photocraft_cms::WideWork::SampleUnits(128) => {
                                if sampled == 3 {
                                    return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
                                }
                                sampled += 1;
                            }
                            photocraft_cms::WideWork::ReserveBytes(n) => live += n,
                            photocraft_cms::WideWork::ReleaseBytes(n) => live -= n,
                            _ => {}
                        }
                        Ok(())
                    },
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert_eq!(sampled, 3);
        assert_eq!(live, 0);
    }
    #[test]
    fn malformed_tile_origin_is_rejected_and_current_low_bits_are_untouched() {
        let mut doc = doc();
        let before = doc.layers[0].surface().unwrap().try_read_interleaved(photocraft_geom::Rect::new(0, 0, 1, 1)).unwrap();
        {
            let mut budget = Budget::new(8 << 20);
            let renderer = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
            assert!(budget.live.get() > 0);
            drop(renderer);
        }
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(photocraft_geom::Rect::new(0, 0, 1, 1)).unwrap(), before);
        let tile = doc.layers[0].surface().unwrap().tiles().next().unwrap().1.clone();
        doc.layers[0].surface_mut().unwrap().put_tiles([(photocraft_geom::TileCoord { tx: i32::MAX, ty: 0 }, tile)]);
        let mut budget = Budget::new(8 << 20);
        assert!(PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn work_denial_precedes_default_sample_interpretation() {
        struct Deny {
            live: Rc<Cell<u64>>,
            calls: usize,
        }
        impl PreparationBudget for Deny {
            type Lease = Lease;
            fn reserve_prepared_bytes(&mut self, n: u64) -> Result<Lease> {
                self.live.set(n);
                Ok(Lease(self.live.clone(), n))
            }
            fn consume_preparation_work(&mut self, _: u64) -> Result<()> {
                self.calls += 1;
                Err(photocraft_raster::RasterInputError::LimitExceeded.into())
            }
        }
        let mut doc = doc();
        let raw: Vec<_> = [f64::NAN, 0., 0., 1.].into_iter().flat_map(f64::to_ne_bytes).collect();
        *doc.layers[0].surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(doc.pixel_format(), &raw).unwrap();
        let mut budget = Deny { live: Rc::new(Cell::new(0)), calls: 0 };
        assert!(matches!(
            PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()),
            Err(CompositionError::Raster(photocraft_raster::RasterInputError::LimitExceeded))
        ));
        assert_eq!(budget.calls, 1);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn current_rgb_gray_native_formats_preserve_default_and_adjacent_values() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
                let mut doc = Document::new("Native current scope", Size::new(8, 6), mode, sample);
                doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
                let bytes = if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb } else { photocraft_cms::Builtin::SGray }.profile().to_bytes();
                doc.working_profile_interpretation = if mode == ColorMode::Rgb {
                    WorkingProfileInterpretation::implicit_srgb(&bytes)
                } else {
                    WorkingProfileInterpretation::implicit_sgray(&bytes)
                };
                doc.icc_profile = Some(bytes);
                let mut layer = Layer::raster("Adjacent literals", doc.pixel_format());
                let value = if sample == SampleType::U32 { 2147483649.0 / f64::from(u32::MAX) } else { f64::from_bits(0.5_f64.to_bits() + 1) };
                let pixel = if mode == ColorMode::Rgb { vec![-0., value, 0.875, 0.75] } else { vec![value, 0.75] };
                layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &pixel).unwrap();
                let before = layer.surface().unwrap().clone();
                doc.layers.push(layer);
                let mut budget = Budget::new(8 << 20);
                let renderer = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
                assert_eq!(budget.work, doc.pixel_format().channels() as u64 * (65536 + 1) + 2 * doc.icc_profile.as_ref().unwrap().len() as u64);
                assert_eq!(doc.layers[0].surface().unwrap(), &before);
                drop(renderer);
                assert_eq!(budget.live.get(), 0);
            }
        }
    }
    #[test]
    fn finite_hdr_colors_preserve_all_retained_bits_and_unit_alpha() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for sample in [SampleType::F32, SampleType::F64] {
                let mut doc = Document::new("Finite HDR current", Size::new(8, 6), mode, sample);
                doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
                let bytes = if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb } else { photocraft_cms::Builtin::SGray }.profile().to_bytes();
                doc.working_profile_interpretation = if mode == ColorMode::Rgb {
                    WorkingProfileInterpretation::implicit_srgb(&bytes)
                } else {
                    WorkingProfileInterpretation::implicit_sgray(&bytes)
                };
                doc.icc_profile = Some(bytes);
                let pixel = if mode == ColorMode::Rgb { vec![-0.125, 1.5, 4.0, 0.75] } else { vec![-0.125, 0.75] };
                let mut raw = vec![0; doc.pixel_format().bytes_per_pixel()];
                for (index, value) in pixel.iter().copied().enumerate() {
                    photocraft_color::try_write_sample_f64(&mut raw, sample, index, value).unwrap();
                }
                let mut layer = Layer::raster("Exact HDR literals", doc.pixel_format());
                *layer.surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(doc.pixel_format(), &raw).unwrap();
                layer.surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &pixel).unwrap();
                layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &pixel).unwrap();
                let before = layer.surface().unwrap().clone();
                doc.layers.push(layer);
                let mut budget = Budget::new(16 << 20);
                let prepared = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
                let output = prepared.renderer().render_reduced_with_interrupt(8, 6, None, &mut |_| Ok(()), &Interrupt::NONE).unwrap();
                assert_eq!(output.px.len(), 48);
                assert_eq!(doc.layers[0].surface().unwrap(), &before);
                drop(prepared);
                assert_eq!(budget.live.get(), 0);
            }
        }
    }
    #[test]
    fn hdr_does_not_bypass_malformed_samples_or_render_overflow() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for sample_index in 0..4 {
                let mut doc = doc();
                let mut pixel = [-0.125, 1.5, 4.0, 0.75];
                pixel[sample_index] = bad;
                doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &pixel).unwrap();
                let before = doc.layers[0].surface().unwrap().clone();
                let mut budget = Budget::new(16 << 20);
                assert!(PreparedRenderer::supports_plain_normal_budgeted(&doc));
                assert!(PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
                assert_eq!(budget.live.get(), 0);
                assert_eq!(doc.layers[0].surface().unwrap(), &before);
            }
        }
        for alpha in [-0.001, 1.0001] {
            let mut doc = doc();
            doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &[-0.125, 1.5, 4.0, alpha]).unwrap();
            let mut budget = Budget::new(16 << 20);
            assert!(PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
            assert_eq!(budget.live.get(), 0);
        }
        let mut doc = doc();
        doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[f64::MAX, f64::MAX, f64::MAX, 1.0]).unwrap();
        let before = doc.layers[0].surface().unwrap().clone();
        let mut budget = Budget::new(16 << 20);
        let prepared = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
        let mut live_output = 0;
        let rendered = prepared.renderer().render_reduced_with_interrupt(
            8,
            6,
            None,
            &mut |event| {
                match event {
                    photocraft_cms::WideWork::ReserveBytes(n) => live_output += n,
                    photocraft_cms::WideWork::ReleaseBytes(n) => live_output -= n,
                    _ => {}
                }
                Ok(())
            },
            &Interrupt::NONE,
        );
        assert!(rendered.is_err(), "extreme finite transfer must fail before publishing output");
        assert_eq!(live_output, 0);
        assert_eq!(doc.layers[0].surface().unwrap(), &before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn declared_linear_perceptual_current_profiles_use_checked_budgeted_preparation() {
        for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let mut doc = Document::new("Declared transfer current", Size::new(8, 6), mode, SampleType::F64);
                doc.sample_encoding = Some(encoding);
                let space = if mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray };
                let stored = match encoding {
                    SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
                    SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
                    SampleEncoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                };
                let bytes = photocraft_cms::xcf_color::effective_profile(None, space, stored).unwrap().to_bytes();
                doc.working_profile_interpretation = if mode == ColorMode::Rgb {
                    WorkingProfileInterpretation::implicit_srgb(&bytes)
                } else {
                    WorkingProfileInterpretation::implicit_sgray(&bytes)
                };
                doc.icc_profile = Some(bytes);
                let mut layer = Layer::raster("Unchanged current values", doc.pixel_format());
                let pixel: &[f64] = if mode == ColorMode::Rgb { &[-0.125, 1.5, 4.0, 0.75] } else { &[1.5, 0.75] };
                layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, pixel).unwrap();
                let before = layer.surface().unwrap().clone();
                doc.layers.push(layer);
                assert!(PreparedRenderer::supports_plain_normal_budgeted(&doc));
                let mut budget = Budget::new(16 << 20);
                let prepared = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
                let mut work = 0;
                let output = prepared
                    .renderer()
                    .render_reduced_with_interrupt(
                        8,
                        6,
                        None,
                        &mut |event| {
                            if let photocraft_cms::WideWork::SampleUnits(n) = event {
                                work += n;
                            }
                            Ok(())
                        },
                        &Interrupt::NONE,
                    )
                    .unwrap();
                assert_eq!(output.px.len(), 48);
                assert_eq!(work, 128 * 48);
                assert_eq!(doc.layers[0].surface().unwrap(), &before);
                drop(prepared);
                assert_eq!(budget.live.get(), 0);
            }
        }
    }
    #[test]
    fn encoding_extension_keeps_current_profile_and_sample_failures_atomic() {
        for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
                    let mut current = Document::new("Current transfer formats", Size::new(8, 6), mode, sample);
                    current.sample_encoding = Some(encoding);
                    let space = if mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray };
                    let transfer = if encoding == SampleEncoding::Linear {
                        photocraft_cms::xcf_color::Encoding::Linear
                    } else {
                        photocraft_cms::xcf_color::Encoding::Perceptual
                    };
                    let correct = photocraft_cms::xcf_color::effective_profile(None, space, transfer).unwrap().to_bytes();
                    current.working_profile_interpretation = if mode == ColorMode::Rgb {
                        WorkingProfileInterpretation::implicit_srgb(&correct)
                    } else {
                        WorkingProfileInterpretation::implicit_sgray(&correct)
                    };
                    current.icc_profile = Some(correct.clone());
                    // Immutable source provenance is inert; current storage and ICC are operative.
                    current.source_color_context =
                        Some(photocraft_doc::SourceColorContext { original_icc: Some(std::sync::Arc::new(vec![0xff; 140])), source_precision: 550 });
                    let mut layer = Layer::raster("Current adjacent literal", current.pixel_format());
                    let value = if sample == SampleType::U32 { 2147483649.0 / f64::from(u32::MAX) } else { f64::from_bits(0.5_f64.to_bits() + 1) };
                    let pixel: &[f64] = if mode == ColorMode::Rgb { &[-0.0, value, 0.875, 0.75] } else { &[value, 0.75] };
                    layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, pixel).unwrap();
                    current.layers.push(layer);
                    let before = current.clone();
                    let mut budget = Budget::new(16 << 20);
                    drop(PreparedRenderer::new_plain_normal_budgeted(&current, &Interrupt::NONE, &mut budget, ()).unwrap());
                    assert_eq!(budget.live.get(), 0);
                    assert_eq!(current, before);
                    // A fresh valid fingerprint cannot authorize the wrong transfer curves.
                    let wrong_transfer = if encoding == SampleEncoding::Linear {
                        photocraft_cms::xcf_color::Encoding::NonlinearProfile
                    } else {
                        photocraft_cms::xcf_color::Encoding::Linear
                    };
                    let wrong = photocraft_cms::xcf_color::effective_profile(None, space, wrong_transfer).unwrap().to_bytes();
                    current.working_profile_interpretation = if mode == ColorMode::Rgb {
                        WorkingProfileInterpretation::implicit_srgb(&wrong)
                    } else {
                        WorkingProfileInterpretation::implicit_sgray(&wrong)
                    };
                    current.icc_profile = Some(wrong);
                    let invalid_before = current.clone();
                    assert!(PreparedRenderer::supports_plain_normal_budgeted(&current));
                    assert!(PreparedRenderer::new_plain_normal_budgeted(&current, &Interrupt::NONE, &mut budget, ()).is_err());
                    assert_eq!(current, invalid_before);
                    assert_eq!(budget.live.get(), 0);
                }
            }
        }
        for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
            for bad in [f64::NAN, f64::INFINITY, -0.001, 1.001] {
                let mut current = doc();
                current.sample_encoding = Some(encoding);
                let transfer = if encoding == SampleEncoding::Linear {
                    photocraft_cms::xcf_color::Encoding::Linear
                } else {
                    photocraft_cms::xcf_color::Encoding::Perceptual
                };
                let bytes = photocraft_cms::xcf_color::effective_profile(None, photocraft_cms::ColorSpace::Rgb, transfer).unwrap().to_bytes();
                current.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&bytes);
                current.icc_profile = Some(bytes);
                current.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &[-0.125, 1.5, 4.0, bad]).unwrap();
                let mut budget = Budget::new(16 << 20);
                assert!(PreparedRenderer::new_plain_normal_budgeted(&current, &Interrupt::NONE, &mut budget, ()).is_err());
                assert_eq!(budget.live.get(), 0);
            }
        }
    }
    #[test]
    fn canonical_policy_is_preserved_and_other_supported_families_keep_selection() {
        use photocraft_doc::{
            XcfState,
            import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
            item_identity::ItemRef,
            xcf_state::{ItemLocks, ItemState},
        };
        let mut doc = doc();
        let layer = &doc.layers[0];
        let policy = GimpComposition {
            compatibility_epoch: "xcf-11-normal".into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 0,
            stored_composite_space: -1,
            stored_operator: -1,
            pass_through: false,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        };
        doc.xcf_state = Some(XcfState {
            items: vec![ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: Some(1),
                source_flags: 0,
                raster_bounds: Some(doc.bounds()),
                mask: None,
                composition: Some(policy.clone()),
            }],
            ..Default::default()
        });
        let mut budget = Budget::new(8 << 20);
        {
            let prepared = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).unwrap();
            assert!(budget.live.get() > 0);
            drop(prepared);
        }
        assert_eq!(doc.xcf_state.as_ref().unwrap().items[0].composition.as_ref(), Some(&policy));
        doc.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().compatibility_epoch =
            photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH.into();
        assert!(!PreparedRenderer::supports_plain_normal_budgeted(&doc));
        doc.xcf_state = None;
        doc.layers[0].opacity = 0.5;
        assert!(!PreparedRenderer::supports_plain_normal_budgeted(&doc));
        doc.layers[0].opacity = f32::NAN;
        assert!(PreparedRenderer::supports_plain_normal_budgeted(&doc));
        assert!(PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
    }
    #[test]
    #[ignore = "24MP release preparation and cancellation measurement"]
    fn plain_normal_preparation_24mp() {
        use std::time::{Duration, Instant};
        let mut doc = Document::new("24MP checked preparation", Size::new(6000, 4000), ColorMode::Grayscale, SampleType::F64);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let bytes = photocraft_cms::Builtin::SGray.profile().to_bytes();
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_sgray(&bytes);
        doc.icc_profile = Some(bytes);
        let mut layer = Layer::raster("Retained unit tiles", doc.pixel_format());
        for y in (0..4000).step_by(256) {
            for x in (0..6000).step_by(256) {
                layer.surface_mut().unwrap().try_write_pixel_f64(x, y, &[0.25, 1.0]).unwrap();
            }
        }
        let source_bytes = layer.surface().unwrap().resident_payload_bytes().unwrap();
        doc.layers.push(layer);
        let mut budget = Budget::new(512 << 20);
        let source = budget.reserve_prepared_bytes(source_bytes).unwrap();
        let start = Instant::now();
        let prepared = PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        eprintln!(
            "24MP GrayF64 retained_source_bytes={source_bytes} preparation_ms={} preparation_work={} resident_charged={}",
            start.elapsed().as_secs_f64() * 1000.0,
            budget.work,
            budget.live.get()
        );
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
        let source = budget.reserve_prepared_bytes(source_bytes).unwrap();
        let start = Instant::now();
        let cancel = || start.elapsed() >= Duration::from_millis(50);
        assert!(matches!(
            PreparedRenderer::new_plain_normal_budgeted(&doc, &Interrupt::cancel_only(&cancel), &mut budget, ()),
            Err(CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))
        ));
        eprintln!("24MP GrayF64 50ms deadline observed_ms={}", start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(budget.live.get(), source_bytes);
        drop(source);
        assert_eq!(budget.live.get(), 0);
    }
}
