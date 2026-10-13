#![cfg(feature = "corpus")]
//! Hash-bound authored modern basic blends; exact stored Auto mappings.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, Rect, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn read(root: &std::path::Path, name: &str, hashes: &Value) -> Vec<u8> {
    let raw = fs::read(root.join(name)).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, hashes[name].as_str().unwrap());
    raw
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn authored_multiply_explicit_spaces_match_checked_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("gimp_multiply_spaces_hashes.json")).unwrap();
    for (case, epoch, stored_blend, stored_composite, blend, composite) in [
        ("blend-space-rgb-non-linear", 12, 2, -1, GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
        ("blend-space-rgb-perceptual", 23, 4, -1, GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
        ("composite-space-rgb-non-linear", 12, -1, 2, GimpSpace::LinearRgb, GimpSpace::ProfileRgb),
        ("composite-space-rgb-perceptual", 23, -1, 4, GimpSpace::LinearRgb, GimpSpace::PerceptualRgb),
        ("composite-space-lab", 12, -1, 3, GimpSpace::LinearRgb, GimpSpace::Lab),
    ] {
        let source_mode = 30;
        let blend_mode = BlendMode::Multiply;
        for phase in ["source", "edit"] {
            let state: Value = serde_json::from_slice(&read(&root, &format!("{case}.fresh-{phase}_state.json"), &hashes)).unwrap();
            assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            assert!(state["assigned_icc_sha256"].is_null());
            let reference = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let w = state["size"][0].as_u64().unwrap() as u32;
            let h = state["size"][1].as_u64().unwrap() as u32;
            let xcf_name = format!("{case}.{}", if phase == "source" { "xcf" } else { "edit.xcf" });
            let xcf = read(&root, &xcf_name, &hashes);
            assert_eq!(&xcf[..14], format!("gimp xcf v{epoch:03}\0").as_bytes());
            assert_eq!(state["size"], serde_json::json!([16, 12]));
            assert_eq!(state["layers"].as_array().unwrap().len(), 2);
            let operator = GimpOperator::ClipToBackdrop;
            let mut doc = Document::new(case, Size::new(w, h), ColorMode::Rgb, SampleType::F32);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
            doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
            doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
            let mut imported = XcfState::default();
            imported.provenance.format_version = epoch;
            for (index, item) in state["layers"].as_array().unwrap().iter().enumerate().rev() {
                assert!(item["mask"].is_null());
                assert!(item["parent"].is_null());
                assert!(item["filters"].as_array().unwrap().is_empty());
                assert_eq!(item["visible"], true);
                assert_eq!(item["size"], if index == 0 { serde_json::json!([10, 9]) } else { serde_json::json!([16, 12]) });
                assert_eq!(
                    item["offset"],
                    if index == 0 {
                        if phase == "source" { serde_json::json!([2, -1]) } else { serde_json::json!([-1, 2]) }
                    } else {
                        serde_json::json!([0, 0])
                    }
                );
                assert_eq!(item["composition"]["mode"], if index == 0 { source_mode } else { 28 });
                for field in ["blend_space", "composite_space", "composite_mode"] {
                    assert_eq!(
                        item["composition"][field],
                        if index == 0 && field == "blend_space" && stored_blend > 0 {
                            stored_blend
                        } else if index == 0 && field == "composite_space" && stored_composite > 0 {
                            stored_composite
                        } else {
                            0
                        }
                    );
                }
                let x = item["offset"][0].as_i64().unwrap() as i32;
                let y = item["offset"][1].as_i64().unwrap() as i32;
                let bounds = Rect::from_xywh(x, y, item["size"][0].as_u64().unwrap() as u32, item["size"][1].as_u64().unwrap() as u32);
                let bytes = fs::read(root.join(item["native"]["file"].as_str().unwrap())).unwrap();
                let native_hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(native_hash, item["native"]["sha256"].as_str().unwrap());
                assert_eq!(item["native"]["byte_order"], "little");
                let bytes: Vec<u8> = bytes.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect();
                let mut layer = Layer::raster(item["name"].as_str().unwrap(), doc.pixel_format());
                *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &bytes).unwrap();
                layer.opacity = (item["opacity"].as_f64().unwrap() / 100.0) as f32;
                layer.blend = if index == 0 { blend_mode } else { BlendMode::Normal };
                imported.items.push(ItemState {
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
                        compatibility_epoch: "authored-Auto".into(),
                        source_mode: if index == 0 { source_mode } else { 28 },
                        legacy: false,
                        stored_blend_space: if index == 0 { stored_blend } else { 0 },
                        stored_composite_space: if index == 0 { stored_composite } else { -1 },
                        stored_operator: if index == 0 { -2 } else { -1 },
                        pass_through: false,
                        resolved: Some(ResolvedGimpComposition {
                            blend_space: if index == 0 { blend } else { GimpSpace::LinearRgb },
                            composite_space: if index == 0 { composite } else { GimpSpace::LinearRgb },
                            operator: if index == 0 { operator } else { GimpOperator::Union },
                        }),
                    }),
                });
                doc.layers.push(layer);
            }
            doc.xcf_state = Some(imported);
            doc.validate_xcf_state().unwrap();
            let before = doc.clone();
            let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(doc, before);
            if case == "composite-space-lab" && phase == "edit" {
                let min = actual.px.iter().flat_map(|pixel| pixel[..3].iter()).copied().fold(f64::INFINITY, f64::min);
                println!("Lab edit authoritative minimum {min} ({} /255)", min * 255.0);
                assert!(min < -0.01 && min.is_finite(), "Lab out-of-gamut result must remain unclipped");
            }
            let projection_error = |pixels: &[[f64; 4]]| {
                let mut worst = 0.0_f64;
                for y in 0..h {
                    for x in 0..w {
                        for (c, sample) in pixels[(y * w + x) as usize].iter().enumerate() {
                            // PNG is bounded; authoritative HDR is not clipped.
                            worst = worst.max((sample.clamp(0.0, 1.0) - f64::from(reference.get(x, y, c))).abs());
                        }
                    }
                }
                worst
            };
            let worst = projection_error(&actual.px);
            println!("{case}/{phase}: {}/255", worst * 255.0);
            assert!(worst <= 1.0 / 65535.0, "{case}/{phase}: {worst}");
            if stored_blend == 0 {
                // Monotone channel-wise transfer keeps min/max ordering. These
                // witnesses cannot uniquely identify the zero/Auto blend space.
                for candidate in [GimpSpace::LinearRgb, GimpSpace::PerceptualRgb] {
                    let mut alternate = doc.clone();
                    alternate.xcf_state.as_mut().unwrap().items.last_mut().unwrap().composition.as_mut().unwrap().resolved.as_mut().unwrap().blend_space =
                        candidate;
                    let other = try_render(&alternate, alternate.bounds(), &mut |_| Ok(())).unwrap();
                    assert!(projection_error(&other.px) <= 1.0 / 65535.0);
                    let delta = other.px.iter().zip(&actual.px).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0_f64, f64::max);
                    assert!(delta < 1e-12, "min/max transfer tie: {delta}");
                }
                let mut wrong = doc.clone();
                wrong.xcf_state.as_mut().unwrap().items.last_mut().unwrap().composition.as_mut().unwrap().resolved.as_mut().unwrap().composite_space =
                    GimpSpace::ProfileRgb;
                let other = try_render(&wrong, wrong.bounds(), &mut |_| Ok(())).unwrap();
                assert!(projection_error(&other.px) > 1.0 / 255.0, "nonlinear composite should differ");
                let mut wrong = doc.clone();
                wrong.xcf_state.as_mut().unwrap().items.last_mut().unwrap().composition.as_mut().unwrap().resolved.as_mut().unwrap().operator =
                    GimpOperator::Union;
                let other = try_render(&wrong, wrong.bounds(), &mut |_| Ok(())).unwrap();
                assert!(projection_error(&other.px) > 1.0 / 255.0, "Union coverage should differ");
                assert_eq!(doc, before);
            }
        }
    }
}
