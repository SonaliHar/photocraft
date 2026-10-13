#![cfg(feature = "corpus")]
//! Independent saved Gray alpha/color modes, with no RGB/profile inference.
use photocraft_compose::gimp_render::{PreparedRenderer, try_render};
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, Rect, SampleEncoding, SampleType, Size, SourceColorContext, WorkingProfileInterpretation,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn read(root: &Path, blob: &Value) -> Vec<u8> {
    let b = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(b.len(), blob["bytes"].as_u64().unwrap() as usize);
    let h: String = Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(h, blob["sha256"].as_str().unwrap());
    b
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn independently_authored_gray_alpha_color_modes_match_prepared_consumer() {
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let root = full.join("alpha-gray-evidence");
    let raw = fs::read(root.join("observations.json")).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect();
    let expected: Value = serde_json::from_str(include_str!("gimp_alpha_gray_hashes.json")).unwrap();
    assert_eq!(hash, expected["observations.json"].as_str().unwrap());
    let observations: Value = serde_json::from_slice(&raw).unwrap();
    let mut cases = 0;
    for row in observations["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert_eq!(row["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
        assert!(row["assigned_icc"].is_null());
        assert_eq!(row["canvas"], serde_json::json!([16, 12]));
        let bytes = fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(hash, row["source_sha256"].as_str().unwrap());
        assert_eq!(&bytes[..14], b"gimp xcf v012\0");
        let mut doc = Document::new(id, Size::new(16, 12), ColorMode::Grayscale, SampleType::F32);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::SGray.profile().to_bytes());
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap());
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
        let mut state = XcfState::default();
        state.provenance.format_version = 12;
        assert_eq!(row["layers"].as_array().unwrap().len(), 2);
        for (index, l) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
            assert_eq!(l["native_format"], "Y'A float");
            assert_eq!(l["byte_order"], "little");
            let mode = l["mode"].as_i64().unwrap() as i32;
            assert!(if index == 0 { (57..=60).contains(&mode) } else { mode == 28 });
            let b = read(&root, &l["native"]);
            for p in b.as_chunks::<4>().0.iter() {
                let v = f32::from_le_bytes(*p);
                assert!(v.is_finite() && (0.0..=1.0).contains(&v));
            }
            let native: Vec<u8> = b.as_chunks::<4>().0.iter().flat_map(|v| f32::from_le_bytes(*v).to_ne_bytes()).collect();
            let bounds = Rect::from_xywh(
                l["offset"][0].as_i64().unwrap() as i32,
                l["offset"][1].as_i64().unwrap() as i32,
                l["size"][0].as_u64().unwrap() as u32,
                l["size"][1].as_u64().unwrap() as u32,
            );
            let mut layer = Layer::raster(l["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &native).unwrap();
            layer.opacity = (l["opacity"].as_f64().unwrap() / 100.) as f32;
            layer.blend = BlendMode::Normal;
            let (stored_blend, stored_composite, stored_operator, blend, composite, operator) = match mode {
                57 => (-1, -1, -2, GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop),
                58 => (0, -1, -2, GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop),
                59 => (0, -1, -1, GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::Union),
                60 => (0, 0, -2, GimpSpace::ProfileRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToBackdrop),
                _ => (0, -1, -1, GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::Union),
            };
            state.items.push(ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(bounds),
                mask: None,
                composition: Some(GimpComposition {
                    compatibility_epoch: "gray-alpha-observation".into(),
                    source_mode: mode,
                    legacy: false,
                    stored_blend_space: stored_blend,
                    stored_composite_space: stored_composite,
                    stored_operator,
                    pass_through: false,
                    resolved: Some(ResolvedGimpComposition { blend_space: blend, composite_space: composite, operator }),
                }),
            });
            doc.layers.push(layer);
        }
        doc.xcf_state = Some(state);
        doc.validate_xcf_state().unwrap();
        let before = doc.clone();
        let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let reference = read(&root, &row["projection"]["nonlinear_double"]);
        let mut worst = 0_f64;
        let mut hidden_error = 0_f64;
        for (pixel, r) in actual.px.iter().zip(reference.as_chunks::<16>().0.iter()) {
            let y = f64::from_le_bytes(r[..8].try_into().unwrap());
            let alpha = f64::from_le_bytes(r[8..].try_into().unwrap());
            for sample in pixel.iter().take(3) {
                if alpha == 0.0 {
                    hidden_error = hidden_error.max((sample - y).abs());
                } else {
                    worst = worst.max((sample - y).abs());
                }
                worst = worst.max((sample * pixel[3] - y * alpha).abs());
            }
            worst = worst.max((pixel[3] - alpha).abs());
        }
        println!("{id}: visible and premultiplied {}/255; alpha0 hidden RGB {}/255", worst * 255., hidden_error * 255.);
        assert!(worst <= 1. / 65535.);
        let reference = photocraft_codecs::decode(&read(&root, &row["reference"])).unwrap();
        let mut png_error = 0_f64;
        for y in 0..12 {
            for x in 0..16 {
                let pixel = actual.px[(y * 16 + x) as usize];
                png_error = png_error.max((pixel[0] - f64::from(reference.get(x, y, 0))).abs());
                png_error = png_error.max((pixel[3] - f64::from(reference.get(x, y, 1))).abs());
            }
        }
        println!("{id}: native Gray PNG YA {}/255", png_error * 255.);
        assert!(png_error <= 1. / 65535.);
        cases += 1;
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(PreparedRenderer::new_with_interrupt(&doc, &cancelled).is_err());
        assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work"))).is_err());
        assert_eq!(doc, before);
    }
    assert_eq!(cases, 8);
}
