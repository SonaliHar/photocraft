#![cfg(feature = "corpus")]
//! Hash-bound legacy HSV/HSL originals with independent min-alpha overlap coverage.
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
fn authored_legacy_hsv_blends_match_checked_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("gimp_legacy_hsv_hashes.json")).unwrap();
    for (case, source_mode, blend_mode, epoch, stored_blend) in [
        ("hsv-hue-legacy", 11, BlendMode::Hue, 12, -2),
        ("hsv-saturation-legacy", 12, BlendMode::Saturation, 12, -2),
        ("hsl-color-legacy", 13, BlendMode::Color, 12, -2),
        ("hsv-value-legacy", 14, BlendMode::Luminosity, 12, -2),
    ] {
        for phase in ["source", "edit"] {
            let state: Value = serde_json::from_slice(&read(&root, &format!("mode-{case}.fresh-{phase}_state.json"), &hashes)).unwrap();
            assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            assert!(state["assigned_icc_sha256"].is_null());
            let reference = photocraft_codecs::decode(&read(&root, &format!("mode-{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let w = state["size"][0].as_u64().unwrap() as u32;
            let h = state["size"][1].as_u64().unwrap() as u32;
            let xcf_name = format!("mode-{case}.{}", if phase == "source" { "xcf" } else { "edit.xcf" });
            let xcf = read(&root, &xcf_name, &hashes);
            assert_eq!(&xcf[..14], format!("gimp xcf v{epoch:03}\0").as_bytes());
            assert_eq!(state["size"], serde_json::json!([16, 12]));
            assert_eq!(state["layers"].as_array().unwrap().len(), 2);
            let blend = if stored_blend == -2 {
                GimpSpace::ProfileRgb
            } else if stored_blend == -4 {
                GimpSpace::PerceptualRgb
            } else if stored_blend == 0 {
                GimpSpace::ProfileRgb
            } else {
                GimpSpace::LinearRgb
            };
            let composite = GimpSpace::ProfileRgb;
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
                    assert_eq!(item["composition"][field], 0);
                }
                let x = item["offset"][0].as_i64().unwrap() as i32;
                let y = item["offset"][1].as_i64().unwrap() as i32;
                let bounds = Rect::from_xywh(x, y, item["size"][0].as_u64().unwrap() as u32, item["size"][1].as_u64().unwrap() as u32);
                let bytes = fs::read(root.join(item["native"]["file"].as_str().unwrap())).unwrap();
                let native_hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(native_hash, item["native"]["sha256"].as_str().unwrap());
                assert_eq!(item["native"]["byte_order"], "little");
                for sample in bytes.as_chunks::<4>().0.iter() {
                    let value = f32::from_le_bytes(*sample);
                    assert!(value.is_finite() && (0.0..=1.0).contains(&value));
                }
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
                        legacy: index == 0,
                        stored_blend_space: if index == 0 { stored_blend } else { 0 },
                        stored_composite_space: if index == 0 { -2 } else { -1 },
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
            let mut missing_legacy = doc.clone();
            missing_legacy.xcf_state.as_mut().unwrap().items.last_mut().unwrap().composition.as_mut().unwrap().legacy = false;
            assert!(try_render(&missing_legacy, missing_legacy.bounds(), &mut |_| Ok(())).is_err());
            for (blend_space, composite_space, operator) in [
                (GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToBackdrop),
                (GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop),
                (GimpSpace::ProfileRgb, GimpSpace::ProfileRgb, GimpOperator::Union),
            ] {
                let mut wrong = doc.clone();
                wrong.xcf_state.as_mut().unwrap().items.last_mut().unwrap().composition.as_mut().unwrap().resolved =
                    Some(ResolvedGimpComposition { blend_space, composite_space, operator });
                assert!(try_render(&wrong, wrong.bounds(), &mut |_| Ok(())).is_err());
            }
            assert!(photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&doc, &photocraft_raster::Interrupt::cancel_only(&|| true)).is_err());
            assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work budget"))).is_err());
            assert_eq!(doc, before);
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
