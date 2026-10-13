#![cfg(feature = "corpus")]
//! Private-byte recoding controls: immutable authored source/oracles stay unchanged.
use photocraft_raster::Interrupt;
use sha2::{Digest, Sha256};

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn mixed_alpha_clip_saved_tuple_cannot_fall_back_to_another_epoch() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-alpha-clip-value-domain-nonlinear-controls");
    let path = root.join("rgb-float_non_linear.unit.ordinary.xcf");
    let original = std::fs::read(&path).unwrap();
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let row = observations["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb-float_non_linear.unit.ordinary").unwrap();
    let hash = Sha256::digest(&original).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hash, row["sha256"]);
    let source = photocraft_xcf::decode(&original, &Default::default()).unwrap();
    assert!(photocraft_io::xcf_import::import("original.xcf", &original, &Interrupt::NONE).is_ok());
    for tag in [35u32, 36, 37] {
        let mut record = Vec::new();
        record.extend_from_slice(&tag.to_be_bytes());
        record.extend_from_slice(&4u32.to_be_bytes());
        record.extend_from_slice(&(-305954223i32).to_be_bytes());
        let positions = original.windows(record.len()).enumerate().filter(|(_, bytes)| *bytes == record).map(|(offset, _)| offset).collect::<Vec<_>>();
        assert_eq!(positions.len(), 1, "unique exact effect property tag{tag}");
        for value in [0i32, -1710980527] {
            let mut private = original.clone();
            let offset = positions[0] + 8;
            private[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            let decoded = photocraft_xcf::decode(&private, &Default::default()).unwrap();
            let actual_plane = decoded.layers[0].plane.as_ref().unwrap();
            let source_plane = source.layers[0].plane.as_ref().unwrap();
            assert_eq!(actual_plane.bytes, source_plane.bytes);
            assert_eq!(
                (actual_plane.width, actual_plane.height, actual_plane.channels, actual_plane.sample),
                (source_plane.width, source_plane.height, source_plane.channels, source_plane.sample)
            );
            let state = &decoded.layers[0].effects[0].state;
            assert_eq!(
                match tag {
                    35 => state.composite_mode,
                    36 => state.composite_space,
                    _ => state.blend_space,
                },
                Some(value)
            );
            let imported = photocraft_io::xcf_import::import("private-mixed-tuple.xcf", &private, &Interrupt::NONE);
            assert!(imported.is_err(), "tag{tag}={value} mixed tuple admitted through another epoch");
        }
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
