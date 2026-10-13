//! Existing admitted scalar evaluator work bridge. No new evaluator/admission.
//! Pixel results remain private until all inner work and final polling succeed.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_cms::WideWork;
use photocraft_raster::{Interrupt, RasterInputError};
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};
/// The caller's WideWork callback owns actual cancellation/deadline and shared
/// accounting. Native inner work forwards to it; rejection latches the supplied
/// Interrupt proxy and the exact original error. No callback failure is swallowed.
/// No-work fixed scalar callbacks still have entry/exit cancellation boundaries.
/// This adds no heap allocation and never publishes a partial output pixel.
pub(crate) fn run<T>(
    poll: &mut impl FnMut(WideWork) -> Result<()>,
    evaluate: impl FnOnce(&Interrupt<'_>, &dyn Fn(u64) -> std::result::Result<(), RasterInputError>) -> Result<T>,
) -> Result<T> {
    poll(WideWork::SampleUnits(0))?;
    let callback = RefCell::new(poll);
    let failure = RefCell::new(None);
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let control = Interrupt::cancel_only(&cancel);
    let work = |units: u64| -> std::result::Result<(), RasterInputError> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(RasterInputError::Cancelled);
        }
        let mut callback = callback.try_borrow_mut().map_err(|_| RasterInputError::LimitExceeded)?;
        match callback(WideWork::SampleUnits(units)) {
            Ok(()) => Ok(()),
            Err(error) => {
                let native = if matches!(&error, CompositionError::Raster(RasterInputError::Cancelled)) {
                    RasterInputError::Cancelled
                } else {
                    RasterInputError::LimitExceeded
                };
                let mut failure = failure.try_borrow_mut().map_err(|_| RasterInputError::LimitExceeded)?;
                *failure = Some(error);
                cancelled.store(true, Ordering::Relaxed);
                Err(native)
            }
        }
    };
    let result = evaluate(&control, &work);
    if let Some(error) = failure.into_inner() {
        return Err(error);
    }
    let output = result?;
    callback.into_inner()(WideWork::SampleUnits(0))?;
    Ok(output)
}
pub(crate) fn finish_pixel(pixel: [f64; 4], poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
    poll(WideWork::SampleUnits(0))?;
    Ok(pixel)
}


pub(super) fn checkpoint(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
pub(super) fn preparation<T>(control: &Interrupt<'_>, operation: impl FnOnce(&Interrupt<'_>) -> Result<T>) -> Result<T> {
    let stopped = AtomicBool::new(false);
    let cancel = || {
        if control.cancelled() { stopped.store(true, Ordering::Relaxed); }
        stopped.load(Ordering::Relaxed)
    };
    let latched = Interrupt::cancel_only(&cancel);
    checkpoint(&latched)?;
    let result = operation(&latched);
    checkpoint(&latched)?;
    result
}
/// Stop borrowed envelope scans as soon as cancellation is observed. Every
/// preparation entry is fenced, so a stopped scan cannot publish a plan or
/// substitute a policy refusal for the caller's cancellation.
pub(super) fn iter<'a, T>(values: &'a [T], control: &'a Interrupt<'_>) -> impl Iterator<Item = &'a T> {
    let mut values = values.iter();
    std::iter::from_fn(move || if control.cancelled() { None } else { values.next() })
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    #[test]
    fn cancelled_point_preparation_refuses_before_empty_stack_publication() {
        let cancel = || true;
        let result = crate::gimp_filters::PointStack::from_nodes_with_interrupt(
            &[], photocraft_cms::Builtin::Srgb.profile(), photocraft_doc::SampleEncoding::NonlinearProfile, 22,
            &Interrupt::cancel_only(&cancel),
        );
        assert!(matches!(result, Err(CompositionError::Raster(RasterInputError::Cancelled))));
    }
    #[test]
    fn envelope_scan_stops_and_fence_preserves_cancellation() {
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
        let control = Interrupt::cancel_only(&cancel);
        let count = std::cell::Cell::new(0);
        let result = preparation(&control, |control| {
            for _ in iter(&[0u8; 1024], control) { count.set(count.get() + 1); }
            Ok(())
        });
        assert!(count.get() < 1024);
        assert!(matches!(result, Err(CompositionError::Raster(RasterInputError::Cancelled))));
    }
}
