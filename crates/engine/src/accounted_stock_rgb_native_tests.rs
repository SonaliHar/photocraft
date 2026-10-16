//! Deferred native routing for declared three-kind CURRENT adapters.
use super::*;
#[path = "accounted_filtered_flat_stock_rgb_test_support.rs"]
mod fixture;

#[test]
#[ignore = "UNRUN declared CURRENT mixed and sole-three-kind owners, unmarked exact epoch hints, forced native mixed routing and wrong generic/Point denial. No producer Source or numerical oracle."]
fn stock_rgb_native_epochs_keep_the_actual_mixed_class() {
    use fixture::Kind::{AlienRgb, Sepia, Temperature};
    for kind in [None, Some(Temperature), Some(Sepia), Some(AlienRgb)] {
        let owner = match kind {
            Some(kind) => fixture::sole(kind),
            None => fixture::build(),
        };
        assert!(is_current_filtered_flat_candidate(owner.document(), &fixture::ctl()).unwrap());
        let archive = fixture::unmarked_archive(&owner);
        assert_eq!(archive.current_storage_hint(), None);
        assert!(is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut wrong = Session::new();
        assert!(wrong.add_owned_native_archive(archive, AccountedNativeOpenPolicy::KeepCurrentInterpretation, "stock.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.documents().is_empty());
        assert!(wrong.add_owned_native_point_archive(fixture::unmarked_archive(&owner), "stock.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.documents().is_empty());
        let mut right = Session::new();
        right.add_owned_native_filtered_flat_archive(fixture::unmarked_archive(&owner), "stock.pcraft", None, &fixture::ctl()).unwrap();
        let current = &right.active().unwrap().doc;
        assert!(current.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        fixture::storage(owner.document(), current);
        assert_eq!(fixture::archive(current.accounted().unwrap()).current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::FilteredFlatPoint));
    }
}
