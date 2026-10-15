//! UNRUN CURRENT backdrop activity; generated owned data, no source enrollment.
use super::*;

#[test]
fn typed_backdrop_activity_matches_existing_isolated_normal_math_and_bare_refuses() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for (visible, opacity) in [(false, 1.0), (true, 0.375), (true, 0.0)] {
                let owner = fixture_with_backdrop(mode, sample, 0, 0, visible, opacity);
                let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).unwrap();
                let typed = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).unwrap();
                assert!(
                    PreparedBackedDocument::new_accounted(&owner, binding.profile(), Default::default(), &ctl()).is_err(),
                    "old bare route remains source-like"
                );
                let band = typed.render_band(owner.document().bounds(), &ctl()).unwrap();
                let working = WorkingSpace::new(binding.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
                let rect = owner.document().bounds();
                for (y, row) in band.pixels().chunks_exact(rect.width() as usize).enumerate() {
                    for (x, pixel) in row.iter().enumerate() {
                        assert_eq!(*pixel, reference_layers(owner.document(), &owner.document().layers, &working, rect.x0 + x as i32, rect.y0 + y as i32));
                    }
                }
            }
        }
    }
}

#[test]
fn hidden_or_zero_backdrop_still_validates_last_native_word() {
    for (visible, opacity) in [(false, 1.0), (true, 0.0)] {
        let owner = fixture_with_backdrop(ColorMode::Rgb, SampleType::F64, 0, 3, visible, opacity);
        let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).unwrap();
        assert!(PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).is_err());
    }
    for opacity in [f32::NAN, f32::INFINITY, -0.25, 1.25] {
        let owner = fixture_with_backdrop(ColorMode::Rgb, SampleType::F64, 0, 0, false, opacity);
        assert!(PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).is_err());
    }
}

#[test]
fn hidden_group_and_zero_backdrop_produce_transparent_current_band_with_paid_control() {
    let owner = fixture_with_backdrop(ColorMode::Rgb, SampleType::F64, 2, 0, true, 0.0);
    let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).unwrap();
    let typed = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).unwrap();
    let before = owner.quota().usage().unwrap();
    let band = typed.render_band(owner.document().bounds(), &ctl()).unwrap();
    assert!(band.pixels().iter().all(|p| *p == [0.0; 4]));
    assert!(owner.quota().usage().unwrap().resident > before.resident);
    drop(band);
    assert_eq!(owner.quota().usage().unwrap(), before);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(typed.render_band(owner.document().bounds(), &denied), Err(Error::Backing(BackingError::Work))));
    assert_eq!(owner.quota().usage().unwrap(), before);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(typed.render_band(owner.document().bounds(), &control), Err(Error::Backing(BackingError::Cancelled))));
    assert_eq!(owner.quota().usage().unwrap(), before);
}
