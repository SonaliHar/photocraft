//! Discarded synthetic masks retain decode errors without materializing pixels; real masks and
//! unreadable preferred vector blocks preserve the original bitmap-mask selection.
use photocraft_io::{import, psd_to_document};
use photocraft_psd::{ChannelData, ColorMode, Compression, LayerRecord, MaskData, RealMask, Rect, SectionType, TaggedBlock, Version, testgen};

fn fixture(
    version: Version,
    depth: u16,
    compression: Compression,
    kind: &str,
    bad_preferred: bool,
    real_metadata: bool,
    bad_bitmap: bool,
) -> photocraft_psd::PsdFile {
    let mut file = testgen::merged_only(version, ColorMode::Rgb, depth, compression, 2, 2);
    let rect = Rect::from_xywh(0, 0, 2, 2);
    let sample: Vec<u8> = match depth {
        16 => 16384u16.to_be_bytes().to_vec(),
        32 => 0.25f32.to_be_bytes().to_vec(),
        _ => vec![64],
    };
    let plane = sample.repeat(4);
    let mut mask = photocraft_psd::LayerMask::new(rect, 255, 8);
    if real_metadata {
        mask.real = Some(RealMask { flags: 0, background: 255, rect });
    }
    let mut blocks = vec![TaggedBlock::new(*b"vmsk", vec![0, 0, 0, 3, 0, 0, 0, 0])];
    if bad_preferred {
        blocks.push(TaggedBlock::new(*b"vsms", vec![0]));
    }
    match kind {
        "Shape" => blocks.push(TaggedBlock::new(*b"SoCo", vec![])),
        "Type" => blocks.push(TaggedBlock::new(*b"TySh", vec![])),
        "Smart Object" => blocks.push(TaggedBlock::new(*b"SoLd", vec![])),
        "Group" => blocks.push(TaggedBlock::section_divider(SectionType::OpenFolder, None, None)),
        _ => (),
    }
    let mut bitmap = ChannelData::encode(-2, compression, &plane, 2, 2, depth, version).unwrap();
    if bad_bitmap {
        bitmap.data.truncate(1);
    }
    // -3 without matching real metadata must not override the synthetic -2 channel.
    let real = ChannelData::encode(-3, compression, &plane, 2, 2, depth, version).unwrap();
    file.layers_mut().push(LayerRecord {
        rect,
        name: b"mask work".to_vec(),
        mask: MaskData::Mask(mask),
        channels: vec![ChannelData::encode(0, compression, &plane, 2, 2, depth, version).unwrap(), bitmap, real],
        blocks,
        ..Default::default()
    });
    file
}

#[test]
fn synthetic_mask_discard_keeps_precedence_and_malformed_warnings() {
    for version in [Version::Psd, Version::Psb] {
        for depth in [8, 16, 32] {
            for compression in Compression::ALL {
                for (kind, bad_preferred, real_metadata, discarded) in [
                    ("Pixel", false, false, true),
                    ("Pixel", true, false, false),
                    ("Group", false, false, true),
                    ("Group", true, false, false),
                    ("Shape", true, false, true),
                    ("Type", true, false, true),
                    ("Smart Object", true, false, true),
                    ("Shape", false, true, false),
                    ("Pixel", false, true, false),
                ] {
                    for bad_bitmap in [false, true] {
                        let file = fixture(version, depth, compression, kind, bad_preferred, real_metadata, bad_bitmap);
                        let (mapped, warnings) = psd_to_document(&file);
                        let bytes = file.to_bytes().unwrap();
                        let imported = import("mask-work.psd", &bytes).unwrap();
                        assert_eq!(imported.warnings, warnings);
                        for doc in [&mapped, &imported.document] {
                            let layer = &doc.layers[0];
                            assert_eq!(layer.content.kind_name(), kind);
                            assert_eq!(layer.mask.is_none(), discarded, "{version:?}/{depth}/{compression:?}/{kind}/{bad_preferred}/{real_metadata}");
                            if real_metadata || !discarded && !bad_bitmap {
                                let value = layer.mask.as_ref().unwrap().surface.sample_channel(0, 0, 0);
                                let expected = if depth == 8 {
                                    64.0 / 255.0
                                } else if depth == 16 {
                                    16384.0 / 65535.0
                                } else {
                                    0.25
                                };
                                assert!((value - expected).abs() < 1e-6, "{value} != {expected}");
                            }
                            if matches!(kind, "Pixel" | "Group") {
                                assert_eq!(layer.vector_mask.is_some(), !bad_preferred);
                            }
                        }
                        let decode_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("channel -2 could not be decoded")).collect();
                        if bad_bitmap && !real_metadata {
                            let error = file.layers()[0].decode_channel(-2, depth, version).unwrap_err();
                            assert_eq!(decode_warnings, vec![&format!("layer \"mask work\": channel -2 could not be decoded ({error}); treated as empty")]);
                        } else {
                            assert!(decode_warnings.is_empty());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn absent_synthetic_mask_channel_does_not_emit_decode_warning() {
    for version in [Version::Psd, Version::Psb] {
        for depth in [8, 16, 32] {
            for kind in ["Pixel", "Group", "Shape", "Type", "Smart Object"] {
                for real_metadata in [false, true] {
                    let mut file = fixture(version, depth, Compression::Rle, kind, false, real_metadata, false);
                    // Mask metadata alone is not a channel. Neither synthetic nor real coverage
                    // is available, so the old importer emitted no missing-channel warning.
                    file.layers_mut()[0].channels.retain(|channel| ![-2, -3].contains(&channel.id));
                    let (mapped, warnings) = psd_to_document(&file);
                    let imported = import("absent-mask.psd", &file.to_bytes().unwrap()).unwrap();
                    assert_eq!(imported.warnings, warnings);
                    assert!(warnings.iter().all(|warning| !warning.contains("channel -2") && !warning.contains("channel -3")), "{warnings:?}");
                    assert!(mapped.layers[0].mask.is_none());
                    assert!(imported.document.layers[0].mask.is_none());
                }
            }
        }
    }
}
