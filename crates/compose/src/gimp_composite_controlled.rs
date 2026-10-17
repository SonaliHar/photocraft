//! Controlled WorkingSpace storage and selected CMS pipeline preparation.
//! Builtin profile initialization remains an outer-polled provider boundary.
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

pub(super) fn cms_error(error: photocraft_cms::control::Error) -> CompositionError {
    match error {
        photocraft_cms::control::Error::Cancelled => RasterInputError::Cancelled.into(),
        photocraft_cms::control::Error::Allocation => RasterInputError::AllocationFailed.into(),
        photocraft_cms::control::Error::Limit(_) => RasterInputError::LimitExceeded.into(),
        photocraft_cms::control::Error::Cms(error) => error.into(),
    }
}

/// Both constructors remain alive together. Conservatively retain the first
/// complete phase envelope while admitting the reverse constructor.
pub(super) fn transform_pair(source: &Profile, target: &Profile, control: &Interrupt<'_>) -> Result<(photocraft_cms::Transform, photocraft_cms::Transform)> {
    transform_pair_with_limits(source, target, photocraft_cms::control::Limits::default(), control)
}
fn transform_pair_with_limits(
    source: &Profile,
    target: &Profile,
    limits: photocraft_cms::control::Limits,
    control: &Interrupt<'_>,
) -> Result<(photocraft_cms::Transform, photocraft_cms::Transform)> {
    use photocraft_cms::{Intent, Transform, control::Limits, transform::TransformOptions};
    checkpoint(control)?;
    let cancelled = || control.cancelled();
    let options = TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, ..Default::default() };
    let forward = Transform::requirements_with_control(source, target, options, limits, &cancelled).map_err(cms_error)?;
    let reverse = Transform::requirements_with_control(target, source, options, limits, &cancelled).map_err(cms_error)?;
    let peak = forward.peak_resident_bytes.checked_add(reverse.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?;
    let work = forward.work_units.checked_add(reverse.work_units).and_then(|n| n.checked_mul(3)).ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let first_limits = Limits {
        resident_bytes: limits.resident_bytes.checked_sub(reverse.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?,
        work_units: limits
            .work_units
            .checked_sub(reverse.work_units.checked_mul(3).ok_or(RasterInputError::LimitExceeded)?)
            .ok_or(RasterInputError::LimitExceeded)?,
    };
    let second_limits = Limits {
        resident_bytes: limits.resident_bytes.checked_sub(forward.peak_resident_bytes).ok_or(RasterInputError::LimitExceeded)?,
        work_units: limits
            .work_units
            .checked_sub(forward.work_units.checked_mul(3).ok_or(RasterInputError::LimitExceeded)?)
            .ok_or(RasterInputError::LimitExceeded)?,
    };
    let forward = Transform::with_options_with_control(source, target, options, first_limits, &cancelled).map_err(cms_error)?;
    let reverse = Transform::with_options_with_control(target, source, options, second_limits, &cancelled).map_err(cms_error)?;
    checkpoint(control)?;
    Ok((forward, reverse))
}

#[cfg(test)]
mod transform_pair_tests {
    use super::*;
    use photocraft_cms::{Builtin, Intent, Transform};
    #[test]
    fn paired_constructor_retains_relative_no_bpc_forward_reverse_words() {
        for builtin in [Builtin::Srgb, Builtin::GrayGamma22] {
            let source = builtin.profile();
            let target = Builtin::LabD50.profile();
            let (forward, reverse) = transform_pair(source, target, &Interrupt::NONE).unwrap();
            let old_forward = Transform::new(source, target, Intent::RelativeColorimetric, false).unwrap();
            let old_reverse = Transform::new(target, source, Intent::RelativeColorimetric, false).unwrap();
            for values in [[0., 0., 0.], [0.25, 0.5, 0.75], [1., 1., 1.]] {
                let input = &values[..source.channels()];
                let mut actual = [0.; 3];
                let mut expected = [0.; 3];
                forward.try_eval_f64(input, &mut actual).unwrap();
                old_forward.try_eval_f64(input, &mut expected).unwrap();
                assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
                let mut back = [0.; 3];
                let mut old_back = [0.; 3];
                reverse.try_eval_f64(&actual, &mut back[..source.channels()]).unwrap();
                old_reverse.try_eval_f64(&expected, &mut old_back[..source.channels()]).unwrap();
                assert_eq!(back.map(f64::to_bits), old_back.map(f64::to_bits));
            }
        }
    }
    #[test]
    fn cancelled_pair_never_returns_partial_working_space() {
        let stop = || true;
        assert!(matches!(
            transform_pair(Builtin::Srgb.profile(), Builtin::LabD50.profile(), &Interrupt::cancel_only(&stop)),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
    }
}

// Identical validation/override policy to effective_profile_from_definition,
// without cloning a profile which the separable-transfer consumer never uses.
pub(super) fn validate_interpretation(profile: &Profile, encoding: Encoding, control: &Interrupt<'_>) -> Result<()> {
    checkpoint(control)?;
    photocraft_cms::xcf_color::validate_profile_with_control(profile, photocraft_cms::control::Limits::default(), &|| control.cancelled())
        .map_err(cms_error)?;
    if (profile.a2b.iter().any(Option::is_some) || profile.b2a.iter().any(Option::is_some)) && encoding != Encoding::NonlinearProfile {
        return Err(photocraft_cms::CmsError::Unsupported(
            "XCF ICC LUT transfer override has no separable tone curves; retain profile-defined nonlinear encoding".into(),
        )
        .into());
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
        if end > output.capacity() {
            return Err(RasterInputError::LimitExceeded.into());
        }
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
        }
        .ok_or(CompositionError::Invalid("missing prepared transfer curve"))?;
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
        let result = prepare(&Interrupt::cancel_only(&cancel), || {
            called.set(true);
            Ok(())
        });
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

/// Physical-Lab pipelines and both normalized ICC transforms coexist. Reserve
/// their combined envelopes before any side copy; no numerical recipe changes.
type LabBundle = (photocraft_cms::Transform, photocraft_cms::Transform, Option<(photocraft_cms::pipeline::Pipeline, photocraft_cms::pipeline::Pipeline)>);

pub(super) fn lab_bundle(source: &Profile, target: &Profile, physical: bool, control: &Interrupt<'_>) -> Result<LabBundle> {
    use photocraft_cms::{
        Intent,
        control::Limits,
        pipeline::{Pipeline, Stage},
        profile::Pcs,
    };
    let limits = Limits::default();
    let cancelled = || control.cancelled();
    if !physical {
        let (a, b) = transform_pair(source, target, control)?;
        return Ok((a, b, None));
    }
    let a = source.side_requirements_with_control(true, Intent::RelativeColorimetric, limits, &cancelled).map_err(cms_error)?;
    let b = source.side_requirements_with_control(false, Intent::RelativeColorimetric, limits, &cancelled).map_err(cms_error)?;
    // Forward append and reverse prepend may temporarily retain both old/new
    // stage inventories. Each side reserves a bounded extra pair of inventories.
    let extra = 8usize.checked_mul(std::mem::size_of::<Stage>()).ok_or(RasterInputError::LimitExceeded)?;
    let bytes = a.peak_resident_bytes.checked_add(b.peak_resident_bytes).and_then(|n| n.checked_add(extra)).ok_or(RasterInputError::LimitExceeded)?;
    let work = a.work_units.checked_add(b.work_units).and_then(|n| n.checked_mul(3)).and_then(|n| n.checked_add(128)).ok_or(RasterInputError::LimitExceeded)?;
    let pair_limits = Limits {
        resident_bytes: limits.resident_bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?,
        work_units: limits.work_units.checked_sub(work).ok_or(RasterInputError::LimitExceeded)?,
    };
    // The pair constructor's borrowed admission completes before side allocation.
    let options = photocraft_cms::transform::TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, ..Default::default() };
    let f = photocraft_cms::Transform::requirements_with_control(source, target, options, pair_limits, &cancelled).map_err(cms_error)?;
    let r = photocraft_cms::Transform::requirements_with_control(target, source, options, pair_limits, &cancelled).map_err(cms_error)?;
    if f.peak_resident_bytes.checked_add(r.peak_resident_bytes).is_none_or(|n| n > pair_limits.resident_bytes)
        || f.work_units.checked_add(r.work_units).and_then(|n| n.checked_mul(3)).is_none_or(|n| n > pair_limits.work_units)
    {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let side_limits = Limits { resident_bytes: bytes, work_units: work };
    let (mut forward, fp) = source.device_to_pcs_with_control(Intent::RelativeColorimetric, side_limits, &cancelled).map_err(cms_error)?;
    let (reverse, rp) = source.pcs_to_device_with_control(Intent::RelativeColorimetric, side_limits, &cancelled).map_err(cms_error)?;
    let physical = if fp != Pcs::Xyz || rp != Pcs::Xyz {
        None
    } else {
        forward.try_reserve_exact(1).map_err(|_| RasterInputError::AllocationFailed)?;
        forward.push(Stage::XyzToLab);
        let count = reverse.len().checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
        let mut backward = Vec::new();
        backward.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
        let actual = forward
            .capacity()
            .checked_add(backward.capacity())
            .and_then(|n| n.checked_mul(std::mem::size_of::<Stage>()))
            .ok_or(RasterInputError::LimitExceeded)?;
        if actual > bytes {
            return Err(RasterInputError::LimitExceeded.into());
        }
        backward.push(Stage::LabToXyz);
        for stage in reverse {
            checkpoint(control)?;
            backward.push(stage);
        }
        Some((Pipeline::new(3, forward), Pipeline::new(3, backward)))
    };
    let (forward, reverse) = transform_pair_with_limits(source, target, pair_limits, control)?;
    checkpoint(control)?;
    Ok((forward, reverse, physical))
}

#[cfg(test)]
mod physical_side_tests {
    use super::*;
    #[test]
    fn physical_reverse_keeps_signed_unclipped_f64_words() {
        use photocraft_cms::{
            Builtin, Intent,
            pipeline::{Pipeline, Stage},
        };
        let source = Builtin::Srgb.profile();
        let target = Builtin::LabD50.profile();
        let (_, _, physical) = lab_bundle(source, target, true, &Interrupt::NONE).unwrap();
        let (_, actual) = physical.unwrap();
        let (reverse, _) = source.pcs_to_device(Intent::RelativeColorimetric).unwrap();
        let mut stages = vec![Stage::LabToXyz];
        stages.extend(reverse);
        let old = Pipeline::new(3, stages);
        for input in [[53.0, 200.0, -300.0], [53.0, 30.0, -20.0], [f64::from_bits(53.0f64.to_bits() + 1), 30.0, -20.0]] {
            let mut a = [0.0; 3];
            let mut b = [0.0; 3];
            actual.try_eval_f64(&input, &mut a).unwrap();
            old.try_eval_f64(&input, &mut b).unwrap();
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    #[test]
    fn physical_bundle_cancellation_is_not_a_normalized_lab_fallback() {
        let stop = || true;
        assert!(matches!(
            lab_bundle(photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Builtin::LabD50.profile(), true, &Interrupt::cancel_only(&stop)),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
    }
}
