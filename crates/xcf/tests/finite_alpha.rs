mod common;
use photocraft_xcf::*;
use std::io::Write;
fn rgba(precision: u32, alpha: f64) -> Vec<u8> {
    let pixels = if precision == 750 {
        [0.25_f64, 0.5, 0.75, alpha].into_iter().flat_map(f64::to_be_bytes).collect::<Vec<_>>()
    } else {
        [0.25_f32, 0.5, 0.75, alpha as f32].into_iter().flat_map(f32::to_be_bytes).collect()
    };
    common::image(12, precision, 0, 1, 1, &pixels)
}
fn with_mask(precision: u32, coverage: f64) -> Vec<u8> {
    let mut b = rgba(precision, 1.);
    let layer = u64::from_be_bytes(b[47..55].try_into().unwrap()) as usize;
    let name_len = u32::from_be_bytes(b[layer + 12..layer + 16].try_into().unwrap()) as usize;
    let mask_slot = layer + 16 + name_len + 8 + 8;
    let mask = b.len();
    for v in [1, 1, 5] {
        common::u32be(&mut b, v);
    }
    b.extend(b"Mask\0");
    common::prop(&mut b, 0, &[]);
    let hp = b.len();
    common::pointer(&mut b, 0, 12);
    let hierarchy = b.len();
    for v in [1, 1, if precision == 750 { 8 } else { 4 }] {
        common::u32be(&mut b, v);
    }
    let lp = b.len();
    common::pointer(&mut b, 0, 12);
    common::pointer(&mut b, 0, 12);
    let level = b.len();
    common::u32be(&mut b, 1);
    common::u32be(&mut b, 1);
    let tp = b.len();
    common::pointer(&mut b, 0, 12);
    common::pointer(&mut b, 0, 12);
    let tile = b.len();
    if precision == 750 {
        b.extend(coverage.to_be_bytes());
    } else {
        b.extend((coverage as f32).to_be_bytes());
    }
    common::patch(&mut b, mask_slot, mask, 12);
    common::patch(&mut b, hp, hierarchy, 12);
    common::patch(&mut b, lp, level, 12);
    common::patch(&mut b, tp, tile, 12);
    b
}
#[test]
fn finite_nonunit_drawable_alpha_requires_explicit_opt_in_and_preserves_bits() {
    for precision in [650, 750] {
        for alpha in [-0.017882155254483223, 1.0387777090072632] {
            let bytes = rgba(precision, alpha);
            let error = decode(&bytes, &Default::default()).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Unsupported);
            assert!(error.message.contains("alpha/mask coverage"));
            let options = DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() };
            let file = decode(&bytes, &options).unwrap();
            let plane = file.layers[0].plane.as_ref().unwrap();
            let expected = if precision == 750 { alpha.to_ne_bytes().to_vec() } else { (alpha as f32).to_ne_bytes().to_vec() };
            assert_eq!(&plane.bytes[plane.bytes.len() - expected.len()..], expected);
        }
    }
}
#[test]
fn opted_in_drawable_alpha_still_rejects_nonfinite_values() {
    let options = DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() };
    for precision in [650, 750] {
        for alpha in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(decode(&rgba(precision, alpha), &options).is_err());
        }
    }
}
#[test]
fn opt_in_never_relaxes_mask_coverage() {
    let options = DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() };
    for precision in [650, 750] {
        let valid = decode(&with_mask(precision, 0.5), &options).unwrap();
        assert!(valid.layers[0].mask.is_some());
        for coverage in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
            assert!(decode(&with_mask(precision, coverage), &options).is_err());
        }
    }
}

#[test]
fn bounded_container_parser_preserves_explicit_drawable_alpha_policy() {
    let plain = rgba(750, -0.017882155254483223);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain).unwrap();
    let wrapped = encoder.finish().unwrap();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    assert!(decode_with_resources(&wrapped, &Default::default(), &limits).is_err());
    let options = DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() };
    let decoded = decode_with_resources(&wrapped, &options, &limits).unwrap();
    assert_eq!(decoded.file.layers[0].plane.as_ref().unwrap().bytes, decode(&plain, &options).unwrap().layers[0].plane.as_ref().unwrap().bytes);
}

#[test]
#[ignore = "requires the reviewed local float-alpha composition originals"]
fn genuine_six_float_resample_edits_decode_exact_only_with_opt_in() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let mut cases = Vec::new();
    for model in ["rgb", "gray"] {
        cases.push(("wide-transform-evidence", format!("double_non_linear-{model}-impulse.edit")));
        for geometry in ["rotation", "projective"] {
            cases.push(("wide-resample-geometry-evidence", format!("cubic-double_non_linear-{model}-{geometry}.edit")));
        }
    }
    let options = DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() };
    for (folder, id) in cases {
        let source = root.join(folder).join(format!("{id}.xcf"));
        let bytes = std::fs::read(&source).unwrap();
        assert_eq!(decode(&bytes, &Default::default()).unwrap_err().kind, ErrorKind::Unsupported, "default remains strict: {id}");
        let decoded = decode(&bytes, &options).unwrap();
        let expected = std::fs::read(root.join("float-alpha-composition-evidence").join(format!("{id}.transparent.source.native.bin"))).unwrap();
        assert_eq!(decoded.layers[0].plane.as_ref().unwrap().bytes, expected, "unchanged native samples: {id}");
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}
