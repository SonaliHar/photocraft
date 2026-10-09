#[path = "common/xcf.rs"]
mod common;
#[test]
fn native_reopen_preserves_xcf_samples_and_interpretation() {
    let bytes = common::image(12, 650, 2, 1, 1, &[0.25f32, 0.5, 0.75, 1.0].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>());
    let r = photocraft_io::import("float.xcf", &bytes).unwrap();
    let native = photocraft_format::save_to_bytes(&r.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(r.document.layers, reopened.layers);
    assert_eq!(r.document.sample_encoding, reopened.sample_encoding);
    assert_eq!(r.document.icc_profile, reopened.icc_profile);
    assert_eq!(r.document.source_color_context, reopened.source_color_context);
}
