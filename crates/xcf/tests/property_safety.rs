mod common;
use photocraft_xcf::*;
fn image(image_properties: Vec<(u32, Vec<u8>)>, layer_properties: Vec<(u32, Vec<u8>)>, effect_records: Vec<Vec<u8>>) -> Vec<u8> {
    common::image_custom(
        22,
        150,
        0,
        1,
        1,
        &[1, 2, 3, 255],
        &common::FixtureOptions { image_properties, layer_properties, effect_records, ..Default::default() },
    )
}
fn string(out: &mut Vec<u8>, value: &str) {
    common::u32be(out, (value.len() + 1) as u32);
    out.extend(value.as_bytes());
    out.push(0);
}
fn effect(properties: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
    let mut out = Vec::new();
    for value in ["Own effect", "", "gegl:exposure", "1:0"] {
        string(&mut out, value);
    }
    for (tag, payload) in properties {
        common::prop(&mut out, tag, &payload);
    }
    common::prop(&mut out, 0, &[]);
    common::pointer(&mut out, 0, 22);
    out
}
#[test]
fn duplicate_singleton_properties_reject_instead_of_overwriting_or_appending() {
    for tag in [6, 7, 8, 20, 28, 30, 32, 33, 35, 36, 37, 42] {
        let payload = if tag == 33 { 0.5f32.to_be_bytes().to_vec() } else { 0u32.to_be_bytes().to_vec() };
        let result = decode(&image(vec![], vec![(tag, payload.clone()), (tag, payload)], vec![]), &Default::default());
        assert!(result.is_err(), "drawable singleton{tag} duplicated");
    }
    for (tag, payload) in [(1, [1u32.to_be_bytes().to_vec(), vec![1, 2, 3]].concat()), (20, 1u32.to_be_bytes().to_vec()), (22, 1u32.to_be_bytes().to_vec())] {
        assert!(decode(&image(vec![(tag, payload.clone()), (tag, payload)], vec![], vec![]), &Default::default()).is_err(), "image singleton{tag}");
    }
    // The common generator already emits COMPRESSION; this must not win last.
    assert!(decode(&image(vec![(17, vec![1])], vec![], vec![]), &Default::default()).is_err());
    for tag in [7, 8, 33, 35, 36, 37, 44, 46] {
        let payload = if tag == 33 { 0.5f32.to_be_bytes().to_vec() } else { 0u32.to_be_bytes().to_vec() };
        let record = effect(vec![(tag, payload.clone()), (tag, payload)]);
        assert!(decode(&image(vec![], vec![], vec![record]), &Default::default()).is_err(), "effect singleton{tag}");
    }
}
#[test]
fn repeated_collection_properties_remain_legal_and_ordered() {
    let mut set = 0u32.to_be_bytes().to_vec();
    set.extend(u32::MAX.to_be_bytes());
    string(&mut set, "Own named set");
    let file = decode(
        &image(vec![(40, set.clone()), (40, set)], vec![(41, 0u32.to_be_bytes().to_vec()), (41, 1u32.to_be_bytes().to_vec())], vec![]),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(file.item_sets.len(), 2);
    assert_eq!(file.layers[0].item_sets, [0, 1]);
    let mut parasite = Vec::new();
    string(&mut parasite, "own inert metadata");
    common::u32be(&mut parasite, 0);
    common::u32be(&mut parasite, 0);
    assert!(decode(&image(vec![(21, parasite.clone()), (21, parasite)], vec![], vec![]), &Default::default()).is_ok());
}
#[test]
fn duplicate_empty_icc_records_are_not_mistaken_for_absence() {
    let mut parasite = Vec::new();
    string(&mut parasite, "icc-profile");
    common::u32be(&mut parasite, 0);
    common::u32be(&mut parasite, 0);
    let result = decode(&image(vec![(21, parasite.clone()), (21, parasite)], vec![], vec![]), &Default::default());
    assert_eq!(result.unwrap_err().kind, ErrorKind::Structure);
}
#[test]
fn legacy_negative_guides_are_ignored_but_raw_source_is_retained() {
    let mut guides = (-3i32).to_be_bytes().to_vec();
    guides.push(1);
    guides.extend(2i32.to_be_bytes());
    guides.push(2);
    for version in [0, 14, 15, 26] {
        let options = common::FixtureOptions { image_properties: vec![(18, guides.clone())], ..Default::default() };
        let bytes = common::image_custom(version, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        let file = decode(&bytes, &Default::default()).unwrap();
        assert_eq!(file.guides.len(), if version < 15 { 1 } else { 2 }, "version{version}");
        assert_eq!(file.raw_properties.iter().find(|property| property.tag == 18).unwrap().payload, guides);
    }
}
#[test]
fn string_byte_lengths_utf8_terminators_and_embedded_nul_are_checked() {
    let options = common::FixtureOptions { name: Some("Own Ω 日本語".into()), ..Default::default() };
    let valid = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    assert_eq!(decode(&valid, &Default::default()).unwrap().layers[0].name, "Own Ω 日本語");
    let mut malformed = valid.clone();
    let start = malformed.windows("Own Ω 日本語".len()).position(|bytes| bytes == "Own Ω 日本語".as_bytes()).unwrap();
    malformed[start] = 0xff;
    assert!(decode(&malformed, &Default::default()).is_err());
    malformed = valid.clone();
    malformed[start + "Own Ω 日本語".len()] = 1;
    assert!(decode(&malformed, &Default::default()).is_err());
    malformed = valid;
    malformed[start - 4..start].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(decode(&malformed, &Default::default()).unwrap_err().kind, ErrorKind::Limit);
    let options = common::FixtureOptions { name: Some("Own\0ambiguous".into()), ..Default::default() };
    assert!(decode(&common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).is_err());
}
#[test]
fn untrusted_color_resource_byte_counts_fail_before_allocation() {
    for profile in [false, true] {
        let mut value = Vec::new();
        string(&mut value, "own color");
        common::u32be(&mut value, 8); // typed GeglColor
        string(&mut value, "RGBA double");
        if profile {
            common::u32be(&mut value, 0); // no samples
        }
        common::u32be(&mut value, u32::MAX);
        let record = effect(vec![(45, value)]);
        assert_eq!(decode(&image(vec![], vec![], vec![record]), &Default::default()).unwrap_err().kind, ErrorKind::Limit);
    }
}
