mod common;
use photocraft_xcf::*;
fn link(flags: u32, path: &str) -> Vec<u8> {
    let mut out = flags.to_be_bytes().to_vec();
    common::u32be(&mut out, path.len() as u32 + 1);
    out.extend(path.as_bytes());
    out.push(0);
    common::u32be(&mut out, u32::MAX); // inert vector source extent, never allocated
    common::u32be(&mut out, u32::MAX);
    out
}
fn transform(interpolation: u32, nonfinite: bool) -> Vec<u8> {
    let mut out = i32::MIN.to_be_bytes().to_vec();
    out.extend(i32::MAX.to_be_bytes());
    common::u32be(&mut out, interpolation);
    for value in [1f32, -0., 3.25, 0., 1., -2.5, 0.125, 0., if nonfinite { f32::NAN } else { 1. }] {
        out.extend(value.to_be_bytes());
    }
    out
}
fn fixture(version: u32, properties: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
    common::image_custom(version, 150, 0, 1, 1, &[11, 22, 33, 255], &common::FixtureOptions { layer_properties: properties, ..Default::default() })
}
#[test]
fn linked_and_transform_records_require_the_documented_structure_epoch() {
    for version in [0, 12, 22, 24] {
        for (tag, body) in [(48, link(0, "missing-own-resource.png")), (49, transform(0, false))] {
            assert_eq!(
                decode(&fixture(version, vec![(tag, body)]), &Default::default()).unwrap_err().kind,
                ErrorKind::Unsupported,
                "version{version} tag{tag}"
            );
        }
    }
}
#[test]
fn inert_link_flags_pre_offset_projective_bits_and_raster_are_retained() {
    for flags in 0u32..4 {
        for interpolation in 0..5 {
            let source_link = link(flags, "../../missing-own-resource-Ω.png");
            let source_transform = transform(interpolation, false);
            let file = decode(&fixture(25, vec![(49, source_transform.clone()), (48, source_link.clone())]), &Default::default()).unwrap();
            let layer = &file.layers[0];
            let linked = layer.link.as_ref().unwrap();
            assert_eq!(linked.flags, flags);
            assert_eq!(linked.custom_name(), flags & 1 != 0);
            assert_eq!(linked.monitoring_disabled(), flags & 2 != 0);
            assert_eq!(linked.unknown_flags(), 0);
            assert_eq!(linked.path, "../../missing-own-resource-Ω.png");
            assert_eq!((linked.width, linked.height), (u32::MAX, u32::MAX));
            let transform = layer.transform.as_ref().unwrap();
            assert_eq!(transform.offset, (i32::MIN, i32::MAX));
            assert_eq!(transform.interpolation, interpolation);
            assert_eq!(transform.matrix[1].to_bits(), (-0.0f32).to_bits());
            assert_eq!(transform.matrix[6].to_bits(), 0.125f32.to_bits());
            assert_eq!(layer.plane.as_ref().unwrap().bytes, [11, 22, 33, 255]);
            assert_eq!(layer.raw_properties[0].payload, source_transform);
            assert_eq!(layer.raw_properties[1].payload, source_link);
            assert!(layer.unsupported.iter().any(|message| message.contains("linked layer")));
        }
    }
}
#[test]
fn unknown_link_flags_remain_named_and_hostile_transform_values_fail() {
    let file = decode(&fixture(25, vec![(48, link(0x80000003, "own-inert://missing"))]), &Default::default()).unwrap();
    assert_eq!(file.layers[0].link.as_ref().unwrap().flags, 0x80000003);
    assert!(file.layers[0].unsupported.iter().any(|message| message.contains("unknown linked-layer flags")));
    for body in [transform(5, false), transform(0, true), transform(0, false)[..44].to_vec()] {
        assert!(decode(&fixture(25, vec![(49, body)]), &Default::default()).is_err());
    }
}
