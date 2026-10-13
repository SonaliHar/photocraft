//! HDR compatibility controls: retained predecessor previews versus current native save.
use super::*;
use photocraft_doc::{ColorMode, Layer, SampleEncoding, SampleType, Size, WorkingProfileInterpretation};
use std::io::Cursor;
fn limits() -> NativeSaveLimits {
    NativeSaveLimits {
        preparation: PreparationLimits { resident_bytes: 128 << 20, ..Default::default() },
        archive: photocraft_format::zip::stream::Limits { max_metadata_bytes: 1 << 20, ..Default::default() },
    }
}
fn current(mode: ColorMode, depth: SampleType) -> Document {
    let mut doc = Document::new("Own finite HDR native compatibility", Size::new(8, 6), mode, depth);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let bytes = if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb.profile() } else { photocraft_cms::Builtin::SGray.profile() }.to_bytes();
    doc.working_profile_interpretation =
        if mode == ColorMode::Rgb { WorkingProfileInterpretation::implicit_srgb(&bytes) } else { WorkingProfileInterpretation::implicit_sgray(&bytes) };
    doc.icc_profile = Some(bytes);
    doc.layers.push(Layer::raster("Own original HDR samples", doc.pixel_format()));
    hdr_pixels(&mut doc);
    doc
}
fn hdr_pixels(doc: &mut Document) {
    let mode = doc.mode;
    let surface = doc.layers[0].surface_mut().unwrap();
    for y in 0..6 {
        for x in 0..8 {
            let alpha = [0.0, 0.25, 0.75, 1.0][(x + y) as usize % 4];
            let value = if (x + y) % 2 == 0 { -0.125 } else { 1.5 };
            let values: &[f64] = if mode == ColorMode::Rgb { &[value, 1.5, 4.0, alpha] } else { &[value, alpha] };
            surface.try_write_pixel_f64(x, y, values).unwrap();
        }
    }
}
fn pngs(bytes: &[u8]) -> [Vec<u8>; 2] {
    let mut r = photocraft_format::zip::stream::Reader::new(Cursor::new(bytes), limits().archive, &Interrupt::NONE).unwrap();
    [r.read_by_name("thumb.png", 8 << 20, &Interrupt::NONE).unwrap(), r.read_by_name("composite/preview.png", 8 << 20, &Interrupt::NONE).unwrap()]
}
fn old_archive(doc: &Document) -> Vec<u8> {
    let (previews, warnings) = crate::native_previews(doc);
    assert!(warnings.is_empty(), "predecessor preview warnings:{warnings:?}");
    assert!(previews.thumbnail.is_some() && previews.composite.is_some());
    let bytes = photocraft_format::save_to_bytes(doc, &previews).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *doc);
    bytes
}
#[test]
fn finite_hdr_native_save_keeps_predecessor_both_previews_and_exact_source() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::F32, SampleType::F64] {
            let doc = current(mode, depth);
            let before = doc.clone();
            let old = old_archive(&doc);
            let mut output = Cursor::new(Vec::new());
            let result = save_native_stream(&doc, &mut output, limits(), &Interrupt::NONE);
            assert!(result.is_ok(), "existing HDR native-save regression {mode:?}/{depth:?}: {result:?}");
            assert!(result.unwrap().1.warnings.is_empty());
            assert_eq!(pngs(output.get_ref()), pngs(&old));
            assert_eq!(photocraft_format::load_from_bytes(output.get_ref()).unwrap(), before);
            assert_eq!(doc, before);
        }
    }
}
#[test]
#[ignore = "requires reviewed unpublished originals and private capture output"]
fn capture_finite_hdr_native_before_compatibility() {
    use serde_json::json;
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let out = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_HDR_BEFORE_OUTPUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let mut rows = Vec::new();
    let mut capture = |id: String, doc: Document, original: Option<String>, route: &str| {
        let source = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let archive = old_archive(&doc);
        let previews = pngs(&archive);
        let public = save_native_stream(&doc, Cursor::new(Vec::new()), limits(), &Interrupt::NONE);
        let public_error = public.as_ref().err().map(ToString::to_string);
        if route == "actual current public fallback" {
            let (actual, result) = public.as_ref().unwrap();
            assert!(result.warnings.is_empty());
            assert_eq!(pngs(actual.get_ref()), previews);
            assert_eq!(photocraft_format::load_from_bytes(actual.get_ref()).unwrap(), doc);
        }
        let mut paths = Vec::new();
        for (name, png) in ["thumb", "composite"].into_iter().zip(previews) {
            let file = format!("{id}.{name}.png");
            std::fs::write(out.join(&file), &png).unwrap();
            paths.push(json!({"file":file,"sha256":hash(&png)}));
        }
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), source);
        rows.push(json!({"id":id,"baseline_route":route,"original_xcf_sha256":original,"source_native_sha256":hash(&source),"baseline_archive_sha256":hash(&archive),"source_native_exact":true,"depth":format!("{:?}",doc.depth),"model":format!("{:?}",doc.mode),"encoding":format!("{:?}",doc.sample_encoding),"working_profile":doc.working_profile_interpretation,"active_icc_sha256":doc.icc_profile.as_ref().map(|v|hash(v)),"canonical_state_retained":doc.xcf_state.is_some(),"public_error_before":public_error,"previews":paths}));
    };
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::F32, SampleType::F64] {
            capture(format!("current-{mode:?}-{depth:?}"), current(mode, depth), None, "preserved predecessor native_previews plus real format writer");
        }
    }
    for model in ["rgb", "gray"] {
        for phase in ["", ".edit"] {
            let path = root.join(format!("precision-matrix/{model}-float-linear-hdr{phase}.xcf"));
            let bytes = std::fs::read(&path).unwrap();
            let doc = crate::import("own-hdr.xcf", &bytes).unwrap().document;
            capture(format!("intact-{model}-linear-hdr{phase}"), doc, Some(hash(&bytes)), "actual current public fallback");
        }
    }
    for model in ["rgb", "gray"] {
        let path = root.join(format!("{model}-double_non_linear.xcf"));
        let bytes = std::fs::read(&path).unwrap();
        let mut doc = crate::import("own-hdr.xcf", &bytes).unwrap().document;
        let context = doc.xcf_state.clone();
        hdr_pixels(&mut doc);
        assert_eq!(doc.xcf_state, context);
        capture(
            format!("imported-current-{model}-nonlinear-hdr"),
            doc,
            Some(hash(&bytes)),
            "preserved predecessor native_previews plus real format writer; authoritative current HDR edit, intact imported metadata",
        );
    }
    std::fs::write(
        out.join("baseline.json"),
        serde_json::to_vec_pretty(&json!({"status":"HDR compatibility BEFORE, not new source qualification","records":rows})).unwrap(),
    )
    .unwrap();
    println!("captured {} HDR contexts", rows.len());
}

#[test]
#[ignore = "requires reviewed unpublished HDR BEFORE packet and originals"]
fn actual_hdr_native_save_matches_frozen_before_twenty_previews() {
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let packet = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_HDR_BEFORE_OUTPUT").unwrap());
    let raw = std::fs::read(packet.join("baseline.json")).unwrap();
    assert_eq!(hash(&raw), "f7cce88cb77d559cb509b10fae328706d3a68d07c4b0073c722227ebec52886f");
    let baseline: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let mut docs = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::F32, SampleType::F64] {
            docs.push((format!("current-{mode:?}-{depth:?}"), current(mode, depth), None));
        }
    }
    for model in ["rgb", "gray"] {
        for phase in ["", ".edit"] {
            let bytes = std::fs::read(root.join(format!("precision-matrix/{model}-float-linear-hdr{phase}.xcf"))).unwrap();
            let doc = crate::import("own-hdr.xcf", &bytes).unwrap().document;
            docs.push((format!("intact-{model}-linear-hdr{phase}"), doc, Some(hash(&bytes))));
        }
    }
    for model in ["rgb", "gray"] {
        let bytes = std::fs::read(root.join(format!("{model}-double_non_linear.xcf"))).unwrap();
        let mut doc = crate::import("own-hdr.xcf", &bytes).unwrap().document;
        let state = doc.xcf_state.clone();
        hdr_pixels(&mut doc);
        assert_eq!(doc.xcf_state, state);
        docs.push((format!("imported-current-{model}-nonlinear-hdr"), doc, Some(hash(&bytes))));
    }
    assert_eq!(docs.len(), 10);
    for (id, doc, source_hash) in docs {
        let row = baseline["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
        assert_eq!(row["original_xcf_sha256"], serde_json::to_value(source_hash).unwrap());
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let (actual, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits(), &Interrupt::NONE).unwrap();
        assert!(result.warnings.is_empty(), "{id}:{:?}", result.warnings);
        assert_eq!(photocraft_format::load_from_bytes(actual.get_ref()).unwrap(), doc);
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
        for (png, expected) in pngs(actual.get_ref()).into_iter().zip(row["previews"].as_array().unwrap()) {
            let old = std::fs::read(packet.join(expected["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&old), expected["sha256"].as_str().unwrap());
            assert_eq!(png, old, "{id}");
        }
    }
}

#[test]
fn hdr_cross_product_invalid_samples_profile_cancel_and_budget_are_atomic() {
    for case in ["negative-alpha", "over-alpha", "nan-color", "infinite-color", "stale-icc", "cancel", "budget"] {
        let mut doc = current(ColorMode::Rgb, SampleType::F64);
        match case {
            "negative-alpha" => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[1.5, 4., -0.125, -0.25]).unwrap(),
            "over-alpha" => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[1.5, 4., -0.125, 1.25]).unwrap(),
            "nan-color" => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[f64::NAN, 4., -0.125, 1.]).unwrap(),
            "infinite-color" => doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(0, 0, &[f64::INFINITY, 4., -0.125, 1.]).unwrap(),
            "stale-icc" => std::sync::Arc::make_mut(doc.icc_profile.as_mut().unwrap())[0] ^= 1,
            _ => {}
        }
        let before = doc.clone();
        let cancelled = || case == "cancel";
        let control = Interrupt::cancel_only(&cancelled);
        let mut cap = limits();
        if case == "budget" {
            cap.preparation.resident_bytes = 32768;
        }
        let mut output = Cursor::new(Vec::new());
        assert!(save_native_stream(&doc, &mut output, cap, &control).is_err(), "{case}");
        assert!(output.get_ref().is_empty(), "{case}");
        assert_eq!(doc, before, "{case}");
    }
}
