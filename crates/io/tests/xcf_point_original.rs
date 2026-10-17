#![cfg(feature = "corpus")]
//! Hash-bound GIMP originals through the real parser, mapper, native store and exports.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn read(root: &Path, file: &str, hashes: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}; set PHOTOCRAFT_XCF_FULL_CORPUS to the full corpus"));
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, hashes[file].as_str().unwrap(), "{file}");
    bytes
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_legacy_blends_import_reopen_and_render() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_legacy_basic_hashes.json")).unwrap();
    let arithmetic: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_legacy_arithmetic_hashes.json")).unwrap();
    let contrast: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_legacy_contrast_hashes.json")).unwrap();
    for (case, mode) in [
        ("multiply-legacy", 3),
        ("screen-legacy", 4),
        ("difference-legacy", 6),
        ("addition-legacy", 7),
        ("subtract-legacy", 8),
        ("darken-only-legacy", 9),
        ("lighten-only-legacy", 10),
        ("softlight-legacy", 19),
        ("overlay-legacy", 19),
    ] {
        let hashes = if mode <= 4 {
            &hashes
        } else if mode <= 10 {
            &arithmetic
        } else {
            &contrast
        };
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let source = photocraft_io::import("legacy.xcf", &read(&root, &format!("mode-{case}.{suffix}"), hashes)).unwrap().document;
            let before = source.clone();
            let policy =
                source.xcf_state.as_ref().unwrap().items.iter().filter_map(|item| item.composition.as_ref()).find(|policy| policy.source_mode == mode).unwrap();
            assert!(policy.legacy);
            assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (-2, -2, -2));
            let native = photocraft_io::export(&source, "legacy.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty());
            let reopened = photocraft_io::import("legacy.pcraft", &native.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&source, "legacy.pcraft"), reopened);
            let expected = photocraft_codecs::decode(&read(&root, &format!("mode-{case}.fresh-{phase}.ref.png"), hashes)).unwrap();
            let mut worst = 0.0_f64;
            for doc in [&source, &reopened] {
                let actual = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                for (pixel, index) in actual.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        worst = worst.max((value - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                    }
                }
            }
            assert!(worst <= 1.0 / 65535.0, "{case}/{phase}: {}/255", worst * 255.0);
            assert_eq!(source, before);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_multiply_explicit_spaces_import_reopen_and_render() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_multiply_spaces_hashes.json")).unwrap();
    for case in
        ["blend-space-rgb-non-linear", "blend-space-rgb-perceptual", "composite-space-rgb-non-linear", "composite-space-rgb-perceptual", "composite-space-lab"]
    {
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let input = read(&root, &format!("{case}.{suffix}"), &hashes);
            let source = photocraft_io::import("spaces.xcf", &input).unwrap().document;
            let before = source.clone();
            let expected = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let native = photocraft_io::export(&source, "spaces.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty(), "{:?}", native.warnings);
            let reopened = photocraft_io::import("spaces.pcraft", &native.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&source, "spaces.pcraft"), reopened);
            let mut worst = 0.0_f64;
            for doc in [&source, &reopened] {
                let actual = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                if case == "composite-space-lab" && phase == "edit" {
                    assert!(actual.px.iter().any(|pixel| pixel.iter().take(3).any(|v| *v < 0.0)), "Lab composite must retain its out-of-gamut floating result");
                }
                for (pixel, index) in actual.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        assert!(value.is_finite());
                        // The independent PNG clips for display; the authoritative
                        // floating result and source/native samples remain unclipped.
                        worst = worst.max((value.clamp(0.0, 1.0) - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                    }
                }
            }
            assert!(worst <= 1.0 / 65535.0, "{case}/{phase}: {}/255", worst * 255.0);
            assert_eq!(source, before);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_masked_groups_import_reopen_and_render() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_group_hashes.json")).unwrap();
    for mode in [28, 61] {
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let case = format!("group-{mode}-none-mask");
            let input = read(&root, &format!("{case}.{suffix}"), &hashes);
            let parsed = photocraft_xcf::decode(&input, &Default::default()).unwrap();
            for d in &parsed.layers {
                println!(
                    "group{mode}/{phase}: {} group{} path{:?} mode{} tuple{}/{}/{} bounds{}x{} {:?} mask{:?} flags{}/{}/{}",
                    d.name,
                    d.group,
                    d.path,
                    d.mode,
                    d.blend_space,
                    d.composite_space,
                    d.composite_mode,
                    d.width,
                    d.height,
                    d.offset,
                    d.mask.as_ref().map(|m| (m.width, m.height, m.offset)),
                    d.mask_enabled,
                    d.mask_shown,
                    d.mask_editing
                );
            }
            let source = photocraft_io::import("group.xcf", &input).unwrap().document;
            let before = source.clone();
            let expected = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let native = photocraft_io::export(&source, "group.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty(), "{:?}", native.warnings);
            let reopened = photocraft_io::import("group.pcraft", &native.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&source, "group.pcraft"), reopened);
            let mut worst = 0.0_f64;
            for doc in [&source, &reopened] {
                let actual = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                for (pixel, index) in actual.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        worst = worst.max((value - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                    }
                }
            }
            assert!(worst <= 1.0 / 65535.0, "{case}/{phase}: {}/255", worst * 255.0);
            assert_eq!(source, before);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_gaussian_groups_import_reopen_and_render() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_gaussian_group_hashes.json")).unwrap();
    for mode in [28, 61] {
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let case = format!("group-{mode}-group-mask");
            let bytes = read(&root, &format!("{case}.{suffix}"), &hashes);
            if mode == 61 {
                assert!(photocraft_io::import("filtered-group.xcf", &bytes).is_err(), "pass-through filter input remains unqualified");
                continue;
            }
            let doc = photocraft_io::import("filtered-group.xcf", &bytes).unwrap().document;
            let before = doc.clone();
            assert_eq!(doc.xcf_state.as_ref().unwrap().filter_stacks.len(), 1);
            let native = photocraft_io::export(&doc, "filtered-group.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty());
            let reopened = photocraft_io::import("filtered-group.pcraft", &native.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&doc, "filtered-group.pcraft"), reopened);
            let expected = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            for source in [&doc, &reopened] {
                let actual = photocraft_compose::gimp_render::try_render(source, source.bounds(), &mut |_| Ok(())).unwrap();
                let mut worst = 0.0_f64;
                for (pixel, index) in actual.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        worst = worst.max((value - f64::from(expected.get(index % source.size.width, index / source.size.width, channel))).abs());
                    }
                }
                assert!(worst <= 1.0 / 65535.0, "{case}/{phase}: {}/255", worst * 255.0);
            }
            assert_eq!(doc, before);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_gray_masked_normal_and_multiply_import_and_reopen() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_gray_mask_hashes.json")).unwrap();
    for case in ["gray-mode-28", "gray-mode-30"] {
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let input = read(&root, &format!("{case}.{suffix}"), &hashes);
            let parsed = photocraft_xcf::decode(&input, &Default::default()).unwrap();
            println!("{case}/{phase} v{} {:?} {:?} {:?}", parsed.version, parsed.model, parsed.sample, parsed.encoding);
            for d in &parsed.layers {
                println!(
                    "{} mode{} tuple{}/{}/{} path{:?} bounds{}x{} {:?} opacity{:?} mask{:?} flags{}/{}/{} plane{:?}",
                    d.name,
                    d.mode,
                    d.blend_space,
                    d.composite_space,
                    d.composite_mode,
                    d.path,
                    d.width,
                    d.height,
                    d.offset,
                    d.opacity,
                    d.mask.as_ref().map(|m| (m.width, m.height, m.offset, m.model, m.alpha, m.plane.as_ref().map(|p| (p.sample, p.channels)))),
                    d.mask_enabled,
                    d.mask_shown,
                    d.mask_editing,
                    d.plane.as_ref().map(|p| (p.sample, p.channels))
                );
            }
            let source = photocraft_io::import("gray.xcf", &input).unwrap().document;
            let expected = photocraft_codecs::decode(&read(&root, &format!("{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let before = source.clone();
            let native = photocraft_io::export(&source, "gray.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty(), "{:?}", native.warnings);
            let reopened = photocraft_io::import("gray.pcraft", &native.bytes).unwrap().document;
            assert_eq!(*photocraft_io::named_for_file(&source, "gray.pcraft"), reopened);
            let mut worst = 0.0_f64;
            for doc in [&source, &reopened] {
                let actual = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                for (pixel, index) in actual.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        let reference_channel = if channel == 3 { 1 } else { 0 };
                        worst = worst.max((value - f64::from(expected.get(index % doc.size.width, index / doc.size.width, reference_channel))).abs());
                    }
                }
            }
            assert!(worst <= 1.0 / 65535.0, "{case}/{phase}: {}/255", worst * 255.0);
            assert_eq!(source, before);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_basic_blends_import_reopen_and_render() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_basic_blends_hashes.json")).unwrap();
    for case in [
        "screen",
        "difference",
        "addition",
        "subtract",
        "exclusion",
        "overlay",
        "linear-light",
        "hardlight",
        "darken-only",
        "lighten-only",
        "softlight",
        "pin-light",
        "dodge",
        "burn",
        "vivid-light",
        "linear-burn",
        "divide",
        "hard-mix",
    ] {
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let input = read(&root, &format!("mode-{case}.{suffix}"), &hashes);
            let mut outside_domain = input.clone();
            outside_domain[14..18].copy_from_slice(&17_u32.to_be_bytes());
            assert!(photocraft_io::import("outside-domain.xcf", &outside_domain).is_err());
            let source = photocraft_io::import("blend.xcf", &input).unwrap().document;
            let before = source.clone();
            let expected = photocraft_codecs::decode(&read(&root, &format!("mode-{case}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let native = photocraft_io::export(&source, "blend.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty(), "{:?}", native.warnings);
            let reopened = photocraft_format::load_from_bytes(&native.bytes).unwrap();
            assert_eq!(source.layers, reopened.layers);
            assert_eq!(source.xcf_state, reopened.xcf_state);
            let mut worst = 0.0_f64;
            for doc in [&source, &reopened] {
                let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                for (actual, index) in rendered.px.iter().zip(0_u32..) {
                    for (channel, value) in actual.iter().enumerate() {
                        worst = worst.max((value.clamp(0.0, 1.0) - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                    }
                }
            }
            println!("{case}/{phase} parser/native/render max {}/255", worst * 255.0);
            assert!(worst <= 2.0 / 65535.0);
            assert_eq!(source, before);
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_point_filters_import_reopen_and_export_without_source_substitution() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_point_hashes.json")).unwrap();
    for name in ["gegl-exposure", "gegl-brightness-contrast", "gegl-invert-linear", "gegl-invert-gamma", "gegl-levels"] {
        let base = format!("operations/{name}/operation-{name}");
        check(&root, &hashes, name, &base);
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_gaussian_filters_import_reopen_and_export_without_source_substitution() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_gaussian_hashes.json")).unwrap();
    check(&root, &hashes, "gegl-gaussian-blur", "filter-owner-layer");
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_free256_curves_import_reopen_and_export_without_source_substitution() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_curves_hashes.json")).unwrap();
    check(&root, &hashes, "gimp-curves", "curve-config-matrix/free256-rgb-u8");
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_structured_levels_import_reopen_and_color_managed_render() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"));
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_levels_hashes.json")).unwrap();
    for trc in [0, 1] {
        for clamps in ["00", "01", "10", "11"] {
            for (suffix, reference, stage) in [("xcf", "ref.png", "source"), ("edit.xcf", "edit.ref.png", "edit-source")] {
                let base = format!("curve-config-matrix/levels-trc{trc}-clamps{clamps}");
                let encoded = read(&root, &format!("{base}.{suffix}"), &hashes);
                let source = photocraft_io::import("levels.xcf", &encoded).unwrap().document;
                let before = source.clone();
                let raw = read(&root, &format!("{base}.{stage}.0.native.bin"), &hashes);
                let raw: Vec<_> = raw
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| chunk.as_slice())
                    .flat_map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()).to_ne_bytes())
                    .collect();
                assert_eq!(source.layers[0].surface().unwrap().try_read_interleaved(source.bounds()).unwrap(), raw);
                let expected = photocraft_codecs::decode(&read(&root, &format!("{base}.{reference}"), &hashes)).unwrap();
                let display = expected
                    .icc
                    .as_ref()
                    .map(|bytes| photocraft_cms::Profile::parse(bytes).unwrap())
                    .unwrap_or_else(|| photocraft_cms::Builtin::Srgb.profile().clone());
                let stored = photocraft_cms::Profile::parse(source.icc_profile.as_ref().unwrap()).unwrap();
                let transform = photocraft_cms::Transform::new(&stored, &display, photocraft_cms::Intent::RelativeColorimetric, true).unwrap();
                let saved = photocraft_io::export(&source, "levels.pcraft", &Default::default()).unwrap();
                assert!(saved.warnings.is_empty(), "{:?}", saved.warnings);
                let reopened = photocraft_format::load_from_bytes(&saved.bytes).unwrap();
                assert_eq!(reopened.layers, source.layers);
                assert_eq!(reopened.xcf_state, source.xcf_state);
                let mut worst = 0.0_f64;
                for doc in [&source, &reopened] {
                    let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                    for (pixel, index) in rendered.px.iter().zip(0_u32..) {
                        let mut rgb = [0.0; 3];
                        transform.try_eval_f64(&pixel[..3], &mut rgb).unwrap();
                        for (channel, value) in rgb.into_iter().chain([pixel[3]]).enumerate() {
                            worst = worst.max((value.clamp(0.0, 1.0) - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                        }
                    }
                }
                println!("Levels trc{trc} clamps{clamps}/{suffix} parser→native→display {}/255", worst * 255.0);
                assert!(worst <= 2.0 / 255.0);
                assert_eq!(source, before);
            }
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_modern_multiply_auto_imports_and_reopens_with_resolved_policy() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_multiply_hashes.json")).unwrap();
    for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
        let input = read(&root, &format!("mode-multiply.{suffix}"), &hashes);
        let mut outside_domain = input.clone();
        outside_domain[14..18].copy_from_slice(&17_u32.to_be_bytes());
        assert!(photocraft_io::import("outside-domain.xcf", &outside_domain).is_err());
        let parsed = photocraft_xcf::decode(&input, &Default::default()).unwrap();
        println!(
            "Multiply/{phase} epoch{} model{:?} sample{:?} encoding{:?} icc{} paths{} channels{}",
            parsed.version,
            parsed.model,
            parsed.sample,
            parsed.encoding,
            parsed.icc.len(),
            parsed.paths.len(),
            parsed.channels.len()
        );
        for layer in &parsed.layers {
            println!(
                "layer mode{} path{:?} size{}x{} offset{:?} opacity{:?} spaces{}/{}/{}",
                layer.mode, layer.path, layer.width, layer.height, layer.offset, layer.opacity, layer.blend_space, layer.composite_space, layer.composite_mode
            );
        }
        let source = photocraft_io::import("multiply.xcf", &input).unwrap().document;
        let before = source.clone();
        let expected = photocraft_codecs::decode(&read(&root, &format!("mode-multiply.fresh-{phase}.ref.png"), &hashes)).unwrap();
        let native = photocraft_io::export(&source, "multiply.pcraft", &Default::default()).unwrap();
        assert!(native.warnings.is_empty(), "{:?}", native.warnings);
        let reopened = photocraft_format::load_from_bytes(&native.bytes).unwrap();
        assert_eq!(source.layers, reopened.layers);
        assert_eq!(source.xcf_state, reopened.xcf_state);
        let mut worst = 0.0_f64;
        for doc in [&source, &reopened] {
            let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            for (actual, index) in rendered.px.iter().zip(0_u32..) {
                for (channel, value) in actual.iter().enumerate() {
                    worst = worst.max((*value - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                }
            }
        }
        println!("Multiply/{phase} parser/native/render max {}/255", worst * 255.0);
        assert!(worst <= 2.0 / 65535.0);
        assert_eq!(source, before);
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_offset_masks_and_operators_import_and_reopen() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_mask_hashes.json")).unwrap();
    for name in ["operator-union", "operator-clip-to-backdrop", "operator-clip-to-layer", "operator-intersection"] {
        for (suffix, phase) in [("xcf", "source"), ("edit.xcf", "edit")] {
            let input = read(&root, &format!("{name}.{suffix}"), &hashes);
            let parsed = photocraft_xcf::decode(&input, &Default::default()).unwrap();
            println!("{name}/{phase} epoch{} {:?}/{:?} icc{}", parsed.version, parsed.sample, parsed.encoding, parsed.icc.len());
            for l in &parsed.layers {
                println!(
                    "layer mode{} size{}x{} offset{:?} opacity{} tuple{}/{}/{} mask {:?}",
                    l.mode,
                    l.width,
                    l.height,
                    l.offset,
                    l.opacity,
                    l.blend_space,
                    l.composite_space,
                    l.composite_mode,
                    l.mask.as_ref().map(|m| (m.width, m.height, m.offset))
                );
            }
            let source = photocraft_io::import(name, &input).unwrap().document;
            let expected = photocraft_codecs::decode(&read(&root, &format!("{name}.fresh-{phase}.ref.png"), &hashes)).unwrap();
            let native = photocraft_io::export(&source, "mask.pcraft", &Default::default()).unwrap();
            assert!(native.warnings.is_empty(), "{:?}", native.warnings);
            let reopened = photocraft_format::load_from_bytes(&native.bytes).unwrap();
            assert_eq!(source.layers, reopened.layers);
            assert_eq!(source.xcf_state, reopened.xcf_state);
            let mut worst = 0.0_f64;
            for doc in [&source, &reopened] {
                let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
                for (pixel, index) in rendered.px.iter().zip(0_u32..) {
                    for (channel, value) in pixel.iter().enumerate() {
                        worst = worst.max((*value - f64::from(expected.get(index % doc.size.width, index / doc.size.width, channel))).abs());
                    }
                }
            }
            assert!(worst <= 2.0 / 65535.0, "{name}/{phase} error {worst}");
        }
    }
}

fn check(root: &Path, hashes: &Value, name: &str, base: &str) {
    let raw = read(root, &format!("{base}.source.0.native.bin"), hashes);
    for (suffix, reference_suffix) in [("xcf", "ref.png"), ("edit.xcf", "edit.ref.png")] {
        let encoded = read(root, &format!("{base}.{suffix}"), hashes);
        if name == "gimp-curves" {
            let parsed = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
            for argument in &parsed.layers[0].effects[0].arguments {
                println!(
                    "Curves {}/{} typed {:?}",
                    suffix,
                    argument.name,
                    argument.configuration.as_ref().and_then(|c| c.typed.as_ref()).map(|c| match c {
                        photocraft_xcf::TypedConfiguration::Curves(c) => (c.time_bits, c.trc.as_deref(), c.channels.len()),
                        _ => (None, None, 0),
                    })
                );
            }
        }
        let imported = photocraft_io::import(&format!("{name}.xcf"), &encoded).unwrap();
        if name == "gimp-curves" {
            let mut outside_domain = encoded.clone();
            outside_domain[14..18].copy_from_slice(&13_u32.to_be_bytes());
            assert!(photocraft_io::import("outside-domain.xcf", &outside_domain).is_err());
        }
        let source = imported.document.clone();
        assert_eq!(source.layers[0].surface().unwrap().try_read_interleaved(source.bounds()).unwrap(), raw);
        let state = source.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks.len(), 1);
        assert_eq!(state.filter_stacks[0].nodes.len(), 1);
        assert_eq!(state.filter_masks.len(), 1);
        assert_eq!(state.filter_stacks[0].nodes[0].mask, Some(photocraft_doc::item_identity::ItemRef::Mask(state.filter_masks[0].id)));
        assert_eq!(state.filter_stacks[0].nodes[0].operation.name, name.replacen('-', ":", 1));
        let expected = photocraft_codecs::decode(&read(root, &format!("{base}.{reference_suffix}"), hashes)).unwrap();
        let saved = photocraft_io::export(&source, "result.pcraft", &Default::default()).unwrap();
        assert!(saved.warnings.is_empty(), "{:?}", saved.warnings);
        let reopened = photocraft_format::load_from_bytes(&saved.bytes).unwrap();
        assert_eq!(reopened.xcf_state, source.xcf_state);
        for doc in [&source, &reopened] {
            let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            let flat = photocraft_io::document_to_image(doc, &mut Vec::new()).unwrap();
            let mut worst = 0.0_f64;
            for y in 0..doc.size.height {
                for x in 0..doc.size.width {
                    let pixel = &rendered.px[(y * doc.size.width + x) as usize];
                    for (c, component) in pixel.iter().enumerate() {
                        let reference = f64::from(expected.get(x, y, c));
                        worst = worst.max((component.clamp(0.0, 1.0) - reference).abs());
                        let actual = if c == 3 && !flat.layout().has_alpha() { 1.0 } else { f64::from(flat.get(x, y, c)) };
                        assert!((actual - reference).abs() <= 2.0 / 255.0, "{name}/{suffix} flat ({x},{y},{c})");
                    }
                }
            }
            println!("{name}/{suffix}: parser→native→consumer max {}/255", worst * 255.0);
            assert!(worst <= 2.0 / 255.0);
        }
        if state.filter_stacks[0].nodes[0].enabled {
            assert!(photocraft_io::export(&source, "result.psd", &Default::default()).is_err());
        }
        assert_eq!(imported.document, source);
    }
}
