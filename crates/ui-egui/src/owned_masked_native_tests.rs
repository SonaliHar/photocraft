//! Deferred pathless native11 marker routing and deterministic native continuation.
use super::*;
#[path = "../../engine/src/accounted_exposure_masked_test_support.rs"]
mod fixture;
#[test]
#[ignore = "UNRUN genuine6 Source→UIactualPoint→v11 pathless nativeadoption, exactmask/native/current display andsource bytes. Internal continuation only, no producer display or visual acceptance."]
fn masked_native_marker_routes_to_point_and_preserves_display() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "masked.xcf", &source, &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
        let display = app.session.color.canvas_display_for(&before, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&before, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        let expected = image.pixels.clone();
        drop((image, fee));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        adopt_bytes(&mut reopened, "masked.pcraft", fixture::archive(before.accounted().unwrap()), &fixture::ctl()).unwrap();
        let after = reopened.session.active().unwrap().doc.clone();
        assert!(after.accounted().unwrap().is_readonly_point_storage());
        fixture::same(&before, &after);
        let display = reopened.session.color.canvas_display_for(&after, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&after, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected);
        drop((image, fee));
        assert_eq!(fixture::bytes(&case), std::fs::read(fixture::workspace().join(case["source"].as_str().unwrap())).unwrap());
    }
}
