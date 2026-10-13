#![cfg(feature = "corpus")]
//! Independent public GIMP floating appearance, separate from anchoring edits.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    ColorMode, Document, Layer, Rect, SampleEncoding, SampleType, Size,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::ItemRef,
    xcf_editing::FloatingAttachment,
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn authored_floating_normal_appearance_matches_ten_rgb_gray_native_depths() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("floating-anchor-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, "e1893a1601c94f745f9f445db5d865a90ab1f2f0cfae7fbf0cee8f2696e7f523");
    let packet: Value = serde_json::from_slice(&bytes).unwrap();
    let read = |descriptor: &Value| {
        let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash, descriptor["sha256"].as_str().unwrap());
        bytes
    };
    let mut compared = 0;
    for record in packet["records"].as_array().unwrap() {
        let id = record["id"].as_str().unwrap();
        let mode = if id.starts_with("gray-") { ColorMode::Grayscale } else { ColorMode::Rgb };
        let sample = if id.contains("-u8_") {
            SampleType::U8
        } else if id.contains("-u16_") {
            SampleType::U16
        } else if id.contains("-u32_") {
            SampleType::U32
        } else if id.contains("-float_") {
            SampleType::F32
        } else {
            SampleType::F64
        };
        let builtin = if mode == ColorMode::Rgb { Builtin::Srgb } else { Builtin::SGray };
        let state = &record["source_reopened"];
        assert!(!state["floating"].is_null());
        assert!(state["profiles"]["assigned"].is_null());
        let mut doc = Document::new(id, Size::new(8, 6), mode, sample);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(builtin.profile().to_bytes());
        doc.working_profile_interpretation = if mode == ColorMode::Rgb {
            photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
        } else {
            photocraft_doc::WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
        };
        let mut canonical = XcfState::default();
        for item in state["layers"].as_array().unwrap().iter().rev() {
            assert_eq!(item["mode"], 28);
            assert_eq!(item["composite_space"], 2);
            assert_eq!(item["composite_mode"], 1);
            assert_eq!(item["has_mask"], false);
            assert_eq!(item["filter_count"], 0);
            let bounds = Rect::from_xywh(
                item["offset"][0].as_i64().unwrap() as i32,
                item["offset"][1].as_i64().unwrap() as i32,
                item["size"][0].as_u64().unwrap() as u32,
                item["size"][1].as_u64().unwrap() as u32,
            );
            let raw = read(&item["native"]);
            let raw: Vec<u8> = match sample {
                SampleType::U8 => raw,
                SampleType::U16 => raw.as_chunks::<2>().0.iter().flat_map(|p| u16::from_le_bytes(*p).to_ne_bytes()).collect(),
                SampleType::U32 | SampleType::F32 => raw.as_chunks::<4>().0.iter().flat_map(|p| u32::from_le_bytes(*p).to_ne_bytes()).collect(),
                SampleType::F64 => raw.as_chunks::<8>().0.iter().flat_map(|p| u64::from_le_bytes(*p).to_ne_bytes()).collect(),
            };
            let mut layer = Layer::raster(item["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &raw).unwrap();
            canonical.items.push(ItemState {
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
                    compatibility_epoch: "qualified-floating-normal-profile".into(),
                    source_mode: 28,
                    legacy: false,
                    stored_blend_space: 0,
                    stored_composite_space: 2,
                    stored_operator: 1,
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
        canonical.floating = Some(FloatingAttachment { layer: doc.layers[1].id, target: ItemRef::Layer(doc.layers[0].id) });
        doc.xcf_state = Some(canonical);
        doc.validate_xcf_state().unwrap();
        let before = doc.clone();
        let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        let expected = read(&state["projection"]);
        let mut worst = 0.0_f64;
        for (pixel, raw) in actual.px.iter().zip(expected.as_chunks::<32>().0.iter()) {
            let expected_alpha = f64::from_le_bytes(raw[24..32].try_into().unwrap());
            for (c, value) in pixel.iter().enumerate() {
                if c < 3 && expected_alpha == 0.0 && pixel[3] == 0.0 {
                    continue;
                }
                worst = worst.max((value - f64::from_le_bytes(raw[c * 8..c * 8 + 8].try_into().unwrap())).abs());
            }
        }
        // Public new_from_visible projection is a derived display oracle;
        // original wide native samples above remain exact and immutable.
        let display_tolerance = match sample {
            SampleType::U8 => 0.501 / 255.0 + 1e-6,
            SampleType::U16 => 0.501 / 65535.0 + 1e-6,
            _ => 1e-6,
        };
        assert!(worst <= display_tolerance, "{id}: {worst}");
        println!("floating {id}: native-derived projection max {worst}");
        assert_eq!(doc, before);
        compared += 1;
    }
    assert_eq!(compared, 10);
}
