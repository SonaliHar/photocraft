#![cfg(feature = "corpus")]
//! Saved Dissolve endpoints and explicit rejection of unqualified partial patterns.
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
    let sha: String = Sha256::digest(&b).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(sha, blob["sha256"].as_str().unwrap());
    b
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn independent_saved_dissolve_endpoints_match_and_partial_patterns_stay_closed() {
    let full = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let root = full.join("dissolve-evidence");
    let hashes: Value = serde_json::from_str(include_str!("gimp_dissolve_hashes.json")).unwrap();
    let raw = fs::read(root.join("observations.json")).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(hash, hashes["observations.json"].as_str().unwrap());
    let observations: Value = serde_json::from_slice(&raw).unwrap();
    let mut endpoints = 0;
    let mut rejected = 0;
    for row in observations["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert_eq!(row["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
        assert!(row["assigned_icc"].is_null());
        let source = if id.starts_with("original-") { full.join("composition-matrix") } else { root.clone() }.join(row["source_file"].as_str().unwrap());
        let file = fs::read(source).unwrap();
        let hash: String = Sha256::digest(&file).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(hash, row["source_sha256"].as_str().unwrap());
        assert_eq!(&file[..14], b"gimp xcf v012\0");
        let w = row["canvas"][0].as_u64().unwrap() as u32;
        let h = row["canvas"][1].as_u64().unwrap() as u32;
        let mut doc = Document::new(id, Size::new(w, h), ColorMode::Rgb, SampleType::F32);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
        let mut state = XcfState::default();
        state.provenance.format_version = 12;
        assert_eq!(row["layers"].as_array().unwrap().len(), 2);
        for (index, l) in row["layers"].as_array().unwrap().iter().enumerate().rev() {
            assert_eq!(l["native_format"], "R'G'B'A float");
            assert_eq!(l["byte_order"], "little");
            assert_eq!(l["mode"], if index == 0 { 1 } else { 28 });
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
            layer.blend = if index == 0 { BlendMode::Dissolve } else { BlendMode::Normal };
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
                    compatibility_epoch: "dissolve-endpoint-observation".into(),
                    source_mode: if index == 0 { 1 } else { 28 },
                    legacy: false,
                    stored_blend_space: 0,
                    stored_composite_space: if index == 0 { 0 } else { -1 },
                    stored_operator: -1,
                    pass_through: false,
                    resolved: Some(ResolvedGimpComposition {
                        blend_space: GimpSpace::ProfileRgb,
                        composite_space: GimpSpace::ProfileRgb,
                        operator: GimpOperator::Union,
                    }),
                }),
            });
            doc.layers.push(layer);
        }
        doc.xcf_state = Some(state);
        doc.validate_xcf_state().unwrap();
        let before = doc.clone();
        let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(()));
        if id.starts_with("alpha-0-") || id.starts_with("alpha-1000-") {
            let actual = actual.unwrap();
            let reference = read(&root, &row["projection"]["nonlinear_double"]);
            let mut worst = 0_f64;
            for (pixel, r) in actual.px.iter().zip(reference.as_chunks::<32>().0.iter()) {
                for c in 0..4 {
                    let v = f64::from_le_bytes(r[c * 8..c * 8 + 8].try_into().unwrap());
                    worst = worst.max((pixel[c] - v).abs());
                }
            }
            assert_eq!(worst, 0.0);
            endpoints += 1;
        } else {
            assert!(actual.is_err(), "partial coverage must not use another random hash: {id}");
            rejected += 1;
        }
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(PreparedRenderer::new_with_interrupt(&doc, &cancelled).is_err());
        assert!(try_render(&doc, doc.bounds(), &mut |_| Err(photocraft_compose::gimp_composite::CompositionError::Invalid("denied work"))).is_err());
        assert_eq!(doc, before);
    }
    assert_eq!((endpoints, rejected), (4, 16));
}
