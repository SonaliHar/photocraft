//! Native plain-Normal previews share the input/job budget.
//! Immutable source/native bytes and both derived outputs remain accounted.
use photocraft_doc::{ColorMode, Document, Layer, SampleEncoding, SampleType, Size, WorkingProfileInterpretation};
use photocraft_format::{PreparationLimits, load_from_bytes};
use photocraft_io::{NativeSaveLimits, save_native_stream};
use photocraft_raster::{Interrupt, Surface};
use std::{
    io::Cursor,
    sync::atomic::{AtomicUsize, Ordering},
};

fn current_plain(mode: ColorMode, sample: SampleType, size: Size) -> Document {
    let mut doc = Document::new("Own single raster accounting witness", size, mode, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let profile = if mode == ColorMode::Rgb { photocraft_cms::Builtin::Srgb.profile() } else { photocraft_cms::Builtin::SGray.profile() };
    let bytes = profile.to_bytes();
    doc.working_profile_interpretation =
        if mode == ColorMode::Rgb { WorkingProfileInterpretation::implicit_srgb(&bytes) } else { WorkingProfileInterpretation::implicit_sgray(&bytes) };
    doc.icc_profile = Some(bytes);
    let mut layer = Layer::raster("Own literal pixels", doc.pixel_format());
    let format = doc.pixel_format();
    let bpp = format.bytes_per_pixel();
    let mut raw = vec![0; size.width as usize * size.height as usize * bpp];
    for y in 0..size.height as i32 {
        for x in 0..size.width as i32 {
            let alpha = match (x + y).rem_euclid(4) {
                0 => 0.,
                1 => 0.25,
                2 => 0.75,
                _ => 1.,
            };
            let values: &[f64] = if mode == ColorMode::Rgb { &[0.125, 0.5000000000000001, 0.875, alpha] } else { &[0.5000000000000001, alpha] };
            let index = (y as usize * size.width as usize + x as usize) * bpp;
            let pixel = &mut raw[index..index + bpp];
            for (channel, value) in values.iter().enumerate() {
                photocraft_color::try_write_sample_f64(pixel, sample, channel, *value).unwrap();
            }
        }
    }
    *layer.surface_mut().unwrap() = Surface::try_from_interleaved(format, doc.bounds(), &raw).unwrap();
    doc.layers.push(layer);
    doc
}
fn limits(bytes: usize) -> NativeSaveLimits {
    NativeSaveLimits {
        preparation: PreparationLimits { resident_bytes: bytes, ..Default::default() },
        // Explicit reduced archive workspace; default64MiB would make a64MiB combined job impossible.
        archive: photocraft_format::zip::stream::Limits { max_metadata_bytes: 1 << 20, ..Default::default() },
    }
}
fn preview_pngs(archive: &[u8], limits: NativeSaveLimits) -> [Vec<u8>; 2] {
    let mut reader = photocraft_format::zip::stream::Reader::new(Cursor::new(archive), limits.archive, &Interrupt::NONE).unwrap();
    [reader.read_by_name("thumb.png", 8 << 20, &Interrupt::NONE).unwrap(), reader.read_by_name("composite/preview.png", 8 << 20, &Interrupt::NONE).unwrap()]
}
#[test]
fn medium_single_raster_native_and_both_previews_fit_actual64mib() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let doc = current_plain(mode, sample, Size::new(513, 385));
            let before = doc.clone();
            let generous = limits(128 << 20);
            let (baseline, result) = save_native_stream(&doc, Cursor::new(Vec::new()), generous, &Interrupt::NONE).unwrap();
            assert!(result.warnings.is_empty());
            let expected = preview_pngs(baseline.get_ref(), generous);
            let lower = limits(64 << 20);
            let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), lower, &Interrupt::NONE).unwrap();
            assert!(result.warnings.is_empty(), "{mode:?}/{sample:?}: {:?}", result.warnings);
            assert_eq!(preview_pngs(output.get_ref(), lower), expected);
            assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
            assert_eq!(doc, before);
        }
    }
}
#[test]
fn offbounds_nan_and_default_nan_are_not_hidden_from_preparation() {
    for default in [false, true] {
        let mut doc = current_plain(ColorMode::Rgb, SampleType::F64, Size::new(8, 6));
        if default {
            let raw: Vec<_> = [0.1_f64, f64::NAN, 0.3, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect();
            let format = doc.pixel_format();
            *doc.layers[0].surface_mut().unwrap() = Surface::try_with_default_bytes(format, &raw).unwrap();
        } else {
            doc.layers[0].surface_mut().unwrap().try_write_pixel_f64(4096, -4096, &[0.1, f64::NAN, 0.3, 1.0]).unwrap();
        }
        let before = doc.clone();
        let mut output = Cursor::new(Vec::new());
        assert!(save_native_stream(&doc, &mut output, limits(64 << 20), &Interrupt::NONE).is_err());
        assert!(output.get_ref().is_empty());
        // Surface equality is encoded-byte equality; retain direct raw-byte diagnostics as well.
        assert_eq!(doc.layers[0].surface().unwrap().default_bytes(), before.layers[0].surface().unwrap().default_bytes());
        assert_eq!(
            doc.layers[0].surface().unwrap().try_read_interleaved(photocraft_doc::Rect::new(4096, -4096, 4097, -4095)).unwrap(),
            before.layers[0].surface().unwrap().try_read_interleaved(photocraft_doc::Rect::new(4096, -4096, 4097, -4095)).unwrap()
        );
    }
}
#[test]
fn preparation_and_second_preview_cancellation_leave_output_empty() {
    let doc = current_plain(ColorMode::Rgb, SampleType::F64, Size::new(513, 385));
    let before = doc.clone();
    // Exact stage stops require an observer-based seam, not arbitrary poll thresholds.
    // This sweep proves source/output atomicity; the companion private design lists stage-specific tests.
    for stop in [1, 40, 4000, 200000] {
        let polls = AtomicUsize::new(0);
        let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= stop;
        let mut output = Cursor::new(Vec::new());
        assert!(save_native_stream(&doc, &mut output, limits(64 << 20), &Interrupt::cancel_only(&cancelled)).is_err());
        assert!(output.get_ref().is_empty());
        assert_eq!(doc, before);
    }
}

#[test]
fn small_plain_preview_does_not_require_legacy_fixed25mib_reservation() {
    let doc = current_plain(ColorMode::Rgb, SampleType::F64, Size::new(8, 6));
    let before = doc.clone();
    let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits(16 << 20), &Interrupt::NONE).unwrap();
    assert!(result.warnings.is_empty());
    assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
    assert_eq!(preview_pngs(output.get_ref(), limits(16 << 20)).len(), 2);
    assert_eq!(doc, before);
}

#[test]
#[ignore = "requires reviewed unpublished legal-nearest originals and captured BEFORE native-preview baseline"]
fn intact_imported_plain_previews_match_fixed_before_bytes_and_native_source() {
    use sha2::{Digest, Sha256};
    let digest = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("legal-nearest-geometry-evidence");
    let baseline = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_PREVIEW_BASELINE").unwrap());
    let expected_bytes = std::fs::read(baseline.join("baseline.json")).unwrap();
    assert_eq!(digest(&expected_bytes), "56a3b3e309ccf6b886e2ef3d4b6d825338b417bbfd1cc63e38b2a0497b2b4278");
    let expected: serde_json::Value = serde_json::from_slice(&expected_bytes).unwrap();
    let original_manifest = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(digest(&original_manifest), expected["source_manifest_sha256"]);
    let packet: serde_json::Value = serde_json::from_slice(&original_manifest).unwrap();
    let mut accepted = 0;
    for row in expected["records"].as_array().unwrap().iter().filter(|row| row["context"]["selected_proposal"] == true) {
        let id = row["id"].as_str().unwrap();
        let (case, phase) = id.rsplit_once('-').unwrap();
        let entry = &packet["records"].as_array().unwrap().iter().find(|r| r["id"] == case).unwrap()[phase];
        let raw = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
        assert_eq!(digest(&raw), row["original_xcf_sha256"]);
        let doc = photocraft_io::import("own-preview-source.xcf", &raw).unwrap().document;
        assert!(photocraft_compose::gimp_render::PreparedRenderer::supports_plain_normal_budgeted(&doc), "intact context {id}");
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits(64 << 20), &Interrupt::NONE).unwrap();
        assert!(result.warnings.is_empty(), "{id}: {:?}", result.warnings);
        assert_eq!(load_from_bytes(output.get_ref()).unwrap(), doc);
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
        for (png, expected) in preview_pngs(output.get_ref(), limits(64 << 20)).iter().zip(row["previews"].as_array().unwrap()) {
            assert_eq!(digest(png), expected["png_sha256"], "{id}");
            assert_eq!(png, &std::fs::read(baseline.join(expected["file"].as_str().unwrap())).unwrap());
        }
        accepted += 1;
    }
    assert_eq!(accepted, 36);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let id = format!("medium-{mode:?}-{sample:?}");
            let row = expected["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
            assert_eq!(row["synthetic_current_only"], true);
            let doc = current_plain(mode, sample, Size::new(513, 385));
            let before = doc.clone();
            let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits(64 << 20), &Interrupt::NONE).unwrap();
            assert!(result.warnings.is_empty());
            assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
            assert_eq!(doc, before);
            for (png, reference) in preview_pngs(output.get_ref(), limits(64 << 20)).iter().zip(row["previews"].as_array().unwrap()) {
                assert_eq!(digest(png), reference["png_sha256"], "{id}");
                assert_eq!(png, &std::fs::read(baseline.join(reference["file"].as_str().unwrap())).unwrap());
            }
        }
    }
}

fn current_with_encoding(mode: ColorMode, sample: SampleType, size: Size, encoding: SampleEncoding) -> Document {
    let mut doc = current_plain(mode, sample, size);
    let profile = photocraft_cms::xcf_color::effective_profile(
        None,
        if mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
        match encoding {
            SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
            SampleEncoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
            SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
        },
    )
    .unwrap();
    let bytes = profile.to_bytes();
    doc.sample_encoding = Some(encoding);
    doc.working_profile_interpretation =
        if mode == ColorMode::Rgb { WorkingProfileInterpretation::implicit_srgb(&bytes) } else { WorkingProfileInterpretation::implicit_sgray(&bytes) };
    doc.icc_profile = Some(bytes);
    doc
}
#[test]
fn tiny_linear_and_perceptual_current_previews_use_actual_lowered_workspace() {
    for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
                let doc = current_with_encoding(mode, sample, Size::new(8, 6), encoding);
                let before = doc.clone();
                assert!(photocraft_compose::gimp_render::PreparedRenderer::supports_plain_normal_budgeted(&doc));
                let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits(16 << 20), &Interrupt::NONE).unwrap();
                assert!(result.warnings.is_empty());
                assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before);
                assert_eq!(doc, before);
                for png in preview_pngs(output.get_ref(), limits(16 << 20)) {
                    assert!(!png.is_empty());
                }
            }
        }
    }
}
#[test]
#[ignore = "requires reviewed unpublished Linear/Perceptual originals and fixed BEFORE preview supplement"]
fn intact_linear_perceptual_previews_match_fixed_before232_pngs() {
    use sha2::{Digest, Sha256};
    let digest = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let packet = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_ENCODING_PREVIEW_BASELINE").unwrap());
    let raw = std::fs::read(packet.join("baseline.json")).unwrap();
    assert_eq!(digest(&raw), "430bfd092f13a48398f7075101f2627f412c94c420e6dd13d6ff257022c7d388");
    let before: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let verify = |doc: &Document, row: &serde_json::Value| {
        assert!(photocraft_compose::gimp_render::PreparedRenderer::supports_plain_normal_budgeted(doc), "{}", row["id"]);
        let source = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        let (output, result) = save_native_stream(doc, Cursor::new(Vec::new()), limits(64 << 20), &Interrupt::NONE).unwrap();
        assert!(result.warnings.is_empty(), "{}:{:?}", row["id"], result.warnings);
        assert_eq!(load_from_bytes(output.get_ref()).unwrap(), *doc);
        assert_eq!(photocraft_format::save_to_bytes(doc, &Default::default()).unwrap(), source);
        assert_eq!(row["previews"].as_array().unwrap().len(), 2);
        for (png, expected) in preview_pngs(output.get_ref(), limits(64 << 20)).iter().zip(row["previews"].as_array().unwrap()) {
            let old = std::fs::read(packet.join(expected["file"].as_str().unwrap())).unwrap();
            assert_eq!(digest(&old), expected["png_sha256"]);
            assert_eq!(*png, old, "{}", row["id"]);
        }
    };
    let mut imported = 0;
    for binding in before["source_manifests"].as_array().unwrap() {
        let folder = binding["folder"].as_str().unwrap();
        let path = root.join(folder);
        let manifest_raw = std::fs::read(path.join("observations.json")).unwrap();
        assert_eq!(digest(&manifest_raw), binding["manifest_sha256"]);
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_raw).unwrap();
        for row in before["records"].as_array().unwrap().iter().filter(|r| r["folder"] == folder && r["context"]["selected_proposal"] == true) {
            let original = manifest["records"].as_array().unwrap().iter().find(|r| r["id"] == row["case_id"]).unwrap();
            let entry = &original[row["phase"].as_str().unwrap()];
            let bytes = std::fs::read(path.join(entry["file"].as_str().unwrap())).unwrap();
            assert_eq!(digest(&bytes), entry["sha256"]);
            assert_eq!(digest(&bytes), row["original_xcf_sha256"]);
            let doc = photocraft_io::import("own-before-encoding.xcf", &bytes).unwrap().document;
            assert_eq!(serde_json::to_value(&doc.working_profile_interpretation).unwrap(), row["context"]["working_profile"]);
            assert_eq!(doc.icc_profile.as_ref().map(|b| digest(b)), row["context"]["active_icc_sha256"].as_str().map(str::to_owned));
            verify(&doc, row);
            imported += 1;
        }
    }
    assert_eq!(imported, 108);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let id = format!("medium-perceptual-{mode:?}-{sample:?}");
            let row = before["records"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
            assert_eq!(row["synthetic_current_only"], true);
            let doc = current_with_encoding(mode, sample, Size::new(513, 385), SampleEncoding::Perceptual);
            verify(&doc, row);
        }
    }
}
