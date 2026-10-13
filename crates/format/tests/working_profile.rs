use photocraft_doc::{ColorMode, Document, SampleType, Size, WorkingProfileInterpretation};
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes, zip::ZipReader};
use std::sync::Arc;
mod common;
fn manifest(bytes: &[u8]) -> serde_json::Value {
    let zip = ZipReader::new(bytes).unwrap();
    serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap()
}
// Format tests exercise exact identity persistence; CMS independently admits actual ICC semantics.
fn implicit(mode: ColorMode) -> Document {
    let mut doc = Document::new("Own working profile", Size::new(2, 2), mode, SampleType::F64);
    let bytes = b"original profile identity for pure native model testing".to_vec();
    doc.working_profile_interpretation =
        if mode == ColorMode::Grayscale { WorkingProfileInterpretation::implicit_sgray(&bytes) } else { WorkingProfileInterpretation::implicit_srgb(&bytes) };
    doc.icc_profile = Some(Arc::new(bytes));
    doc.source_color_context = Some(photocraft_doc::SourceColorContext { original_icc: doc.icc_profile.clone(), source_precision: 750 });
    doc
}
#[test]
fn native_default_working_profile_is_explicit_icc_policy() {
    let doc = Document::new("Own working profile", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let zip = ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(manifest["document"]["working_profile_interpretation"], "Icc");
    assert_eq!(manifest["minimum_reader_version"], 2);
}

#[test]
fn implicit_rgb_and_gray_identity_round_trip_exactly_and_require_reader_three() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Indexed] {
        let doc = implicit(mode);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(manifest(&bytes)["minimum_reader_version"], 3);
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
        let mut explicit = doc.clone();
        explicit.working_profile_interpretation = WorkingProfileInterpretation::Icc;
        let bytes = save_to_bytes(&explicit, &SaveOptions::default()).unwrap();
        assert_eq!(manifest(&bytes)["minimum_reader_version"], 2);
        assert_eq!(load_from_bytes(&bytes).unwrap(), explicit);
        assert_eq!(explicit.icc_profile, doc.icc_profile);
    }
}

#[test]
fn stale_missing_and_wrong_model_markers_are_rejected_before_save() {
    let mut doc = implicit(ColorMode::Rgb);
    doc.icc_profile = Some(Arc::new(b"changed bytes".to_vec()));
    assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
    doc.icc_profile = None;
    assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
    let mut doc = implicit(ColorMode::Rgb);
    doc.mode = ColorMode::Grayscale;
    assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
}

#[test]
fn native_downgrade_and_corrupted_marker_fail_while_legacy_absence_defaults_to_icc() {
    for variant in 0..3 {
        let doc = implicit(ColorMode::Rgb);
        let path = common::temp_dir("working-profile");
        photocraft_format::PcraftWriter::new().save_dir(&doc, &path, &SaveOptions::default()).unwrap();
        let file = path.join("manifest.json");
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        match variant {
            0 => value["minimum_reader_version"] = 2.into(),
            1 => value["document"]["working_profile_interpretation"]["ImplicitSrgb"]["icc_blake3"] = "0".repeat(64).into(),
            _ => {
                value["document"].as_object_mut().unwrap().remove("working_profile_interpretation");
                value["format_version"] = 2.into();
                value["minimum_reader_version"] = 2.into();
            }
        }
        std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        if variant == 2 {
            let loaded = photocraft_format::load_path(&path).unwrap();
            assert_eq!(loaded.working_profile_interpretation, WorkingProfileInterpretation::Icc);
            assert_eq!(loaded.icc_profile, doc.icc_profile);
            assert_eq!(loaded.source_color_context, doc.source_color_context);
        } else {
            assert!(photocraft_format::load_path(&path).is_err());
        }
        std::fs::remove_dir_all(path).unwrap();
    }
}
