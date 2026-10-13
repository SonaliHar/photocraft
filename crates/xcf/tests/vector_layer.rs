mod common;
use photocraft_xcf::*;
fn payload() -> Vec<u8> {
    let mut p = Vec::new();
    for n in [1, 99, 1, 1] {
        common::u32be(&mut p, n);
    }
    for _ in 0..2 {
        for n in [0, 1, 0, 0] {
            common::u32be(&mut p, n);
        }
    }
    p.extend(3f32.to_be_bytes());
    common::u32be(&mut p, 1);
    common::u32be(&mut p, 2);
    p.extend(10f32.to_be_bytes());
    common::u32be(&mut p, 2);
    p.extend(2f32.to_be_bytes());
    p.extend(1f32.to_be_bytes());
    p
}
#[test]
fn typed_vector_configuration_preserves_flags_and_exact_numbers() {
    let options = common::FixtureOptions { layer_properties: vec![(47, payload())], ..Default::default() };
    let f = decode(&common::image_custom(24, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).unwrap();
    let layer = &f.layers[0];
    let v = layer.vector.as_ref().unwrap();
    assert!(v.source_modified && v.filled && v.stroked);
    assert_eq!(v.path_tattoo, 99);
    assert_eq!(v.stroke.width_bits, 3f32.to_bits());
    assert_eq!(v.stroke.dashes_bits, [2f32.to_bits(), 1f32.to_bits()]);
    assert!(v.fill.color.is_none() && v.fill.pattern.is_none());
    assert!(layer.plane.is_some());
    assert!(layer.unsupported.iter().any(|s| s.contains("vector")));
}
#[test]
fn malformed_required_vector_fields_and_counts_fail() {
    for (offset, value) in [(0, 2), (16, 2), (payload().len() - 12, u32::MAX)] {
        let mut p = payload();
        p[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        let options = common::FixtureOptions { layer_properties: vec![(47, p)], ..Default::default() };
        assert!(decode(&common::image_custom(24, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).is_err());
    }
}
