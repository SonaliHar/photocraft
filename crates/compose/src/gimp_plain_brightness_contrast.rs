//! Closed caller-budgeted singleton Brightness/Contrast preparation.
use super::{PreparationBudget, plain_point};
use crate::gimp_composite::Result;
use photocraft_doc::Document;
use photocraft_raster::Interrupt;
pub(crate) fn supports(doc: &Document) -> bool {
    plain_point::supports_singleton(doc, plain_point::Singleton::BrightnessContrast)
}
pub(crate) fn preparation_bytes(icc_bytes: usize) -> Result<u64> {
    plain_point::preparation_bytes(icc_bytes)
}
pub(crate) fn validate(doc: &Document, ctl: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    plain_point::validate_singleton(doc, ctl, budget, plain_point::Singleton::BrightnessContrast)
}

#[cfg(test)]
mod tests {
    use super::super::{PreparationBudget, PreparedRenderer};
    use crate::gimp_composite::{CompositionError, Result};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
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
        work_limit: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = Lease;
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<Lease> {
            let next = self.live.get().checked_add(n).ok_or(CompositionError::Invalid("test overflow"))?;
            if next > self.limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            self.live.set(next);
            Ok(Lease(self.live.clone(), n))
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            let next = self.work.checked_add(n).ok_or(CompositionError::Invalid("test work overflow"))?;
            if next > self.work_limit {
                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
            }
            self.work = next;
            Ok(())
        }
    }
    fn doc() -> photocraft_doc::Document {
        let mut doc = super::super::plain_point::tests::doc();
        doc.source_color_context = Some(photocraft_doc::SourceColorContext { original_icc: None, source_precision: 150 });
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.name = "gegl:brightness-contrast".into();
        node.operation.stored_version = "0:0".into();
        node.operation.schema_epoch = crate::gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH.into();
        node.arguments = vec![
            FilterArgument { name: "contrast".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1065353216)) },
            FilterArgument { name: "brightness".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0)) },
        ];
        doc
    }
    #[test]
    fn singleton_brightness_contrast_preparation_retains_existing_ledger() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 16 << 20, work: 0, work_limit: u64::MAX };
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let prepared = PreparedRenderer::new_plain_brightness_contrast_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        let mut live_output = 0;
        let mut peak = 0;
        let mut work = 0;
        let out = prepared
            .renderer()
            .render_reduced_with_interrupt(
                32,
                32,
                None,
                &mut |event| {
                    match event {
                        photocraft_cms::WideWork::ReserveBytes(n) => {
                            live_output += n;
                            peak = peak.max(live_output)
                        }
                        photocraft_cms::WideWork::ReleaseBytes(n) => live_output -= n,
                        photocraft_cms::WideWork::SampleUnits(n) => work += n,
                    };
                    Ok(())
                },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(out.px.len(), 1024);
        assert_eq!(peak, 1024 * 16);
        assert_eq!(work, 256 * 1024);
        assert_eq!(doc, before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn preparation_resident_work_and_mid_scan_cancel_refund_source_and_preparation() {
        for mode in 0..3 {
            let doc = doc();
            let before = doc.clone();
            let mut budget = Budget {
                live: Rc::new(Cell::new(0)),
                limit: if mode == 0 { 124 } else { 48 << 20 },
                work: 0,
                work_limit: if mode == 1 { 1000 } else { u64::MAX },
            };
            let unrelated = budget.reserve_prepared_bytes(1).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            let polls = std::sync::atomic::AtomicUsize::new(0);
            let cancel = || mode == 2 && polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 10;
            let ctl = Interrupt::cancel_only(&cancel);
            assert!(PreparedRenderer::new_plain_brightness_contrast_budgeted(&doc, &ctl, &mut budget, source).is_err());
            if mode == 2 {
                assert!(polls.load(std::sync::atomic::Ordering::Relaxed) >= 10);
                assert!(budget.work > 0);
            }
            assert_eq!(doc, before);
            assert_eq!(budget.live.get(), 1);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn valid_out_of_slice_tuples_keep_old_selection_and_malformed_stay_scoped() {
        for (brightness, contrast) in [(0_f32, 1.25_f32), (0.25, 1.0), (-0.0, 1.0)] {
            let mut doc = doc();
            let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            node.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(contrast.to_bits()));
            node.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(brightness.to_bits()));
            assert!(!PreparedRenderer::supports_plain_brightness_contrast_budgeted(&doc));
            assert!(PreparedRenderer::new_with_interrupt(&doc, &Interrupt::NONE).is_ok());
        }
    }
    #[test]
    fn scoped_invalid_context_refunds_before_publication() {
        for kind in 0..13 {
            let mut doc = doc();
            match kind {
                0 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()))
                }
                1 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].value =
                        FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits()))
                }
                2 => {
                    let duplicate = doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].clone();
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1] = duplicate;
                }
                3 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float64Bits(1_f64.to_bits()))
                }
                4 => doc.icc_profile = None,
                5 => doc.icc_profile = Some(std::sync::Arc::new(vec![0; 200])),
                6 => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(32, 0, &[0.1, 0.2, 0.3, 1.]).unwrap(),
                7 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(4096, -4096, &[1.]).unwrap(),
                8 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                9 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float32Bits(6_f32.to_bits()))
                }
                10 => {
                    let format = doc.pixel_format();
                    *doc.layers[0].surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(format, &[1, 0, 0, 0]).unwrap();
                }
                11 => {
                    doc.xcf_state.as_mut().unwrap().filter_masks[0].surface = photocraft_raster::Surface::try_with_default_bytes(
                        photocraft_color::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::U8, false),
                        &[1],
                    )
                    .unwrap()
                }
                _ => {
                    let tile = doc.layers[0].surface().unwrap().tiles().next().unwrap().1.clone();
                    doc.layers[0].surface_mut().unwrap().put_tiles([(photocraft_geom::TileCoord { tx: i32::MAX, ty: 0 }, tile)]);
                }
            }
            assert!(PreparedRenderer::supports_plain_brightness_contrast_budgeted(&doc));
            let before = doc.clone();
            let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 16 << 20, work: 0, work_limit: u64::MAX };
            let unrelated = budget.reserve_prepared_bytes(17).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            assert!(PreparedRenderer::new_plain_brightness_contrast_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
            assert_eq!(doc, before);
            assert_eq!(budget.live.get(), 17);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn failed_point_work_releases_output_but_retains_source_and_preparation() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 16 << 20, work: 0, work_limit: u64::MAX };
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let prepared = PreparedRenderer::new_plain_brightness_contrast_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        let retained = budget.live.get();
        let mut output = 0_u64;
        let mut samples = 0_u64;
        let result = prepared.renderer().render_reduced_with_interrupt(
            32,
            32,
            None,
            &mut |event| {
                match event {
                    photocraft_cms::WideWork::ReserveBytes(n) => output += n,
                    photocraft_cms::WideWork::ReleaseBytes(n) => output -= n,
                    photocraft_cms::WideWork::SampleUnits(n) => {
                        if samples + n > 128 {
                            return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
                        }
                        samples += n;
                    }
                }
                Ok(())
            },
            &Interrupt::NONE,
        );
        assert!(result.is_err());
        assert_eq!(samples, 128);
        assert_eq!(output, 0);
        assert_eq!(budget.live.get(), retained);
        assert_eq!(doc, before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
}
