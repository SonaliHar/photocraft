//! UNRUN display and actual runtime-class dispatch regression drafts.
use super::*;
use crate::backed_threshold_context::tests::{adopt, ctl, fixture};
use photocraft_cms::Builtin;
use photocraft_doc::{SampleEncoding, WorkingProfileInterpretation};
#[test]
fn native_and_display_outputs_are_separate_and_dispatch_matches() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let (doc, q, guard, _, p, _) = fixture(ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
        let owner = adopt(doc, guard, q);
        let display = PreparedBackedThresholdDocument::new(&owner, p, NormalBandLimits::default(), &ctl()).unwrap();
        let dispatched = crate::backed_document::PreparedBackedDocument::new_accounted(&owner, p, NormalBandLimits::default(), &ctl()).unwrap();
        let bounds = owner.document().bounds();
        let band = display.render_band(bounds, &ctl()).unwrap();
        let routed = dispatched.render_band(bounds, &ctl()).unwrap();
        assert_eq!(band.pixels(), routed.pixels());
        for (i, pixel) in band.pixels().iter().enumerate() {
            let rgb = if i % 3 == 0 || i % 4 == 0 { 0. } else { 1. };
            assert_eq!(&pixel[..3], &[rgb; 3]);
            assert!(
                (pixel[3]
                    - if i % 4 == 0 {
                        0.0
                    } else if i % 2 == 0 {
                        0.25
                    } else {
                        0.75
                    })
                .abs()
                    < 0.00002
            );
        }
        assert!(display.render_band(Rect::new(-1, 0, 8, 6), &ctl()).is_err());
    }
}
#[test]
fn explicit_current_profile_and_actual_storage_class_remain_mandatory() {
    let (mut doc, q, guard, _, p, _) = fixture(ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let owner = adopt(doc, guard, q);
    assert!(PreparedBackedThresholdDocument::new(&owner, p, NormalBandLimits::default(), &ctl()).is_err());
    assert!(crate::backed_document::PreparedBackedDocument::new_accounted(&owner, Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).is_err());
    assert!(crate::backed_document::PreparedBackedDocument::new(owner.document(), p, NormalBandLimits::default(), &ctl()).is_err());
}
