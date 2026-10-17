//! Deferred genuine masked Source and native11 pathless display continuation.
use super::*;
#[path = "../../engine/src/accounted_masked_scalar_test_support.rs"]
mod fixture;

#[test]
#[ignore = "UNRUN genuine16 masked scalar Sources→actualPoint UI→native11 pathless reopen, exact raw storage and deterministic CanvasDisplay. No GIMP display/numerical/visual acceptance."]
fn masked_scalar_source_and_native_display_continue() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "masked.xcf", &source, &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
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
        adopt_bytes(&mut reopened, "masked.pcraft", fixture::archive(before.accounted().unwrap()), &fixture::ctl()).unwrap();
        let after = reopened.session.active().unwrap().doc.clone();
        assert!(after.accounted().unwrap().is_readonly_point_storage());
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
        assert_eq!(fixture::bytes(&case), std::fs::read(fixture::path(&case)).unwrap());
    }
}
