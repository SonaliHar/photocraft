#![cfg(feature = "corpus")]
//! Own saved Normal28 explicit operators with independently observed ordinary masks.
#[path = "gimp_accounted_support/mod.rs"]
mod accounted_support;
use photocraft_compose::gimp_render::{PreparedRenderer, try_render};
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, LayerMask, PixelFormat, Rect, SampleEncoding, SampleType, Size, SourceColorContext, WorkingProfileInterpretation,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{ItemRef, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn read(root: &Path, blob: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(bytes.len(), blob["bytes"].as_u64().unwrap() as usize);
    assert_eq!(hash(&bytes), blob["sha256"].as_str().unwrap());
    bytes
}
fn native(bytes: &[u8], wide: bool) -> Vec<u8> {
    if wide {
        bytes.as_chunks::<8>().0.iter().flat_map(|p| f64::from_le_bytes(*p).to_ne_bytes()).collect()
    } else {
        bytes.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect()
    }
}
fn bounds(row: &Value) -> Rect {
    Rect::from_xywh(
        row["offset"][0].as_i64().unwrap() as i32,
        row["offset"][1].as_i64().unwrap() as i32,
        row["size"][0].as_u64().unwrap() as u32,
        row["size"][1].as_u64().unwrap() as u32,
    )
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn independently_authored_normal_operator_mask_matrix_matches_prepared_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("normal-mask-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("gimp_normal_mask_hashes.json")).unwrap();
    assert_eq!(hash(&bytes), hashes["observations.json"]);
    for name in ["fresh/observations.json", "faithful-fresh/faithful-observations.json", "VERIFICATION.json", "batch-manifest.json"] {
        assert_eq!(hash(&fs::read(root.join(name)).unwrap()), hashes[name]);
    }
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    let faithful_bytes = fs::read(root.join("faithful-observations.json")).unwrap();
    assert_eq!(hash(&faithful_bytes), hashes["faithful-observations.json"]);
    let faithful: Value = serde_json::from_slice(&faithful_bytes).unwrap();
    let mut count = 0;
    for row in data["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let gray = row["model"] == "gray";
        let wide = id.contains("f64");
        let model = if gray { ColorMode::Grayscale } else { ColorMode::Rgb };
        let sample = if wide { SampleType::F64 } else { SampleType::F32 };
        assert!(row["assigned_icc"].is_null());
        assert_eq!(row["canvas"], serde_json::json!([16, 12]));
        assert_eq!(row["precision"], if wide { "GIMP_PRECISION_DOUBLE_NON_LINEAR" } else { "GIMP_PRECISION_FLOAT_NON_LINEAR" });
        let file = fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&file), row["source_sha256"]);
        assert_eq!(&file[..14], b"gimp xcf v012\0");
        let mut doc = Document::new(id, Size::new(16, 12), model, sample);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(if gray { photocraft_cms::Builtin::SGray.profile().to_bytes() } else { photocraft_cms::Builtin::Srgb.profile().to_bytes() });
        doc.working_profile_interpretation = if gray {
            WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
        } else {
            WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
        };
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: if wide { 750 } else { 650 } });
        let mut state = XcfState::default();
        state.provenance.format_version = 12;
        assert_eq!(row["layers"].as_array().unwrap().len(), 2);
        for (i, l) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
            assert_eq!(l["mode"], 28);
            assert_eq!(l["byte_order"], "little");
            assert_eq!(l["visible"], true);
            let rect = bounds(l);
            let raw = read(&root, &l["native"]);
            let ne = native(&raw, wide);
            let mut layer = Layer::raster(l["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), rect, &ne).unwrap();
            layer.opacity = (l["opacity"].as_f64().unwrap() / 100.) as f32;
            layer.blend = BlendMode::Normal;
            let op = if i == 0 { l["spaces"]["composite_mode"].as_i64().unwrap() as i32 } else { 1 };
            let operator = match op {
                1 => GimpOperator::Union,
                2 => GimpOperator::ClipToBackdrop,
                3 => GimpOperator::ClipToLayer,
                4 => GimpOperator::Intersection,
                _ => panic!("unknown authored operator"),
            };
            if i == 0 {
                assert_eq!(l["spaces"]["blend_space"], 0);
                assert_eq!(l["spaces"]["composite_space"], 1);
                assert_eq!(l["size"], serde_json::json!([10, 9]));
                let m = &l["mask"];
                assert_eq!(m["offset"], l["offset"]);
                assert_eq!(m["size"], l["size"]);
                assert_eq!(m["buffer_extent"], serde_json::json!([0, 0, 10, 9]));
                assert_eq!(m["owner_apply"], true);
                assert_eq!(m["owner_show"], false);
                assert_eq!(m["owner_edit"], false);
                // Measure outside behavior from the public buffer's expanded read. The
                // canonical world bounds derive from actual public mask offsets, not owner inference.
                let halo = read(&root, &m["halo_none"]);
                assert_eq!(halo.len(), 12 * 11 * 8);
                for y in 0..11 {
                    for x in 0..12 {
                        if x == 0 || x == 11 || y == 0 || y == 10 {
                            assert_eq!(f64::from_le_bytes(halo[(y * 12 + x) * 8..(y * 12 + x) * 8 + 8].try_into().unwrap()), 0.0);
                        }
                    }
                }
                let maskraw = read(&root, &m["native"]);
                let maskrect = bounds(m);
                let surface = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, sample, false), maskrect, &native(&maskraw, wide)).unwrap();
                layer.mask = Some(LayerMask { surface: std::sync::Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
                state.items.push(ItemState {
                    item: ItemRef::Mask(MaskId(900)),
                    name: m["name"].as_str().unwrap().into(),
                    custom_name: false,
                    visible: true,
                    locks: ItemLocks::default(),
                    parent: None,
                    source_tattoo: None,
                    source_flags: 0,
                    raster_bounds: None,
                    composition: None,
                    mask: Some(MaskState {
                        owner: ItemRef::Layer(layer.id),
                        bounds: maskrect,
                        offset: [0, 0],
                        outside_f64_bits: 0.0_f64.to_bits(),
                        enabled: true,
                        shown: false,
                        editing: false,
                        linked: true,
                    }),
                });
            } else {
                assert!(l["mask"].is_null());
                assert_eq!(l["size"], serde_json::json!([16, 12]));
                assert_eq!(l["offset"], serde_json::json!([0, 0]));
            }
            state.items.push(ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(rect),
                mask: None,
                composition: Some(GimpComposition {
                    compatibility_epoch: "normal-mask-observation".into(),
                    source_mode: 28,
                    legacy: false,
                    stored_blend_space: 0,
                    stored_composite_space: if i == 0 { 1 } else { -1 },
                    stored_operator: if i == 0 { op } else { -1 },
                    pass_through: false,
                    resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator }),
                }),
            });
            doc.layers.push(layer);
        }
        doc.xcf_state = Some(state);
        doc.validate_xcf_state().unwrap();
        let before = doc.clone();
        let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        accounted_support::verify(&doc, &actual);
        let reference = read(&root, &row["projection"]["nonlinear_double"]);
        let n = if gray { 2 } else { 4 };
        let mut error = 0.0_f64;
        let mut hidden = 0.0_f64;
        for (p, raw) in actual.px.iter().zip(reference.chunks_exact(n * 8)) {
            let a = f64::from_le_bytes(raw[(n - 1) * 8..n * 8].try_into().unwrap());
            error = error.max((p[3] - a).abs());
            for c in 0..if gray { 1 } else { 3 } {
                let v = f64::from_le_bytes(raw[c * 8..c * 8 + 8].try_into().unwrap());
                if a > 0. {
                    error = error.max((p[c] - v).abs());
                } else {
                    hidden = hidden.max((p[c] - v).abs());
                }
                error = error.max((p[c] * p[3] - v * a).abs());
            }
        }
        println!("{id}: native visible/premult {}/255; hidden alpha0 {}/255", error * 255., hidden * 255.);
        assert!(error <= 1. / 65535., "{id}native {error}");
        let faithful_row = faithful["records"].as_array().unwrap().iter().find(|r| r["id"] == row["id"]).unwrap();
        let png = photocraft_codecs::decode(&read(&root, &faithful_row["reference"])).unwrap();
        let mut png_error = 0.0_f64;
        let mut png_hidden = 0.0_f64;
        for y in 0..12 {
            for x in 0..16 {
                let p = actual.px[(y * 16 + x) as usize];
                for c in 0..n {
                    let v = if c == n - 1 { p[3] } else { p[c] };
                    let reference_value = f64::from(png.get(x, y, c));
                    let reference_alpha = f64::from(png.get(x, y, n - 1));
                    if c == n - 1 || reference_alpha > 0.0 {
                        png_error = png_error.max((v - reference_value).abs());
                    } else {
                        png_hidden = png_hidden.max((v - reference_value).abs());
                    }
                    if c != n - 1 {
                        png_error = png_error.max((v * p[3] - reference_value * reference_alpha).abs());
                    }
                }
            }
        }
        println!("{id}: faithful visible/premult PNG {}/255; hidden alpha0 {}/255", png_error * 255., png_hidden * 255.);
        assert!(png_error <= 1. / 65535., "{id}PNG {png_error}");
        assert!(PreparedRenderer::new_with_interrupt(&doc, &Interrupt::cancel_only(&|| true)).is_err());
        assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work"))).is_err());
        let mut unmasked = doc.clone();
        unmasked.layers.iter_mut().find_map(|layer| layer.mask.as_mut()).unwrap().enabled = false;
        unmasked.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|item| item.mask.as_mut()).unwrap().enabled = false;
        let other = try_render(&unmasked, unmasked.bounds(), &mut |_| Ok(())).unwrap();
        assert!(
            actual.px.iter().zip(&other.px).any(|(a, b)| (a[3] - b[3]).abs() > 1.0 / 255.0 || (a[0] * a[3] - b[0] * b[3]).abs() > 1.0 / 255.0),
            "mask materially changes coverage"
        );
        let mut calls = 0;
        assert!(
            try_render(&doc, doc.bounds(), &mut |_| {
                calls += 1;
                if calls > 8 { Err(photocraft_compose::gimp_composite::CompositionError::Invalid("cancel mid render")) } else { Ok(()) }
            })
            .is_err()
        );
        assert_eq!(doc, before);
        count += 1;
    }
    assert_eq!(count, 32);
}

#[test]
#[ignore = "24 MP release Normal/mask kernel measurement; run explicitly"]
fn normal_mask_operator_kernels_24mp() {
    use photocraft_cms::Builtin;
    use photocraft_compose::gimp_composite::WorkingSpace;
    use std::{hint::black_box, time::Instant};
    for (name, profile) in [("RGB", Builtin::Srgb.profile()), ("Gray", Builtin::SGray.profile())] {
        let space = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let backdrop = [0.19, 0.47, 0.81, 0.375];
        let source = [0.81, 0.02, 0.47, 0.75];
        for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
            let start = Instant::now();
            for _ in 0..24_000_000 {
                black_box(space.normal(black_box(backdrop), black_box(source), black_box(0.7125 * 0.375), operator).unwrap());
            }
            println!("24 MP {name} Normal/mask {operator:?}: {:?}", start.elapsed());
        }
    }
}
