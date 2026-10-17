//! A copied bounded finite-alpha policy must never become an ordinary pasted layer.
use serde_json::json;
#[cfg(feature = "corpus")]
use sha2::{Digest, Sha256};
#[cfg(feature = "corpus")]
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn finite_source_clipboard_retains_policy_and_rejects_ordinary_installation() {
    let base = std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap();
    let packet_root = std::path::Path::new(&base).join("float-alpha-composition-evidence");
    let manifest = std::fs::read(packet_root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(manifest)), "adecf73e948e7c62db8ecee561cd440d1eefcbd0f76ef57e5d4c5614619bac37");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(packet_root.join("observations.json")).unwrap()).unwrap();
    let original = observations["records"].as_array().unwrap().iter().find(|row| row["original"]["file"] == "double_non_linear-rgb-impulse.edit.xcf").unwrap();
    let bytes = std::fs::read(std::path::Path::new(&base).join("wide-transform-evidence/double_non_linear-rgb-impulse.edit.xcf")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), original["original"]["sha256"].as_str().unwrap());
    let doc = photocraft_io::import("finite-clipboard.xcf", &bytes).unwrap().document;
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc.clone(), None).unwrap();
    session.execute("edit.copy", json!({})).unwrap();
    let clip = session.clipboard.clone().unwrap();
    assert!(clip.composition.iter().any(|p| p.compatibility_epoch == photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH));
    let native = clip.surface.try_read_interleaved(clip.bounds).unwrap();
    for command in ["edit.paste", "file.newFromClipboard"] {
        assert!(session.execute(command, json!({})).is_err(), "{command}");
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        let retained = session.clipboard.as_ref().unwrap();
        assert_eq!(retained.composition, clip.composition);
        assert_eq!(retained.surface.try_read_interleaved(retained.bounds).unwrap(), native);
    }
}

#[test]
fn ordinary_clipboard_nonunit_alpha_is_rejected_atomically() {
    use photocraft_doc::{ColorMode, Document, SampleType, WorkingProfileInterpretation};
    use photocraft_engine::edit_cmds::{Clip, ClipColorContext};
    let area = photocraft_doc::Rect::new(0, 0, 1, 1);
    let doc = Document::new("Own alpha guard", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::F64);
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc.clone(), None).unwrap();
    for alpha in [-0.01, 1.03, f64::NAN] {
        let raw: Vec<_> = [0.5, 0.2, 0.7, alpha].iter().flat_map(|value| value.to_ne_bytes()).collect();
        session.clipboard = Some(Clip {
            surface: photocraft_raster::Surface::try_from_interleaved(photocraft_color::PixelFormat::RGBA64F, area, &raw).unwrap(),
            bounds: area,
            color: ClipColorContext { mode: ColorMode::Rgb, icc_profile: None, sample_encoding: None, interpretation: WorkingProfileInterpretation::Icc },
            composition: Vec::new(),
            layers: None,
            resolution: None,
        });
        for command in ["edit.paste", "file.newFromClipboard"] {
            assert!(session.execute(command, json!({})).is_err());
            assert_eq!(*session.active().unwrap().doc, doc);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            assert_eq!(session.clipboard.as_ref().unwrap().surface.try_read_interleaved(area).unwrap(), raw);
        }
    }
}
