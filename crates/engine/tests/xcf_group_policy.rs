#![cfg(feature = "corpus")]
//! Actual independent GIMP group sources keep current composition semantics after commands.
use photocraft_doc::{Document, import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH};
use photocraft_engine::Session;
use serde_json::json;
use std::{fs, path::PathBuf};

fn sources() -> Vec<Document> {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    [
        ("group-28-none-mask.xcf", "bfcc08e961d20e4c74d9eec32990a5f53f6ffbc6ce35814e2ade33ca1fe08863"),
        ("group-28-none-mask.edit.xcf", "00d4d5ab57b1a934e39113b7d33fa2a2319139bae606f908c628f0afd176aea2"),
    ]
    .into_iter()
    .map(|(file, hash)| {
        let raw = fs::read(root.join(file)).expect("run the full XCF corpus fetch or set PHOTOCRAFT_XCF_FULL_CORPUS");
        assert_eq!(blake3::hash(&raw).to_hex().as_str(), hash);
        photocraft_io::import(file, &raw).unwrap().document
    })
    .collect()
}
fn alpha(doc: &Document) -> Vec<u64> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap().px.iter().map(|pixel| pixel[3].to_bits()).collect()
}
fn seeded(doc: &Document) {
    let state = doc.xcf_state.as_ref().unwrap();
    assert_eq!(
        state
            .items
            .iter()
            .filter(|item| item.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == XCF13_ISOLATED_MULTIPLY_SEED_EPOCH))
            .count(),
        1
    );
}
fn native(doc: &Document) {
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded, *doc);
    seeded(&loaded);
    assert_eq!(alpha(&loaded), alpha(doc));
}
fn run(original: &Document, command: &str, params: serde_json::Value) -> Document {
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    session.execute(command, params).unwrap();
    let edited = (*session.active().unwrap().doc).clone();
    seeded(&edited);
    native(&edited);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, *original);
    assert_eq!(alpha(&session.active().unwrap().doc), alpha(original));
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, edited);
    edited
}

#[test]
#[ignore = "requires unpublished local full XCF corpus; run this named suite with --ignored and PHOTOCRAFT_XCF_FULL_CORPUS"]
fn imported_seed_survives_explicit_profile_assignment_conversion_native_and_undo() {
    for original in sources() {
        seeded(&original);
        native(&original);
        let before = alpha(&original);
        for command in ["edit.assignProfile", "edit.convertToProfile"] {
            let edited = run(&original, command, json!({"profile":"srgb"}));
            assert_eq!(edited.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc);
            assert_eq!(alpha(&edited), before, "{command} must not change group initialization");
        }
    }
}

#[test]
#[ignore = "requires unpublished local full XCF corpus; run this named suite with --ignored and PHOTOCRAFT_XCF_FULL_CORPUS"]
fn imported_seed_survives_depth_and_nearest_resize_native_and_undo() {
    for original in sources() {
        seeded(&original);
        let depth = run(&original, "image.mode.bits16", json!({}));
        assert_eq!(depth.pixel_format().sample, photocraft_doc::SampleType::U16);
        // Quantization is an intentional sample conversion, not a group policy change.
        let before = alpha(&original);
        let quantized = alpha(&depth);
        for (a, b) in before.iter().zip(&quantized) {
            assert!((f64::from_bits(*a) - f64::from_bits(*b)).abs() <= 2.0 / 65535.0);
        }
        let resized = run(&original, "image.imageSize", json!({"width":32,"height":24,"resample":"nearest"}));
        assert_eq!(resized.size.width, 32);
        let scaled = alpha(&resized);
        // Existing image resizing uses bilinear mask resampling independently of
        // the selected pixel filter. Native and undo above preserve the exact
        // resulting alpha, rather than claiming a GIMP resize oracle here.
        assert_eq!(scaled.len(), 32 * 24);
        assert!(scaled.iter().any(|bits| (0.0..1.0).contains(&f64::from_bits(*bits))));
    }
}
