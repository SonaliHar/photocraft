#![cfg(feature = "corpus")]
//! Modern HSV/HSL references through actual import, source retention and native reopen.
use photocraft_doc::{
    Rect,
    import_compositing::{GimpOperator, GimpSpace},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn read(root: &Path, name: &str, hashes: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(name)).unwrap();
    assert_eq!(hash(&bytes), hashes[name]);
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished modern HSV/HSL source/edit witnesses"]
fn hsv_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_hsv_hashes.json"),
        &[("hsv-hue", 37), ("hsv-saturation", 38), ("hsl-color", 39), ("hsv-value", 40)],
        -2,
        GimpSpace::ProfileRgb,
    );
}
#[test]
#[ignore = "requires reviewed unpublished modern LCh source/edit witnesses"]
fn lch_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_lch_hashes.json"),
        &[("lch-hue", 24), ("lch-chroma", 25), ("lch-color", 26), ("lch-lightness", 27)],
        -3,
        GimpSpace::Lab,
    );
}
#[test]
#[ignore = "requires reviewed unpublished modern luminance source/edit witnesses"]
fn luminance_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_luma_hashes.json"),
        &[("luma-darken-only", 54), ("luma-lighten-only", 55), ("luminance", 56)],
        -1,
        GimpSpace::LinearRgb,
    );
}
#[test]
#[ignore = "requires reviewed unpublished legacy HSV/HSL source/edit witnesses"]
fn legacy_hsv_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_legacy_hsv_hashes.json"),
        &[("hsv-hue-legacy", 11), ("hsv-saturation-legacy", 12), ("hsl-color-legacy", 13), ("hsv-value-legacy", 14)],
        -2,
        GimpSpace::ProfileRgb,
    );
}
#[test]
#[ignore = "requires reviewed unpublished deterministic alpha source/edit witnesses"]
fn alpha_blends_import_native_source_and_independent_projection() {
    component_pipeline(include_str!("../../compose/tests/gimp_alpha_hashes.json"), &[("erase", 58), ("merge", 59), ("split", 60)], 0, GimpSpace::ProfileRgb);
}
#[test]
#[ignore = "requires reviewed unpublished Color Erase source/edit witnesses"]
fn color_erase_import_native_source_and_independent_projection() {
    component_pipeline(include_str!("../../compose/tests/gimp_color_erase_hashes.json"), &[("color-erase", 57)], -1, GimpSpace::LinearRgb);
}
#[test]
#[ignore = "requires reviewed unpublished legacy Grain source/edit witnesses"]
fn legacy_grain_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_legacy_grain_hashes.json"),
        &[("grain-extract-legacy", 20), ("grain-merge-legacy", 21)],
        -2,
        GimpSpace::ProfileRgb,
    );
}
#[test]
#[ignore = "requires reviewed unpublished legacy ratio/Hard Light witnesses"]
fn legacy_ratios_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_legacy_contrast_hashes.json"),
        &[("divide-legacy", 15), ("dodge-legacy", 16), ("burn-legacy", 17), ("hardlight-legacy", 18)],
        -2,
        GimpSpace::ProfileRgb,
    );
}
#[test]
#[ignore = "requires reviewed unpublished modern Grain source/edit witnesses"]
fn modern_grain_import_native_source_and_independent_projection() {
    component_pipeline(
        include_str!("../../compose/tests/gimp_basic_blends_hashes.json"),
        &[("grain-extract", 46), ("grain-merge", 47)],
        -4,
        GimpSpace::PerceptualRgb,
    );
}
fn component_pipeline(manifest: &str, cases: &[(&str, i32)], blend_raw: i32, blend_space: GimpSpace) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("composition-matrix");
    let hashes: Value = serde_json::from_str(manifest).unwrap();
    for &(case, mode) in cases {
        let legacy = matches!(mode, 11..=18 | 20 | 21);
        let composite_raw = if legacy {
            -2
        } else if mode == 60 {
            0
        } else {
            -1
        };
        let composite_space = if legacy || mode == 60 { GimpSpace::ProfileRgb } else { GimpSpace::LinearRgb };
        let operator_raw = if mode == 59 { -1 } else { -2 };
        let operator = if mode == 59 { GimpOperator::Union } else { GimpOperator::ClipToBackdrop };
        for phase in ["source", "edit"] {
            let path = format!("mode-{case}.{}", if phase == "source" { "xcf" } else { "edit.xcf" });
            let bytes = read(&root, &path, &hashes);
            let doc = photocraft_io::import("hsv.xcf", &bytes).unwrap().document;
            let before = doc.clone();
            let observed: Value = serde_json::from_slice(&read(&root, &format!("mode-{case}.fresh-{phase}_state.json"), &hashes)).unwrap();
            let state = doc.xcf_state.as_ref().unwrap();
            for (layer, original) in doc.layers.iter().rev().zip(observed["layers"].as_array().unwrap()) {
                let native = fs::read(root.join(original["native"]["file"].as_str().unwrap())).unwrap();
                assert_eq!(hash(&native), original["native"]["sha256"]);
                let native: Vec<u8> = native
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| chunk.as_slice())
                    .flat_map(|v| u32::from_le_bytes(v.try_into().unwrap()).to_ne_bytes())
                    .collect();
                let x = original["offset"][0].as_i64().unwrap() as i32;
                let y = original["offset"][1].as_i64().unwrap() as i32;
                let extent = Rect::from_xywh(x, y, original["size"][0].as_u64().unwrap() as u32, original["size"][1].as_u64().unwrap() as u32);
                assert_eq!(layer.surface().unwrap().try_read_interleaved(extent).unwrap(), native);
                if original["composition"]["mode"] == mode {
                    let policy =
                        state.items.iter().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id)).unwrap().composition.as_ref().unwrap();
                    assert_eq!(
                        (policy.source_mode, policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator),
                        (mode, blend_raw, composite_raw, operator_raw)
                    );
                    assert_eq!(policy.legacy, legacy);
                    let resolved = policy.resolved.as_ref().unwrap();
                    assert_eq!((resolved.blend_space, resolved.composite_space, resolved.operator), (blend_space, composite_space, operator));
                }
            }
            let saved = photocraft_io::export(&doc, "hsv.pcraft", &Default::default()).unwrap();
            let reopened = photocraft_io::import("hsv.pcraft", &saved.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&doc, "hsv.pcraft"), reopened);
            let reference = photocraft_codecs::decode(&read(&root, &format!("mode-{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            for candidate in [&doc, &reopened] {
                let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
                for (index, pixel) in rendered.px.iter().enumerate() {
                    for (channel, value) in pixel.iter().enumerate() {
                        assert!(
                            // PNG clips its display projection; retain unrestricted
                            // authoritative f64 render values and source bytes above.
                            (value.clamp(0.0, 1.0) - f64::from(reference.get(index as u32 % 16, index as u32 / 16, channel))).abs() <= 2.0 / 255.0,
                            "{case}/{phase}/{index}/{channel}"
                        );
                    }
                }
            }
            assert_eq!(doc, before);
            assert_eq!(fs::read(root.join(path)).unwrap(), bytes);
        }
    }
}
