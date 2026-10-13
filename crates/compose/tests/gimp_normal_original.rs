#![cfg(feature = "corpus")]
//! Authored Normal/Auto source/edit evidence, not universal Auto resolution proof.
use photocraft_cms::Builtin;
use photocraft_compose::gimp_render::try_render;
use photocraft_doc::{ColorMode, Document, ImportCompositing, Layer, Rect, SampleEncoding, SampleType, Size, SourceColorContext};
use photocraft_raster::Surface;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn authored_normal_auto_source_and_edit_rgba_match_qualified_spaces() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    assert!(root.is_dir(), "stage verified full XCF corpus or set PHOTOCRAFT_XCF_FULL_CORPUS; publication/pin remains a separate gate");
    for (name, phase, mode, working, state_hash, reference_hash) in [
        (
            "mode-normal",
            "source",
            28,
            SampleEncoding::Linear,
            "0029a07cb179e5f23c6f2f4bc11ee79f0f5b4c46b690d90aa52eca6482a44f0c",
            "8904b88277967ad41ced1c0f20142c2a78131e3178da4fbdc71324ce88cf7951",
        ),
        (
            "mode-normal",
            "edit",
            28,
            SampleEncoding::Linear,
            "0feebb02fc74668506f441941b1ec7478b5facf143716c1d710d63c7ddcac161",
            "d8124386132a04084c6043378bef2bccd9f010376b2bfddd6cc9eb77fb77f4df",
        ),
        (
            "mode-normal-legacy",
            "source",
            0,
            SampleEncoding::NonlinearProfile,
            "3d298587cd2b3f1106d859ce089d6fb98cbbcc64d424d330be93257888cb0f89",
            "55e027f32a0acf58fefe2199ce7914ea34fcab958817dd659f4337fa9ae72d81",
        ),
        (
            "mode-normal-legacy",
            "edit",
            0,
            SampleEncoding::NonlinearProfile,
            "488a9910d75ee9859dc503ea8087341c3809ec0e0cea65930ca8b8a9fc9f0113",
            "65bf142eb49bd36071ffc2d9b55c1f5ff636dc195469c8fd143eaa25cee7fa64",
        ),
    ] {
        let raw = fs::read(root.join(format!("{name}.fresh-{phase}_state.json"))).unwrap();
        assert_eq!(sha256(&raw), state_hash);
        let state: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
        assert!(state["assigned_icc_sha256"].is_null());
        let w = state["size"][0].as_u64().unwrap() as u32;
        let h = state["size"][1].as_u64().unwrap() as u32;
        let raw = fs::read(root.join(format!("{name}.fresh-{phase}.ref.png"))).unwrap();
        assert_eq!(sha256(&raw), reference_hash);
        let reference = photocraft_codecs::decode(&raw).unwrap();
        assert_eq!(reference.dimensions(), (w, h));
        assert_eq!(reference.layout(), photocraft_codecs::ChannelLayout::Rgba);
        assert_eq!(reference.sample_type(), photocraft_codecs::SampleType::U16);
        let mut doc = Document::new(name, Size::new(w, h), ColorMode::Rgb, SampleType::F32);
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
        for (index, item) in state["layers"].as_array().unwrap().iter().enumerate().rev() {
            assert!(item["parent"].is_null());
            assert!(item["mask"].is_null());
            assert_eq!(item["composition"]["mode"], if index == 0 { mode } else { 28 });
            for field in ["blend_space", "composite_space", "composite_mode"] {
                assert_eq!(item["composition"][field], 0);
            }
            let x = item["offset"][0].as_i64().unwrap() as i32;
            let y = item["offset"][1].as_i64().unwrap() as i32;
            let lw = item["size"][0].as_u64().unwrap() as u32;
            let lh = item["size"][1].as_u64().unwrap() as u32;
            let raw = fs::read(root.join(item["native"]["file"].as_str().unwrap())).unwrap();
            assert_eq!(sha256(&raw), item["native"]["sha256"].as_str().unwrap());
            assert_eq!(item["native"]["byte_order"], "little");
            // Native artifacts are little endian; canonical Surface storage is host endian.
            let native: Vec<_> = raw.as_chunks::<4>().0.iter().flat_map(|bytes| f32::from_le_bytes(*bytes).to_ne_bytes()).collect();
            let mut layer = Layer::raster(item["name"].as_str().unwrap(), doc.pixel_format());
            *layer.surface_mut().unwrap() = Surface::try_from_interleaved(doc.pixel_format(), Rect::from_xywh(x, y, lw, lh), &native).unwrap();
            layer.opacity = (item["opacity"].as_f64().unwrap() / 100.0) as f32;
            layer.import_compositing = Some(ImportCompositing { blend_encoding: working, composite_encoding: working, pass_through: false });
            doc.layers.push(layer);
        }
        let before = doc.clone();
        let rendered = try_render(&doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(doc, before);
        let mut worst = 0.0_f64;
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    worst = worst.max((rendered.px[(y * w + x) as usize][c] - f64::from(reference.get(x, y, c))).abs());
                }
            }
        }
        // One 16-bit code accommodates ICC fixed-point transfer coefficients and PNG quantization.
        assert!(worst <= 1.0 / 65535.0, "{name}/{phase}: worst {worst} ({}/255)", worst * 255.0);
    }
}
