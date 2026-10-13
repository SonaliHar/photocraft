//! Controlled storage for WorkingSpace; CMS preparation calls are fenced,
//! not claimed to be interruptible inside their existing implementations.
use super::{CompositionError, Result};
use photocraft_cms::{Profile, curve::Curve, xcf_color::Encoding};
use photocraft_raster::{Interrupt, RasterInputError};

pub(super) fn checkpoint(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}

pub(super) fn prepare<T>(control: &Interrupt<'_>, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    checkpoint(control)?;
    let value = operation();
    checkpoint(control)?;
    value
}

// Identical validation/override policy to effective_profile_from_definition,
// without cloning a profile which the separable-transfer consumer never uses.
pub(super) fn validate_interpretation(profile: &Profile, encoding: Encoding, control: &Interrupt<'_>) -> Result<()> {
    prepare(control, || Ok(photocraft_cms::xcf_color::validate_profile(profile)?))?;
    if (profile.a2b.iter().any(Option::is_some) || profile.b2a.iter().any(Option::is_some)) && encoding != Encoding::NonlinearProfile {
        return Err(photocraft_cms::CmsError::Unsupported(
            "XCF ICC LUT transfer override has no separable tone curves; retain profile-defined nonlinear encoding".into(),
        ).into());
    }
    Ok(())
}

fn copy_samples<T: Copy>(values: &[T], control: &Interrupt<'_>) -> Result<Vec<T>> {
    checkpoint(control)?;
    // The CMS validator permits at most this many stored TRC samples.
    if values.len() > 1 << 20 {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| RasterInputError::AllocationFailed)?;
    let actual = output.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)?;
    if u64::try_from(actual).map_err(|_| RasterInputError::LimitExceeded)? > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded.into());
    }
    for chunk in values.chunks(1024) {
        checkpoint(control)?;
        let end = output.len().checked_add(chunk.len()).ok_or(RasterInputError::LimitExceeded)?;
        if end > output.capacity() { return Err(RasterInputError::LimitExceeded.into()); }
        output.extend_from_slice(chunk);
    }
    checkpoint(control)?;
    Ok(output)
}

pub(super) fn transfer_curves(profile: &Profile, control: &Interrupt<'_>) -> Result<[Option<Curve>; 3]> {
    let mut output = [None, None, None];
    for (channel, slot) in output.iter_mut().take(profile.channels()).enumerate() {
        checkpoint(control)?;
        let curve = match profile.color_space {
            photocraft_cms::ColorSpace::Rgb => profile.trc.as_ref().and_then(|curves| curves.get(channel)),
            photocraft_cms::ColorSpace::Gray => profile.gray_trc.as_ref(),
            _ => None,
        }.ok_or(CompositionError::Invalid("missing prepared transfer curve"))?;
        *slot = Some(match curve {
            Curve::Table(values) => Curve::Table(copy_samples(values, control)?),
            Curve::Table64(values) => Curve::Table64(copy_samples(values, control)?),
            _ => curve.clone(),
        });
    }
    checkpoint(control)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_transfer_tables_keep_exact_sample_bits() {
        let mut profile = photocraft_cms::Builtin::Srgb.profile().clone();
        let values = vec![0.0, f64::from_bits(0x3fe0_0000_0000_0001), 1.0];
        profile.trc = Some([Curve::Table64(values.clone()), Curve::Identity, Curve::Gamma(2.2)]);
        let curves = transfer_curves(&profile, &Interrupt::NONE).unwrap();
        let Curve::Table64(copied) = curves[0].as_ref().unwrap() else { panic!("exact table expected") };
        assert_eq!(copied.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), values.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        assert_eq!(curves[1], Some(Curve::Identity));
        assert_eq!(curves[2], Some(Curve::Gamma(2.2)));
    }

    #[test]
    fn cancelled_preparation_does_not_run_operation() {
        let cancel = || true;
        let called = std::cell::Cell::new(false);
        let result = prepare(&Interrupt::cancel_only(&cancel), || { called.set(true); Ok(()) });
        assert!(matches!(result, Err(CompositionError::Raster(RasterInputError::Cancelled))));
        assert!(!called.get());
    }

    #[test]
    fn table_copy_polls_during_copy_and_keeps_input() {
        let values = vec![0.25_f64; 8192];
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4;
        let result = copy_samples(&values, &Interrupt::cancel_only(&cancel));
        assert!(matches!(result, Err(CompositionError::Raster(RasterInputError::Cancelled))));
        assert_eq!(values, vec![0.25; 8192]);
    }
}
