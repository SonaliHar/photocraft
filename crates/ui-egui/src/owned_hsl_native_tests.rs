//! Deferred pathless HSL native storage and internal display continuation.
use super::*;
#[path = "../../engine/src/accounted_filtered_flat_hsl_test_support.rs"]
mod fixture;

#[test]
#[ignore = "UNRUN declared CURRENT HSL marked/unmarked pathless adoption, exact storage and real CanvasDisplay continuation. Internal only, no producer numerical/Float-alpha or visual acceptance."]
fn hsl_pathless_native_retains_internal_display() {
    let owner = fixture::build();
    for marked in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let archive = if marked { fixture::archive(&owner) } else { fixture::unmarked_archive(&owner) };
        adopt_bytes(&mut app, "hsl.pcraft", archive, &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        fixture::storage(owner.document(), &before);
        let display = app.session.color.canvas_display_for(&before, None).unwrap();
        let (image, fee) = crate::canvas_backed_preview::backed_snapshot_display_image(
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
        adopt_bytes(&mut reopened, "hsl.pcraft", fixture::archive(before.accounted().unwrap()), &fixture::ctl()).unwrap();
        let after = reopened.session.active().unwrap().doc.clone();
        fixture::storage(&before, &after);
        assert!(after.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        let display = reopened.session.color.canvas_display_for(&after, None).unwrap();
        let (image, fee) = crate::canvas_backed_preview::backed_snapshot_display_image(
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
