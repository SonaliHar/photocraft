//! Closed caller-budgeted singleton Box Blur preparation.
use super::{PreparationBudget, plain_point};
use crate::gimp_composite::Result;
use photocraft_doc::Document;
use photocraft_raster::Interrupt;
pub(crate) fn supports(doc: &Document) -> bool {
    plain_point::supports_singleton(doc, plain_point::Singleton::Box)
}
pub(crate) fn preparation_bytes(icc_bytes: usize) -> Result<u64> {
    plain_point::preparation_bytes(icc_bytes)
}
pub(crate) fn validate(doc: &Document, ctl: &Interrupt<'_>, budget: &mut impl PreparationBudget) -> Result<()> {
    plain_point::validate_singleton(doc, ctl, budget, plain_point::Singleton::Box)
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
            self.work += n;
            Ok(())
        }
    }
    fn doc() -> photocraft_doc::Document {
        let mut doc = super::super::plain_point::tests::doc();
        doc.source_color_context = Some(photocraft_doc::SourceColorContext { original_icc: None, source_precision: 150 });
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.name = "gegl:box-blur".into();
        node.operation.stored_version = "0:0".into();
        node.operation.schema_epoch = "xcf22-rgb-box-replace-v1".into();
        node.composition.as_mut().unwrap().compatibility_epoch = "xcf22-rgb-box-replace-v1".into();
        node.arguments = vec![FilterArgument { name: "radius".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 }) }];
        doc
    }
    #[test]
    fn singleton_box_preparation_charges_real_cache_and_retains_source() {
        let doc = doc();
        let before = doc.clone();
        let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 48 << 20, work: 0 };
        let source = budget.reserve_prepared_bytes(123).unwrap();
        let prepared = PreparedRenderer::new_plain_box_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
        let mut events = vec![];
        let output = prepared
            .renderer()
            .render_reduced_with_interrupt(
                32,
                32,
                None,
                &mut |e| {
                    events.push(e);
                    Ok(())
                },
                &Interrupt::NONE,
            )
            .unwrap();
        assert_eq!(output.px.len(), 1024);
        assert_eq!(events.iter().filter(|e| matches!(e, photocraft_cms::WideWork::ReserveBytes(33554432))).count(), 1);
        assert_eq!(events.iter().filter(|e| matches!(e, photocraft_cms::WideWork::ReleaseBytes(33554432))).count(), 1);
        assert_eq!(doc, before);
        drop(prepared);
        assert_eq!(budget.live.get(), 0);
    }
    #[test]
    fn scoped_box_invalid_samples_arguments_bindings_refund_preparation() {
        for kind in 0..7 {
            let mut doc = doc();
            match kind {
                0 => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Signed { width: 32, value: 6 })
                }
                1 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(4096, -4096, &[1.0]).unwrap(),
                2 => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(-4096, 4096, &[0.1, 0.2, 0.3, 1.0]).unwrap(),
                3 => doc.icc_profile = None,
                4 => doc.icc_profile = Some(std::sync::Arc::new(vec![0; 200])),
                5 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_clipping = Some(1),
                _ => {
                    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value =
                        FilterValue::Number(ExactNumber::Float32Bits(4_f32.to_bits()))
                }
            }
            assert!(PreparedRenderer::supports_plain_box_budgeted(&doc));
            let before = doc.clone();
            let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: 48 << 20, work: 0 };
            let unrelated = budget.reserve_prepared_bytes(17).unwrap();
            let source = budget.reserve_prepared_bytes(123).unwrap();
            assert!(PreparedRenderer::new_plain_box_budgeted(&doc, &Interrupt::NONE, &mut budget, source).is_err());
            assert_eq!(doc, before);
            assert_eq!(budget.live.get(), 17);
            drop(unrelated);
            assert_eq!(budget.live.get(), 0);
        }
    }
    #[test]
    fn actual_cache_denial_cancel_work_error_refund_only_transient_and_output() {
        for mode in 0..3 {
            let doc = doc();
            let before = doc.clone();
            let mut budget = Budget { live: Rc::new(Cell::new(0)), limit: if mode == 0 { 32 << 20 } else { 48 << 20 }, work: 0 };
            let source = budget.reserve_prepared_bytes(123).unwrap();
            let prepared = PreparedRenderer::new_plain_box_budgeted(&doc, &Interrupt::NONE, &mut budget, source).unwrap();
            let retained = budget.live.get();
            let mut outputs: Vec<(u64, Lease)> = vec![];
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let cancel = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
            let ctl = Interrupt::cancel_only(&cancel);
            let mut cache_seen = false;
            let result = prepared.renderer().render_reduced_with_interrupt(
                32,
                32,
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
