mod common;
use photocraft_xcf::*;
fn string(out: &mut Vec<u8>, s: &str) {
    common::u32be(out, (s.len() + 1) as u32);
    out.extend(s.as_bytes());
    out.push(0);
}
#[test]
fn physical_units_and_two_axis_resolution_are_preserved() {
    for (id, unit) in [(1u32, Unit::Inch), (2, Unit::Millimeter), (3, Unit::Point), (4, Unit::Pica)] {
        let mut dpi = 96f32.to_be_bytes().to_vec();
        dpi.extend(144f32.to_be_bytes());
        let options = common::FixtureOptions { image_properties: vec![(22, id.to_be_bytes().to_vec()), (19, dpi)], ..Default::default() };
        let f = decode(&common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).unwrap();
        assert_eq!(f.unit, unit);
        assert_eq!(f.dpi, (96., 144.));
    }
}
#[test]
fn custom_units_use_versioned_layout_despite_broken_declared_length() {
    for version in [19, 21, 26] {
        let mut payload = 2.5f32.to_be_bytes().to_vec();
        payload.extend(3u32.to_be_bytes());
        for name in ["Original unit Ω", "Ω", "ou"] {
            string(&mut payload, name);
        }
        if version < 21 {
            string(&mut payload, "original unit");
            string(&mut payload, "original units");
        }
        let options = common::FixtureOptions { image_properties: vec![(24, payload.clone())], ..Default::default() };
        for declared in [0, payload.len() as u32 + 100] {
            let mut bytes = common::image_custom(version, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
            bytes[43..47].copy_from_slice(&declared.to_be_bytes());
            let f = decode(&bytes, &Default::default()).unwrap();
            let Unit::Custom(unit) = f.unit else { panic!("custom unit lost") };
            assert_eq!(unit.factor.to_bits(), 2.5f32.to_bits());
            assert_eq!(unit.digits, 3);
            assert_eq!(unit.name, "Original unit Ω");
            assert_eq!(unit.singular.as_deref(), (version < 21).then_some("original unit"));
            let p = f.raw_properties.iter().find(|p| p.tag == 24).unwrap();
            assert_eq!(p.payload, payload);
            assert_eq!(p.declared_bytes, declared);
        }
    }
}
