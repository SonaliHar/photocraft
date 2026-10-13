#![cfg(feature = "corpus")]
//! Hidden saved channels and active selection are image-projection-independent.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{
    AlphaChannel, ColorMode, Document, ImportCompositing, Layer, PixelFormat, Rect, SampleEncoding, SampleType, Size,
    item_identity::ItemRef,
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn hidden_saved_channel_edit_preserves_native_and_layer_projection() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("item-state-probe/fresh-process");
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let bytes = fs::read(root.join("proof.json")).unwrap();
    assert_eq!(hash(&bytes), "8c044e383f3fa3261096d7d3d0a62753033caf38d02a01cc62e965edb6d2f8dd");
    let packet: Value = serde_json::from_slice(&bytes).unwrap();
    let read = |descriptor: &Value| {
        let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
        bytes
    };
    let mut compared = 0;
    for case in packet["cases"].as_array().unwrap() {
        if case["id"] != "active-channels-paths-locks" {
            continue;
        }
        for stage in case["stages"].as_array().unwrap() {
            let state = &stage["state"];
            let mut doc = Document::new("hidden channels", Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
            doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
            doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
            doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
            for item in state["layers"].as_array().unwrap().iter().rev() {
                let bounds = Rect::from_xywh(
                    item["offset"][0].as_i64().unwrap() as i32,
                    item["offset"][1].as_i64().unwrap() as i32,
                    item["size"][0].as_u64().unwrap() as u32,
                    item["size"][1].as_u64().unwrap() as u32,
                );
                let mut l = Layer::raster(item["name"].as_str().unwrap(), doc.pixel_format());
                *l.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), bounds, &read(&item["native"])).unwrap();
                l.import_compositing =
                    Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
                doc.layers.push(l);
            }
            let mut canonical = XcfState::default();
            for item in state["channels"].as_array().unwrap() {
                assert_eq!(item["visible"], false);
                let raw = read(&item["native"]);
                let channel = AlphaChannel::new(
                    item["name"].as_str().unwrap(),
                    Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), doc.bounds(), &raw).unwrap(),
                );
                canonical.items.push(ItemState {
                    item: ItemRef::Channel(channel.id),
                    name: channel.name.clone(),
                    custom_name: false,
                    visible: false,
                    locks: ItemLocks::default(),
                    parent: None,
                    source_tattoo: None,
                    source_flags: 0,
                    raster_bounds: Some(doc.bounds()),
                    mask: None,
                    composition: None,
                });
                doc.channels.push(channel);
            }
            doc.xcf_state = Some(canonical);
            let before = doc.clone();
            let actual = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(doc, before);
            let reference_bytes = fs::read(root.join(stage["render"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&reference_bytes), stage["render_sha256"].as_str().unwrap());
            let reference = photocraft_codecs::decode(&reference_bytes).unwrap();
            let mut worst = 0.0_f64;
            for (i, pixel) in actual.px.iter().enumerate() {
                for (c, v) in pixel.iter().enumerate() {
                    worst = worst.max((v - f64::from(reference.get((i % 32) as u32, (i / 32) as u32, c))).abs());
                }
            }
            assert!(worst <= 2.0 / 255.0, "{} {} {worst}", case["id"], stage["stage"]);
            let mut visible = doc.clone();
            visible.xcf_state.as_mut().unwrap().items[0].visible = true;
            assert!(try_render(&visible, visible.bounds(), &mut |_| Ok(())).is_err());
            compared += 1;
        }
    }
    assert_eq!(compared, 2);
}
