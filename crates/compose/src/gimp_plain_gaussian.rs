//! Closed caller-budgeted singleton Gaussian Blur preparation.
use super::{PreparationBudget, plain_point};
use crate::gimp_composite::Result;
use photocraft_doc::Document;
use photocraft_raster::Interrupt;
pub(crate) fn supports(doc: &Document) -> bool {
    plain_point::supports_singleton(doc, plain_point::Singleton::Gaussian)
}
pub(crate) fn preparation_bytes(icc_bytes: usize) -> Result<u64> {
    plain_point::preparation_bytes(icc_bytes)
}
pub(crate) fn validate(doc: &Document, ctl: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    plain_point::validate_singleton(doc, ctl, budget, plain_point::Singleton::Gaussian)
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
        node.operation.name = "gegl:gaussian-blur".into();
        node.operation.stored_version = "0:0".into();
        node.operation.schema_epoch = "xcf-22-stock-gaussian-replace".into();
        node.composition.as_mut().unwrap().compatibility_epoch = "xcf-22-stock-gaussian-replace".into();
        node.composition.as_mut().unwrap().stored_blend_space = -856260015;
        node.composition.as_mut().unwrap().stored_composite_space = -856260015;
        node.composition.as_mut().unwrap().stored_operator = -856260015;
        node.arguments = vec![
            FilterArgument { name: "std-dev-x".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1074790400)) },
            FilterArgument { name: "std-dev-y".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1069547520)) },
            FilterArgument { name: "filter".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 } },
            FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 } },
            FilterArgument { name: "clip-extent".into(), value: FilterValue::Bool(true) },
        ];
        for y in 0..80 {
            for x in 0..96 {
                doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(x, y, &[0.125, 0.25, 0.5, 1.]).unwrap();
            }
        }
        doc.size = photocraft_doc::Size::new(96, 80);
        let bounds = photocraft_geom::Rect::new(0, 0, 96, 80);
        let state = doc.xcf_state.as_mut().unwrap();
        for item in &mut state.items {
            item.raster_bounds = Some(bounds);
            if let Some(mask) = &mut item.mask {
                mask.bounds = bounds;
            }
        }
        state.filter_masks[0].bounds = bounds;
        doc
    }
    #[test]
    fn singleton_gaussian_preparation_charges_real_cache_and_retains_source() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 48 << 20, work: 0, work_limit: u64::MAX };
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let prepared = PreparedRenderer::new_plain_gaussian_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        let mut events = vec![];
        let output = prepared
            .renderer()
            .render_reduced_with_interrupt(
                96,
                80,
                None,
                &mut |e| {
                    events.push(e);
                    Ok(())
                },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(output.px.len(), 7680);
        assert_eq!(events.iter().filter(|e| matches!(e, photocraft_cms::WideWork::ReserveBytes(33554432))).count(), 1);
        assert_eq!(events.iter().filter(|e| matches!(e, photocraft_cms::WideWork::ReleaseBytes(33554432))).count(), 1);
        assert_eq!(doc, before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn measured_gaussian_invalid_current_state_refunds_preparation() {
        for kind in 0..13 {
            let mut doc = doc();
            match kind {
                0 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float32Bits(4_f32.to_bits()))
                }
                1 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].value =
                        FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()))
                }
                2 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[2].value =
                        FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 1 }
                }
                3 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[3].value = FilterValue::Enum { domain: "wrong::abyss".into(), value: 1 }
                }
                4 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[4].value = FilterValue::Bool(false),
                5 => {
                    let duplicate = doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].clone();
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1] = duplicate;
                }
                6 => doc.icc_profile = None,
                7 => doc.icc_profile = Some(std::sync::Arc::new(vec![0; 200])),
                8 => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(96, 0, &[0.1, 0.2, 0.3, 1.]).unwrap(),
                9 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(4096, -4096, &[1.]).unwrap(),
                10 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = -261651887,
                11 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                _ => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(95, 79, &[0.125, 0.25, 0.5, 0.5]).unwrap(),
            }
            assert!(PreparedRenderer::supports_plain_gaussian_budgeted(&doc));
            let before = doc.clone();
            let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 48 << 20, work: 0, work_limit: u64::MAX };
            let unrelated = budget.reserve_prepared_bytes(17).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            assert!(PreparedRenderer::new_plain_gaussian_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
            assert_eq!(doc, before);
            assert_eq!(budget.live.get(), 17);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn gaussian_extent_is_closed_and_prior_families_keep_32_bounds() {
        let mut doc = doc();
        doc.size = photocraft_doc::Size::new(32, 32);
        assert!(!PreparedRenderer::supports_plain_gaussian_budgeted(&doc));
        let original = super::super::plain_point::tests::doc();
        assert!(PreparedRenderer::supports_plain_point_budgeted(&original));
        assert!(!PreparedRenderer::supports_plain_gaussian_budgeted(&original));
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
            assert!(PreparedRenderer::new_plain_gaussian_budgeted(&doc, &ctl, &mut budget, source).is_err());
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
    fn actual_cache_denial_cancel_work_error_refund_only_transient_and_output() {
        for mode in 0..3 {
            let doc = doc();
            let before = doc.clone();
            let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: if mode == 0 { 32 << 20 } else { 48 << 20 }, work: 0, work_limit: u64::MAX };
            let source = budget.reserve_prepared_bytes(123).unwrap();
            let prepared = PreparedRenderer::new_plain_gaussian_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
            let retained = budget.live.get();
            let mut outputs: Vec<(u64, Lease)> = vec![];
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let cancel = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
            let ctl = Interrupt::cancel_only(&cancel);
            let mut cache_seen = false;
            let result = prepared.renderer().render_reduced_with_interrupt(
                96,
                80,
                None,
                &mut |event| {
                    match event {
                        photocraft_cms::WideWork::ReserveBytes(0) | photocraft_cms::WideWork::ReleaseBytes(0) => {}
                        photocraft_cms::WideWork::ReserveBytes(n) => {
                            if n == 33554432 {
                                cache_seen = true;
                            }
                            outputs.push((n, budget.reserve_prepared_bytes(n)?));
                            if mode == 1 && n == 33554432 {
                                cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        photocraft_cms::WideWork::ReleaseBytes(n) => {
                            let index = outputs.iter().position(|(size, _)| *size == n).ok_or(CompositionError::Invalid("test release"))?;
                            outputs.remove(index);
                        }
                        photocraft_cms::WideWork::SampleUnits(_) => {
                            if mode == 2 && cache_seen {
                                return Err(photocraft_raster::RasterInputError::LimitExceeded.into());
                            }
                        }
                    }
                    Ok(())
                },
                &ctl,
            );
            assert!(result.is_err());
            assert!(cache_seen);
            assert!(outputs.is_empty());
            assert_eq!(budget.live.get(), retained);
            assert_eq!(doc, before);
            drop(prepared);
            assert_eq!(budget.live.get(), 0);
        }
    }
}
