//! Independent GIMP-authored oracles. Missing fixtures are an error when enabled.
#![cfg(feature = "corpus")]
use photocraft_doc::{Layer, LayerContent, Rect, SampleType};
use serde_json::Value;
use std::path::{Path, PathBuf};
fn directory() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_CORPUS").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf"))
}
fn input(root: &Path, name: &str) -> Vec<u8> {
    std::fs::read(root.join(name)).unwrap_or_else(|e| panic!("XCF corpus missing {name}: {e}; run cargo xtask corpus --xcf"))
}
fn flattened(layers: &[Layer]) -> Vec<&Layer> {
    let mut v = Vec::new();
    for l in layers.iter().rev() {
        v.push(l);
        if let LayerContent::Group(g) = &l.content {
            v.extend(flattened(&g.children));
        }
    }
    v
}
fn parent_name(layers: &[Layer], id: photocraft_doc::LayerId) -> Option<&str> {
    for l in layers {
        if let LayerContent::Group(g) = &l.content {
            if g.children.iter().any(|child| child.id == id) {
                return Some(&l.name);
            }
            if let Some(parent) = parent_name(&g.children, id) {
                return Some(parent);
            }
        }
    }
    None
}
#[test]
fn independent_gimp_structure_samples_and_native_migration() {
    let root = directory();
    let manifest: Value = serde_json::from_slice(&input(&root, "manifest.json")).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert!(cases.len() >= 63, "XCF corpus case count fell below the verified floor of 63");
    let mut supported = 0;
    let mut rejected = 0;
    let mut upgraded = 0;
    for case in cases {
        let name = case["input"].as_str().unwrap();
        let bytes = input(&root, name);
        let result = photocraft_io::import(name, &bytes);
        // This pinned manifest predates exact wide samples/two-axis DPI and
        // qualified Linear Invert. Keep
        // its five historical boundary cases, and verify the newly supported
        // ones against their original native oracles instead of requiring an
        // obsolete rejection or a lossy f32 interpretation.
        let newly_supported =
            matches!(name, "unsupported-u32_non_linear.xcf" | "unsupported-double_linear.xcf" | "unsupported-unequal-dpi.xcf" | "unsupported-gegl-effect.xcf");
        if case["expected_status"] == "unsupported" && !newly_supported {
            assert!(result.is_err(), "{name} should reject");
            rejected += 1;
            continue;
        }
        let result = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = &result.document;
        supported += 1;
        if newly_supported {
            upgraded += 1;
        }
        assert!(result.origin.is_read_only());
        assert_eq!(doc.size.width, u32::try_from(case["width"].as_u64().unwrap()).unwrap());
        assert_eq!(doc.size.height, u32::try_from(case["height"].as_u64().unwrap()).unwrap());
        assert_eq!(doc.resolution_dpi, case["dpi"][0].as_f64().unwrap() as f32);
        assert_eq!(doc.effective_resolution().x_dpi, case["dpi"][0].as_f64().unwrap());
        assert_eq!(doc.effective_resolution().y_dpi, case["dpi"][1].as_f64().unwrap());
        let layers = flattened(&doc.layers);
        let expected = case["layers"].as_array().unwrap();
        assert_eq!(layers.len(), expected.len(), "{name}: layer count");
        for (layer, oracle) in layers.iter().zip(expected) {
            assert_eq!(layer.name, oracle["name"].as_str().unwrap(), "{name}: layer order");
            assert_eq!(layer.is_group(), oracle["group"].as_bool().unwrap());
            assert_eq!(layer.visible, oracle["visible"].as_bool().unwrap());
            assert!((layer.opacity - oracle["opacity"].as_f64().unwrap() as f32).abs() < 1e-6);
            assert_eq!(parent_name(&doc.layers, layer.id), oracle["parent"].as_str(), "{name}: hierarchy");
            for (key, actual) in [("lock_content", layer.locks.pixels), ("lock_position", layer.locks.position), ("lock_alpha", layer.locks.transparency)] {
                if let Some(expected) = oracle[key].as_bool() {
                    assert_eq!(actual, expected, "{name}: {key}");
                }
            }
            if oracle["mask"].is_object() {
                let mask = layer.mask.as_ref().unwrap_or_else(|| panic!("{name}: missing mask"));
                let expected = &oracle["mask"];
                assert_eq!(mask.enabled, expected["enabled"].as_bool().unwrap(), "{name}: mask enabled");
                let x = expected["offset"][0].as_i64().unwrap() as i32;
                let y = expected["offset"][1].as_i64().unwrap() as i32;
                let w = expected["width"].as_u64().unwrap() as i32;
                let h = expected["height"].as_u64().unwrap() as i32;
                let original = input(&root, expected["native_sample_file"].as_str().unwrap());
                assert!(mask.surface.to_interleaved(Rect::new(x, y, x + w, y + h)) == original, "{name}: exact mask source");
            } else {
                assert!(layer.mask.is_none(), "{name}: unexpected mask");
            }
            let Some(sample_file) = oracle["sample_file"].as_str() else { continue };
            let raw = input(&root, sample_file);
            let LayerContent::Raster(s) = &layer.content else { panic!("{name}: raster expected") };
            let x = oracle["offset"][0].as_i64().unwrap() as i32;
            let y = oracle["offset"][1].as_i64().unwrap() as i32;
            let w = oracle["width"].as_u64().unwrap() as i32;
            let h = oracle["height"].as_u64().unwrap() as i32;
            let r = Rect::new(x, y, x + w, y + h);
            if let Some(native_file) = oracle["native_sample_file"].as_str()
                && layer.indexed_source.is_none()
            {
                let original = input(&root, native_file);
                let source_stride = original.len() / ((w * h) as usize);
                let actual = s.to_interleaved(r);
                let target_stride = s.format().bytes_per_pixel();
                let is_half = oracle["native_sample_precision"].as_str().unwrap_or("").contains("HALF");
                assert!(source_stride <= target_stride, "{name}: native stride");
                for (i, source) in original.chunks_exact(source_stride).enumerate() {
                    if is_half {
                        let actual = &actual[i * target_stride..i * target_stride + source_stride * 2];
                        for (a, b) in
                            source.as_chunks::<2>().0.iter().map(|chunk| chunk.as_slice()).zip(actual.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()))
                        {
                            let expected = widen_half(u16::from_le_bytes(a.try_into().unwrap()));
                            assert_eq!(f32::from_ne_bytes(b.try_into().unwrap()).to_bits(), expected.to_bits(), "{name}: exact native F16/alpha {i}");
                        }
                        continue;
                    }
                    let actual = &actual[i * target_stride..i * target_stride + source_stride];
                    if cfg!(target_endian = "little") {
                        assert!(source == actual, "{name}: exact native sample/alpha {i}");
                    } else {
                        let bps = s.format().sample.bytes();
                        for (a, b) in source.chunks_exact(bps).zip(actual.chunks_exact(bps)) {
                            assert!(a.iter().rev().copied().eq(b.iter().copied()), "{name}: exact native sample/alpha {i}");
                        }
                    }
                }
                // The native source buffer is authoritative. A converted GIMP R' float
                // buffer may round perceptual/profile encodings through another TRC.
                continue;
            }
            if let Some(indices) = layer.indexed_source.as_ref() {
                let stride = raw.len() / indices.indices().len();
                assert!((1..=2).contains(&stride));
                for (i, index) in indices.indices().iter().enumerate() {
                    assert_eq!(*index, raw[i * stride], "{name}: source index {i}");
                    if let Some(alpha) = indices.alpha() {
                        assert_eq!(alpha[i], raw[i * stride + 1], "{name}: source alpha {i}");
                    }
                }
                assert!(indices.current_matches(s, &doc.color_table.as_ref().unwrap().colors, r));
                continue;
            }
            let format = oracle["sample_format"].as_str().unwrap();
            let channels = if format.starts_with("Y") { 1 } else { 3 };
            let values: Vec<_> = raw.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).map(|b| f32::from_ne_bytes(b.try_into().unwrap())).collect();
            let actual = s.to_interleaved(r);
            let n = s.format().channels();
            let bps = s.format().sample.bytes();
            assert_eq!(values.len(), (w * h) as usize * channels, "{name}: oracle dimensions");
            for (i, expected) in values.chunks_exact(channels).enumerate() {
                for (c, expected) in expected.iter().enumerate() {
                    let bytes = &actual[(i * n + c) * bps..(i * n + c + 1) * bps];
                    match doc.depth {
                        SampleType::U8 => assert_eq!(bytes[0], (expected * 255.0).round() as u8, "{name} sample {i}/{c}"),
                        SampleType::U16 => {
                            assert_eq!(u16::from_ne_bytes(bytes.try_into().unwrap()), (expected * 65535.0).round() as u16, "{name} sample {i}/{c}")
                        }
                        SampleType::F32 => assert_eq!(f32::from_ne_bytes(bytes.try_into().unwrap()).to_bits(), expected.to_bits(), "{name} sample {i}/{c}"),
                        SampleType::U32 | SampleType::F64 => {
                            panic!("{name}: this legacy f32 oracle cannot prove exact wide samples; supply a native U32/F64 oracle")
                        }
                    }
                }
            }
        }
        if let Some(channels) = case["channels"].as_array() {
            assert_eq!(doc.channels.len(), channels.len(), "{name}: saved channels");
            for (actual, oracle) in doc.channels.iter().zip(channels) {
                assert_eq!(actual.name, oracle["name"].as_str().unwrap(), "{name}: channel name");
                let w = oracle["width"].as_u64().unwrap() as i32;
                let h = oracle["height"].as_u64().unwrap() as i32;
                let original = input(&root, oracle["native_sample_file"].as_str().unwrap());
                assert!(actual.surface.to_interleaved(Rect::new(0, 0, w, h)) == original, "{name}: exact saved-channel source");
            }
        }
        let native = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(doc.layers, reopened.layers, "{name}: native layers");
        assert_eq!(doc.icc_profile, reopened.icc_profile);
        assert_eq!(doc.sample_encoding, reopened.sample_encoding);
        assert_eq!(doc.source_color_context, reopened.source_color_context);
        assert_eq!(doc.selection, reopened.selection);
        assert_eq!(doc.channels, reopened.channels);
        assert_eq!(doc.guides, reopened.guides);
        assert_eq!(doc.effective_resolution(), reopened.effective_resolution());
        assert_eq!(doc.xcf_state, reopened.xcf_state);
    }
    eprintln!("GIMP XCF corpus: {supported} supported, {rejected} explicitly rejected");
    assert!(supported >= 62, "supported XCF cases fell below the verified floor of 62");
    assert_eq!(upgraded, 4, "exact wide/DPI/Linear Invert boundary cases must remain supported");
    assert!(rejected + upgraded >= 5, "historical XCF boundary case coverage fell below five");
}
fn widen_half(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exponent = u32::from((h >> 10) & 31);
    let fraction = u32::from(h & 1023);
    let bits = if exponent == 0 {
        if fraction == 0 {
            sign
        } else {
            let highest = 31 - fraction.leading_zeros();
            sign | ((highest + 103) << 23) | ((fraction << (23 - highest)) & 0x7f_ffff)
        }
    } else if exponent == 31 {
        sign | 0x7f80_0000 | (fraction << 13)
    } else {
        sign | ((exponent + 112) << 23) | (fraction << 13)
    };
    f32::from_bits(bits)
}
