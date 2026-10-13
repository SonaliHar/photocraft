#![cfg(feature = "corpus")]
//! Native save must prepare ordinary imported scenes on the existing job ledger.
use photocraft_format::{PreparationLimits, load_from_bytes};
use photocraft_io::{NativeSaveLimits, save_native_stream};
use photocraft_raster::Interrupt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
#[test]
#[ignore = "requires reviewed unpublished ordinary Normal scene originals"]
fn ordinary_scene_native_save_keeps_two_previews_and_all_authoritative_bits() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("normal-mask-ordinary-scene-evidence/medium");
    let raw = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_normal_mask_ordinary_scene_hashes.json")).unwrap();
    assert_eq!(digest(&raw), hashes["medium"]["observations.json"]);
    let observations: Value = serde_json::from_slice(&raw).unwrap();
    let records = observations["records"].as_array().unwrap();
    assert_eq!(records.len(), 64);
    let mut worst_display_code = 0.;
    let mut hidden_rgb_differences = 0u64;
    for row in records {
        let id = row["id"].as_str().unwrap();
        let bytes = fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
        assert_eq!(digest(&bytes), row["source_sha256"]);
        let doc = photocraft_io::import("own-scene.xcf", &bytes).unwrap().document;
        assert!(photocraft_compose::gimp_render::PreparedRenderer::requires_preparation_budget(&doc));
        let before = doc.clone();
        let limits = NativeSaveLimits {
            preparation: PreparationLimits { resident_bytes: 64 << 20, ..Default::default() },
            archive: photocraft_format::zip::stream::Limits { max_metadata_bytes: 4 << 20, ..Default::default() },
        };
        let (output, result) = save_native_stream(&doc, Cursor::new(Vec::new()), limits, &Interrupt::NONE).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(result.warnings.is_empty(), "{id}: {:?}", result.warnings);
        assert_eq!(load_from_bytes(output.get_ref()).unwrap(), before, "{id}: native changed");
        let mut archive = photocraft_format::zip::stream::Reader::new(Cursor::new(output.get_ref()), limits.archive, &Interrupt::NONE).unwrap();
        for path in ["thumb.png", "composite/preview.png"] {
            let png = archive.read_by_name(path, 8 << 20, &Interrupt::NONE).unwrap_or_else(|e| panic!("{id}/{path}: {e}"));
            let image = photocraft_codecs::decode(&png).unwrap();
            assert!(image.width() > 0 && image.height() > 0, "{id}/{path}");
            if path == "composite/preview.png" {
                assert_eq!((image.width(), image.height()), (513, 385));
                let projection = &row["projection"]["native"];
                let observed = fs::read(root.join(projection["file"].as_str().unwrap())).unwrap();
                assert_eq!(digest(&observed), projection["sha256"]);
                let profile_blob = &row["assigned_icc"];
                let profile_bytes = fs::read(root.join(profile_blob["file"].as_str().unwrap())).unwrap();
                assert_eq!(digest(&profile_bytes), profile_blob["sha256"]);
                let gray = row["model"] == "gray";
                let channels = if gray { 1 } else { 3 };
                let profile = photocraft_cms::xcf_color::parse_bounded_profile(
                    &profile_bytes,
                    if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
                )
                .unwrap();
                let transform =
                    photocraft_cms::Transform::new(&profile, photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Intent::RelativeColorimetric, true)
                        .unwrap();
                let pixels = image.to_rgba8();
                let sample = row["sample_type"].as_str().unwrap();
                let width = match sample {
                    "u16" => 2,
                    "u32" | "f32" => 4,
                    "f64" => 8,
                    other => panic!("unobserved sample {other}"),
                };
                assert_eq!(observed.len(), 513 * 385 * (channels + 1) * width);
                for (native, actual) in observed.chunks_exact((channels + 1) * width).zip(pixels.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice())) {
                    let mut values = [0.; 4];
                    for (target, bytes) in values.iter_mut().zip(native.chunks_exact(width)) {
                        *target = match sample {
                            "u16" => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.,
                            "u32" => f64::from(u32::from_le_bytes(bytes.try_into().unwrap())) / 4294967295.,
                            "f32" => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
                            "f64" => f64::from_le_bytes(bytes.try_into().unwrap()),
                            _ => unreachable!(),
                        };
                    }
                    let mut rgb = [0.; 3];
                    transform.try_eval_f64(&values[..channels], &mut rgb).unwrap();
                    let expected = [
                        (rgb[0].clamp(0., 1.) * 255. + 0.5) as u8,
                        (rgb[1].clamp(0., 1.) * 255. + 0.5) as u8,
                        (rgb[2].clamp(0., 1.) * 255. + 0.5) as u8,
                        (values[channels].clamp(0., 1.) * 255. + 0.5) as u8,
                    ];
                    // Reduction intentionally clears unassociated hidden color at
                    // alpha zero. Native source/reload above is still byte-exact.
                    if actual[3] == 0 && expected[3] == 0 && actual[..3] != expected[..3] {
                        hidden_rgb_differences += 1;
                    }
                    for channel in 0..4 {
                        let difference = if channel == 3 {
                            f64::from(actual[3].abs_diff(expected[3]))
                        } else {
                            (f64::from(actual[channel]) * f64::from(actual[3]) / 255. - f64::from(expected[channel]) * f64::from(expected[3]) / 255.).abs()
                        };
                        worst_display_code = f64::max(worst_display_code, difference);
                        assert!(difference <= 2., "{id}: independent visible premultiplied projection display {actual:?} vs {expected:?}");
                    }
                }
            }
        }
        assert_eq!(doc, before, "{id}: source changed");
        if id.ends_with("op1-source") {
            let (memory, result) = match photocraft_io::save_native_bytes(&doc, limits, &Interrupt::NONE) {
                Ok(result) => result,
                Err(error) => {
                    assert!(error.to_string().contains("native byte-export combined resident bytes"), "{id}: unexpected byte export error {error}");
                    assert_eq!(doc, before, "{id}: byte cap rejection source exact");
                    eprintln!("{id}: byte output co-residency correctly rejects64MiB; separate128MiB measurement");
                    let measured = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 128 << 20, ..limits.preparation }, ..limits };
                    photocraft_io::save_native_bytes(&doc, measured, &Interrupt::NONE).unwrap()
                }
            };
            assert!(result.warnings.is_empty(), "{id}: byte export warnings");
            assert_eq!(memory, *output.get_ref(), "{id}: controlled byte export parity");
            assert_eq!(load_from_bytes(&memory).unwrap(), before, "{id}: byte export exact native");
        }
        if row["sample_type"] == "u32" || row["sample_type"] == "f64" {
            let mut destination = Cursor::new(Vec::new());
            for stop_at in [300, 300_000].into_iter().filter(|stop| *stop == 300 || id.ends_with("op1-source")) {
                let count = AtomicUsize::new(0);
                let cancel = || count.fetch_add(1, Ordering::Relaxed) >= stop_at;
                assert!(
                    matches!(save_native_stream(&doc, &mut destination, limits, &Interrupt::cancel_only(&cancel)), Err(photocraft_io::IoError::Cancelled)),
                    "{id}: cancellation at {stop_at}"
                );
                assert!(count.load(Ordering::Relaxed) >= stop_at);
                assert!(destination.get_ref().is_empty());
                assert_eq!(doc, before, "{id}: cancellation source exact");
            }
            let denied = NativeSaveLimits { preparation: PreparationLimits { resident_bytes: 1 << 20, ..limits.preparation }, ..limits };
            assert!(save_native_stream(&doc, &mut destination, denied, &Interrupt::NONE).is_err(), "{id}: cap");
            assert!(destination.get_ref().is_empty());
            assert_eq!(doc, before);
        }
    }
    eprintln!(
        "64 independent scene saves: worst visible premultiplied RGB/alpha display code difference {worst_display_code}/255; preserved diagnostic hidden-RGB differences {hidden_rgb_differences}"
    );
}
