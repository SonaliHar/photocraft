mod common;
use photocraft_xcf::*;
fn string(v: &mut Vec<u8>, s: &str) {
    common::u32be(v, s.len() as u32 + 1);
    v.extend(s.as_bytes());
    v.push(0);
}
fn geometry(v: &mut Vec<u8>) {
    for n in [1, 1, 1, 1, 2, 3] {
        common::u32be(v, n);
    }
    for (kind, x, y) in [(1, 1f32, 2f32), (0, 3., 4.), (1, 5., 6.)] {
        common::u32be(v, kind);
        v.extend(x.to_be_bytes());
        v.extend(y.to_be_bytes());
    }
}
#[test]
fn modern_path_preserves_selected_tattoo_and_exact_geometry() {
    let mut p = Vec::new();
    string(&mut p, "Own Ω");
    let at = p.len();
    common::u32be(&mut p, 0);
    common::prop(&mut p, 43, &[]);
    common::prop(&mut p, 20, &99u32.to_be_bytes());
    common::prop(&mut p, 42, &1u32.to_be_bytes());
    common::prop(&mut p, 0, &[]);
    geometry(&mut p);
    let len = p.len() - at - 4;
    p[at..at + 4].copy_from_slice(&(len as u32).to_be_bytes());
    let options = common::FixtureOptions { path_records: vec![p], ..Default::default() };
    let f = decode(&common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).unwrap();
    let p = &f.paths[0];
    assert_eq!(p.name, "Own Ω");
    assert!(p.selected);
    assert_eq!(p.tattoo, Some(99));
    assert!(p.lock_visibility);
    assert!(!p.raw_record.is_empty());
    assert!(p.strokes[0].closed);
    assert_eq!(p.strokes[0].points[1].components, [3f32.to_bits(), 4f32.to_bits()]);
}
#[test]
fn duplicate_path_singletons_reject_while_membership_properties_repeat() {
    for tag in [8, 20, 28, 32, 34, 42, 43, 41] {
        let mut path = Vec::new();
        string(&mut path, "Own");
        let length_at = path.len();
        common::u32be(&mut path, 0);
        let payload = if tag == 43 { vec![] } else { 0u32.to_be_bytes().to_vec() };
        for _ in 0..2 {
            common::prop(&mut path, tag, &payload);
        }
        common::prop(&mut path, 0, &[]);
        geometry(&mut path);
        let length = path.len() - length_at - 4;
        path[length_at..length_at + 4].copy_from_slice(&(length as u32).to_be_bytes());
        let options = common::FixtureOptions { path_records: vec![path], ..Default::default() };
        let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        let result = decode(&bytes, &Default::default());
        if tag == 41 {
            assert_eq!(result.unwrap().paths[0].item_sets, [0, 0]);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Structure, "tag{tag}");
        }
    }
}
#[test]
fn legacy_vector_property_retains_tattoo_and_active_index() {
    let mut p = Vec::new();
    for n in [1, 0, 1] {
        common::u32be(&mut p, n);
    }
    string(&mut p, "Own");
    for n in [99, 1, 0, 0, 1] {
        common::u32be(&mut p, n);
    }
    // geometry() includes the path version and stroke count; already supplied count above.
    let mut g = Vec::new();
    geometry(&mut g);
    p.extend(&g[8..]);
    let options = common::FixtureOptions { image_properties: vec![(25, p)], ..Default::default() };
    let f = decode(&common::image_custom(12, 150, 0, 1, 1, &[1, 2, 3, 255], &options), &Default::default()).unwrap();
    assert!(f.paths[0].selected);
    assert_eq!(f.paths[0].tattoo, Some(99));
    assert_eq!(f.paths[0].strokes[0].points.len(), 3);
}
#[test]
fn malformed_path_counts_components_lengths_and_cancel_are_errors() {
    for (nf, np, coordinate) in [(1, 1, 0f32), (7, 1, 0.), (2, u32::MAX, 0.), (2, 1, f32::INFINITY)] {
        let mut p = Vec::new();
        string(&mut p, "Own");
        let at = p.len();
        common::u32be(&mut p, 0);
        common::prop(&mut p, 0, &[]);
        for n in [1, 1, 1, 0, nf, np] {
            common::u32be(&mut p, n);
        }
        common::u32be(&mut p, 0);
        p.extend(coordinate.to_be_bytes());
        p.extend(0f32.to_be_bytes());
        let len = p.len() - at - 4;
        p[at..at + 4].copy_from_slice(&(len as u32).to_be_bytes());
        let options = common::FixtureOptions { path_records: vec![p], ..Default::default() };
        let b = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
        assert!(decode(&b, &Default::default()).is_err());
    }
    let mut p = Vec::new();
    string(&mut p, "Own");
    let at = p.len();
    common::u32be(&mut p, 0);
    common::prop(&mut p, 0, &[]);
    geometry(&mut p);
    let len = p.len() - at - 4;
    p[at..at + 4].copy_from_slice(&(len as u32).to_be_bytes());
    let options = common::FixtureOptions { path_records: vec![p], ..Default::default() };
    let b = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    let path_at = decode(&b, &Default::default()).unwrap().paths[0].record_offset.unwrap() as usize;
    let reached = std::cell::Cell::new(false);
    let progress = |n: f32| {
        if n >= 0.95 {
            reached.set(true)
        }
    };
    let cancelled = || reached.get();
    let opt = DecodeOptions { progress: Some(&progress), cancelled: Some(&cancelled), ..Default::default() };
    assert_eq!(decode(&b, &opt).unwrap_err().kind, ErrorKind::Cancelled);
    let mut wrong = b.clone();
    let length_at = path_at + 4 + 4;
    wrong[length_at..length_at + 4].copy_from_slice(&1u32.to_be_bytes());
    assert!(decode(&wrong, &Default::default()).is_err());
}
