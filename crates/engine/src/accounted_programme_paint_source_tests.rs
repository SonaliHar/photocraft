//! Deferred existing-Source and paid-CURRENT programme custody; no new grant.
use crate::Session;
#[path = "accounted_programme_paint_test_support.rs"]
mod fixture;
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl},
};
#[test]
#[ignore = "UNRUN/uncompiled: actual nine stored-programme classes from existing qualified Sources plus explicitly paid maskedHierarchy CURRENT preserve complete programmes/native/profile/masks on typed adoption+native continuation"]
fn all_nine_actual_classes_keep_original_programmes_and_retention() {
    for &case in fixture::cases() {
        let input = fixture::load(case);
        let q = input.owner().quota().clone();
        assert_eq!(
            case.evidence(),
            if case == fixture::Case::MaskedHierarchyCurrent { fixture::Evidence::PaidSyntheticCurrent } else { fixture::Evidence::GenuineSource }
        );
        let witness = fixture::Witness::capture(input.owner(), &fixture::ctl());
        let target = fixture::target(input.owner().document());
        let original = fixture::raw(input.owner().document(), target);
        let patch = fixture::patch(input.owner().document(), target);
        assert_eq!(patch["layer"].as_u64(), Some(target.0));
        let mut s = Session::new();
        fixture::adopt(&mut s, case, &input);
        let current = s.active().unwrap().doc.accounted().unwrap();
        witness.preserved(current.document(), target, &fixture::ctl());
        assert_eq!(fixture::raw(current.document(), target), original);
        let archive = fixture::archive(current);
        let mut reopened = Session::new();
        fixture::reopen(&mut reopened, case, archive);
        let after = reopened.active().unwrap().doc.accounted().unwrap();
        witness.preserved(after.document(), target, &fixture::ctl());
        assert_eq!(fixture::raw(after.document(), target), original);
        drop(input);
        assert!(q.usage().unwrap().resident > 0);
        drop(s);
        drop(reopened);
        assert!(q.usage().unwrap().resident > 0);
        drop(witness);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: unchanged portable programme native writer preserves original Work/cancel and same ledger custody; no source/native semantic fallback"]
fn programme_native_controls_refuse_without_changing_real_owner() {
    for case in [fixture::Case::Point, fixture::Case::MaskedFilteredFlat, fixture::Case::Hierarchy, fixture::Case::MaskedHierarchyCurrent] {
        let input = fixture::load(case);
        let owner = input.owner();
        let witness = fixture::Witness::capture(owner, &fixture::ctl());
        let id = fixture::target(owner.document());
        let raw = fixture::raw(owner.document(), id);
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(
            photocraft_io::accounted_current_native::save_accounted_current_archive(
                owner,
                std::io::Cursor::new(Vec::new()),
                Default::default(),
                owner.quota(),
                &deny
            )
            .is_err()
        );
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let cancel = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(
            photocraft_io::accounted_current_native::save_accounted_current_archive(
                owner,
                std::io::Cursor::new(Vec::new()),
                Default::default(),
                owner.quota(),
                &cancel
            )
            .is_err()
        );
        witness.preserved(owner.document(), id, &fixture::ctl());
        assert_eq!(fixture::raw(owner.document(), id), raw);
    }
}
