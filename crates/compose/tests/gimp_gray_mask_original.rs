#![cfg(feature = "corpus")]
//! Hash-bound authored Gray Normal/Multiply with owner-aligned masks.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    BlendMode, ColorMode, Document, Layer, LayerMask, PixelFormat, Rect, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{ItemRef, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
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
fn authored_gray_masked_normal_and_multiply_match_checked_consumer() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("gimp_gray_mask_hashes.json")).unwrap();
    for (case, source_mode, blend_mode) in [("gray-mode-28", 28, BlendMode::Normal), ("gray-mode-30", 30, BlendMode::Multiply)] {
        let source_operator = 0;
        let operator = if source_mode == 30 { GimpOperator::ClipToBackdrop } else { GimpOperator::Union };
        for phase in ["source", "edit"] {
            let state: Value = serde_json::from_slice(&read(&root, &format!("{case}.fresh-{phase}_state.json"), &hashes)).unwrap();
            assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
            assert!(state["assigned_icc_sha256"].is_null());
            assert!(state["floating"].is_null());
            assert!(state["channels"].as_array().unwrap().is_empty());
            assert!(state["paths"].as_array().unwrap().is_empty());
            let reference = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let w = state["size"][0].as_u64().unwrap() as u32;
            let h = state["size"][1].as_u64().unwrap() as u32;
            let xcf_name = format!("{case}.{}", if phase == "source" { "xcf" } else { "edit.xcf" });
            let xcf = read(&root, &xcf_name, &hashes);
            assert_eq!(&xcf[..14], b"gimp xcf v012\0");
            assert_eq!(state["size"], serde_json::json!([16, 12]));
            assert_eq!(state["layers"].as_array().unwrap().len(), 2);
            for blend in [GimpSpace::LinearRgb] {
                for composite in [GimpSpace::LinearRgb] {
                    for operator in [operator] {
                        let mut doc = Document::new(case, Size::new(w, h), ColorMode::Grayscale, SampleType::F32);
                        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
                        doc.icc_profile = Some(Builtin::SGray.profile().to_bytes());
                        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap());
                        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
                        let mut imported = XcfState::default();
                        imported.provenance.format_version = 12;
                        for (index, item) in state["layers"].as_array().unwrap().iter().enumerate().rev() {
                            assert_eq!(item["mask"].is_null(), index != 0);
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
                                assert_eq!(item["composition"][field], if index == 0 && field == "composite_mode" { source_operator } else { 0 });
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
                            assert_eq!(layer.opacity, if index == 0 { if phase == "source" { 0.7125 } else { 0.4375 } } else { 1.0 });
                            layer.blend = if index == 0 { blend_mode } else { BlendMode::Normal };
                            if index == 0 {
                                let m = &item["mask"];
                                assert_eq!(m["size"], item["size"]);
                                assert_eq!(m["apply"], true);
                                assert_eq!(m["show"], false);
                                // Actual parser and public observation retain the active mask editing flag.
                                assert_eq!(m["edit"], true);
                                let raw = fs::read(root.join(m["file"].as_str().unwrap())).unwrap();
                                let hash: String = Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect();
                                assert_eq!(hash, m["sha256"].as_str().unwrap());
                                let raw: Vec<u8> = raw.as_chunks::<4>().0.iter().flat_map(|p| f32::from_le_bytes(*p).to_ne_bytes()).collect();
                                let mask_surface =
                                    Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), bounds, &raw).unwrap();
                                let mut layer_mask = LayerMask::reveal_all();
                                layer_mask.surface = std::sync::Arc::new(mask_surface);
                                layer.mask = Some(layer_mask);
                                imported.items.push(ItemState {
                                    item: ItemRef::Mask(MaskId(100)),
                                    name: "originalmask".into(),
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
                                        bounds,
                                        offset: [0, 0],
                                        outside_f64_bits: 0.0f64.to_bits(),
                                        enabled: true,
                                        shown: false,
                                        editing: true,
                                        linked: true,
                                    }),
                                });
                            }
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
                                    stored_blend_space: if index == 0 && source_mode == 30 { -1 } else { 0 },
                                    stored_composite_space: -1,
                                    stored_operator: if index == 0 && source_mode == 30 { -2 } else { -1 },
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
                        let mut worst = 0.0_f64;
                        for y in 0..h {
                            for x in 0..w {
                                for c in 0..4 {
                                    let reference_channel = if c == 3 { 1 } else { 0 };
                                    worst = worst.max((actual.px[(y * w + x) as usize][c] - f64::from(reference.get(x, y, reference_channel))).abs());
                                }
                            }
                        }
                        println!("{phase} blend{blend:?} composite{composite:?} {operator:?}: {}/255", worst * 255.0);
                        assert!(worst <= 1.0 / 65535.0, "{worst}");
                    }
                }
            }
        }
    }
}
