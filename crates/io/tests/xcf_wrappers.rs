#[path = "common/xcf.rs"]
mod common;
use std::io::Write;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn renamed_wrapped_xcf_routes_by_content_and_remains_read_only() {
    let plain = common::image(12, 150, 0, 1, 1, &[40, 60, 80, 255]);
    let wrapped = gzip(&plain);
    let expected = photocraft_io::import("plain.xcf", &plain).unwrap();
    let plain_assessment = expected.assessment.as_ref().unwrap();
    assert_eq!(plain_assessment.fidelity, photocraft_io::xcf_assessment::Fidelity::Full);
    assert_eq!(plain_assessment.container, "plain");
    for name in ["renamed.bin", "misleading.psd", "project.xcf.gz"] {
        let imported = photocraft_io::import(name, &wrapped).unwrap();
        assert_eq!(imported.origin, photocraft_io::ImportOrigin::xcf());
        assert_eq!(imported.document.layers.len(), expected.document.layers.len());
        assert_eq!(imported.document.layers[0].surface(), expected.document.layers[0].surface());
        let provenance = &imported.document.xcf_state.as_ref().unwrap().provenance;
        assert_eq!(provenance.container, "gzip");
        assert_eq!(provenance.source_hash.as_deref(), Some(blake3::hash(&wrapped).to_hex().as_str()));
        let assessment = imported.assessment.as_ref().unwrap();
        assert_eq!(assessment.container, "gzip");
        assert_eq!(assessment.source_version, 12);
        assert_eq!(assessment.fidelity, plain_assessment.fidelity);
        assert_eq!(assessment.dependencies, plain_assessment.dependencies);
        let current = photocraft_io::xcf_assessment::assess_xcf_document(&imported.document, &photocraft_raster::Interrupt::NONE).unwrap().unwrap();
        assert_eq!(current, *assessment);
    }
}

#[test]
fn valid_svgz_still_reaches_svg_loader() {
    let wrapped = gzip(br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#ff0000"/></svg>"##);
    let imported = photocraft_io::import("shape.svgz", &wrapped).unwrap();
    assert!(!imported.origin.is_read_only());
    assert_eq!((imported.document.size.width, imported.document.size.height), (2, 2));
    assert!(imported.assessment.is_none());
}

#[test]
fn corrupt_xcf_wrapper_fails_and_compound_destinations_are_rejected() {
    let plain = common::image(12, 150, 0, 1, 1, &[40, 60, 80, 255]);
    let mut wrapped = gzip(&plain);
    let n = wrapped.len();
    wrapped[n - 8] ^= 1;
    assert!(photocraft_io::import("project.xcf.gz", &wrapped).is_err());
    let doc = photocraft_io::import("plain.xcf", &plain).unwrap().document;
    for name in ["new.XCF.GZ", "new.xcf.bz2", "new.xcf.xz", "xcf.gz"] {
        assert!(matches!(photocraft_io::export(&doc, name, &Default::default()), Err(photocraft_io::IoError::Unsupported(_))));
    }
}
