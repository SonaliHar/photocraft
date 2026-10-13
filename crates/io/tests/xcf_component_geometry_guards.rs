#![cfg(feature = "corpus")]
//! Declared hostile recodings preserve the frozen stock originals.
use serde_json::Value;
#[test]
#[ignore = "local unpublished Component geometry source-field controls"]
fn geometry_source_envelope_and_typed_arguments_remain_closed() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-geometry-evidence");
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["enabled"] == true && row["invert"] == false && row["linear"] == false) {
        let path = root.join(row["source"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        photocraft_io::import("positive.xcf", &bytes).unwrap();
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let effect = &raw.layers[0].effects[0];
        for (tag, value) in [(7, 28u32), (35, 0), (36, 0), (37, 0), (44, 1), (46, 1), (33, 0.5f32.to_bits())] {
            let property = effect.properties.iter().find(|p| p.tag == tag).unwrap();
            assert_eq!(property.payload.len(), 4);
            let start = usize::try_from(property.offset).unwrap();
            let mut changed = bytes.clone();
            changed[start..start + 4].copy_from_slice(&value.to_be_bytes());
            assert!(photocraft_io::import("declared-hostile.xcf", &changed).is_err(), "{} tag{tag}", row["id"]);
            count += 1;
        }
        for name in ["component", "invert", "linear"] {
            let argument = effect.arguments.iter().find(|a| a.name == name).unwrap();
            let property = &effect.properties[argument.property_index];
            let end = usize::try_from(property.offset).unwrap() + property.payload.len();
            let mut changed = bytes.clone();
            if name == "component" {
                changed[end - 4..end].copy_from_slice(&3u32.to_be_bytes());
            } else {
                assert_eq!(&changed[end - 8..end - 4], &2u32.to_be_bytes());
                changed[end - 8..end - 4].copy_from_slice(&3u32.to_be_bytes());
            }
            assert!(photocraft_io::import("declared-hostile.xcf", &changed).is_err(), "{} {name}", row["id"]);
            count += 1;
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    assert_eq!(count, 440);
    println!("440 geometry source envelope/type rejections after44 positive baselines");
}
