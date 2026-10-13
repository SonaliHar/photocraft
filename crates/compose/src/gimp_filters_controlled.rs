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
