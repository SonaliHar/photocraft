#![cfg(feature = "corpus")]
//! Modified source buffers retain their pixel authority and inert regeneration metadata.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn artifact(base: &Path, fixture: &Value, role: &str) -> Vec<u8> {
    let row = fixture["artifacts"].as_array().unwrap().iter().find(|r| r["role"] == role).unwrap();
    let bytes = fs::read(base.join(row["path"].as_str().unwrap())).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, row["sha256"].as_str().unwrap());
    bytes
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn modified_source_originals_require_independent_pixel_authority() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf");
    let manifest: Value = serde_json::from_slice(&fs::read(base.join("full/coverage-fixtures.json")).unwrap()).unwrap();
    let mut admitted = 0;
    for case in ["text-painted", "vector-modified"] {
        let fixture = manifest["fixtures"].as_array().unwrap().iter().find(|f| f["id"] == format!("feature002-gimp326-{case}")).unwrap();
        for (role, reference_role) in [("source", "render"), ("reference_edit", "reference_edit_render")] {
            let input = artifact(&base, fixture, role);
            if case == "vector-modified" || role == "reference_edit" {
                assert!(photocraft_io::import("regenerated-source.xcf", &input).unwrap_err().to_string().contains("regeneration"));
                continue;
            }
            let imported = photocraft_io::import("modified.xcf", &input).unwrap();
            assert!(imported.warnings.iter().any(|w| w.contains("regeneration is unavailable")));
            let source = imported.document;
            let before = source.clone();
            let state = source.xcf_state.as_ref().unwrap();
            assert_eq!(state.source_items.len(), 1);
            let cache = state.source_items[0].cache.as_ref().unwrap();
            cache.validate().unwrap();
            let photocraft_doc::item_identity::ItemRef::Layer(owner) = state.source_items[0].owner else { panic!() };
            let layer = source.layer(owner).unwrap();
            let bounds = state.items.iter().find(|i| i.item == state.source_items[0].owner).unwrap().raster_bounds.unwrap();
            assert_eq!(layer.surface().unwrap().try_read_interleaved(bounds).unwrap(), cache.bytes.as_slice());
            let native = photocraft_io::export(&source, "modified.pcraft", &Default::default()).unwrap();
            let reopened = photocraft_io::import("modified.pcraft", &native.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&source, "modified.pcraft"), reopened);
            let expected = photocraft_codecs::decode(&artifact(&base, fixture, reference_role)).unwrap();
            for doc in [&source, &reopened] {
                let actual = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                let mut worst = 0.0_f64;
                let mut witness = (0, 0, 0, 0.0, 0.0);
                for (pixel, index) in actual.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        let reference = f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel));
                        let error = (value - reference).abs();
                        if error > worst {
                            worst = error;
                            witness = (index % doc.size.width, index / doc.size.width, channel, *value, reference);
                        }
                    }
                }
                assert!(worst <= 2.0 / 255.0, "{case}/{role}: {}/255 at {witness:?}", worst * 255.0);
            }
            assert_eq!(source, before);
            admitted += 1;
        }
    }
    assert_eq!(admitted, 1);
}
