//! UNRUN strict caller-basis controls. Synthetic fixture allocations are harness
//! memory; this does not qualify producer profile equivalence or display color.
use super::*;

#[test]
#[ignore = "UNRUN actual Gray Box requires SGray caller basis and separately bound current marker"]
fn gray_factory_requires_exact_mode_basis_and_separately_validates_current_marker() {
    let s = Rect::new(-3, 5, 2, 8);
    for sample in [SampleType::F32, SampleType::F64] {
        let (doc, q, guard, _, counts) = gray_fixture(sample, s, 4, true);
        assert_eq!(doc.mode, ColorMode::Grayscale);
        let owned = owner(doc, q, guard, false);
        let before = owned.document().clone();
        drop(PreparedBackedDocument::new_accounted(&owned, photocraft_cms::Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).unwrap());
        let current = photocraft_cms::xcf_color::effective_profile_from_definition(
            photocraft_cms::Builtin::SGray.profile(),
            photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        )
        .unwrap();
        // The nonlinear effective definition preserves the builtin bytes, so
        // that independent Profile value satisfies the same exact basis check.
        assert_eq!(current.encoded_bytes(), photocraft_cms::Builtin::SGray.profile().encoded_bytes());
        drop(PreparedBackedDocument::new_accounted(&owned, &current, NormalBandLimits::default(), &ctl()).unwrap());
        let wrong_gray_basis =
            photocraft_cms::xcf_color::effective_profile_from_definition(photocraft_cms::Builtin::SGray.profile(), photocraft_cms::xcf_color::Encoding::Linear)
                .unwrap();
        assert_ne!(wrong_gray_basis.encoded_bytes(), photocraft_cms::Builtin::SGray.profile().encoded_bytes());
        let baseline = owned.quota().usage().unwrap();
        let reads = counts.reads.load(Ordering::Relaxed);
        for supplied in [photocraft_cms::Builtin::Srgb.profile(), &wrong_gray_basis] {
            assert!(PreparedBackedDocument::new_accounted(&owned, supplied, NormalBandLimits::default(), &ctl()).is_err());
            assert_eq!(counts.reads.load(Ordering::Relaxed), reads);
            assert_eq!(owned.quota().usage().unwrap(), baseline);
            assert_eq!(owned.document(), &before);
        }

        let (mut doc, q, guard, _, _) = gray_fixture(sample, s, 4, false);
        doc.layers[0].visible = false;
        let layer = doc.layers[0].id;
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().visible = false;
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(b"wrong current encoded identity");
        let invalid = owner(doc, q, guard, false);
        let before = invalid.document().clone();
        let baseline = invalid.quota().usage().unwrap();
        assert!(PreparedBackedDocument::new_accounted(&invalid, photocraft_cms::Builtin::SGray.profile(), NormalBandLimits::default(), &ctl(),).is_err());
        assert_eq!(invalid.document(), &before);
        assert_eq!(invalid.quota().usage().unwrap(), baseline);
    }
}
