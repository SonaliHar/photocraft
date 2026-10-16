//! Deferred mixed HSL native continuation under its declared checked domain.
use super::*;
#[path = "accounted_filtered_flat_hsl_test_support.rs"]
mod fixture;

#[test]
#[ignore = "UNRUN actual declared CURRENT HSL mixed/sole unmarked epoch routing, generic/Point handoff denial and marked continuation. No GIMP Source/Float-alpha/numerical acceptance."]
fn hsl_mixed_native_keeps_its_closed_epoch_and_class() {
    for sole in [false, true] {
        let owner = if sole { fixture::sole() } else { fixture::build() };
        assert!(is_current_filtered_flat_candidate(owner.document(), &fixture::ctl()).unwrap());
        let archive = fixture::unmarked_archive(&owner);
        assert_eq!(archive.current_storage_hint(), None);
        assert!(is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut wrong = Session::new();
        assert!(wrong.add_owned_native_archive(archive, AccountedNativeOpenPolicy::KeepCurrentInterpretation, "hsl.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.documents().is_empty());
        assert!(wrong.add_owned_native_point_archive(fixture::unmarked_archive(&owner), "hsl.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.documents().is_empty());
        let mut right = Session::new();
        if sole {
            assert!(right.add_owned_native_filtered_flat_archive(fixture::unmarked_archive(&owner), "hsl.pcraft", None, &fixture::ctl()).is_err());
            assert!(right.documents().is_empty());
            right.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "hsl.pcraft", None, &fixture::ctl()).unwrap();
        } else {
            right.add_owned_native_filtered_flat_archive(fixture::unmarked_archive(&owner), "hsl.pcraft", None, &fixture::ctl()).unwrap();
        }
        let current = &right.active().unwrap().doc;
        fixture::storage(owner.document(), current);
        assert!(current.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        assert_eq!(fixture::archive(current.accounted().unwrap()).current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::FilteredFlatPoint));
    }
}
