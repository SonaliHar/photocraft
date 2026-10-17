//! Deferred pathless native continuation; no producer or visual acceptance.
use super::*;
#[path = "../../engine/src/accounted_filtered_flat_stock_rgb_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;

#[test]
#[ignore = "UNRUN declared CURRENT three-kind native marker/unmarked pathless routing, exact storage and deterministic real CanvasDisplay continuation. No GIMP Source/numerical or UI visual acceptance."]
fn stock_rgb_pathless_native_keeps_storage_and_display() {
    let owner = fixture::build();
    for marked in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let archive = if marked { fixture::archive(&owner) } else { fixture::unmarked_archive(&owner) };
        adopt_bytes(&mut app, "stock.pcraft", archive, &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        fixture::storage(owner.document(), &before);
        let display = app.session.color.canvas_display_for(&before, None).unwrap();
        let (image, fee) = crate::canvas::test_backed_snapshot_display_image(
            &before,
            before.size.width,
            before.size.height,
            Some(display.as_ref()),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        let expected = image.pixels.clone();
        drop((image, fee));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        adopt_bytes(&mut reopened, "stock.pcraft", fixture::archive(before.accounted().unwrap()), &fixture::ctl()).unwrap();
        let after = reopened.session.active().unwrap().doc.clone();
        fixture::storage(&before, &after);
        let display = reopened.session.color.canvas_display_for(&after, None).unwrap();
        let (image, fee) = crate::canvas::test_backed_snapshot_display_image(
            &after,
            after.size.width,
            after.size.height,
            Some(display.as_ref()),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(image.pixels, expected);
        drop((image, fee));
    }
}
