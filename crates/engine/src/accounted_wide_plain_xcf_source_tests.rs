//! UNRUN wide source precision contracts. Generated XCF/native/comparison
//! buffers are external harness data, not GIMP evidence or incoming model fees.
#[path = "accounted_wide_plain_xcf_test_support.rs"]
mod fixture;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::LayerContent;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[test]
#[ignore = "UNRUN: actual generated RGB/Gray350U32/750F64 Legacy source import retains lowbits/signedzero/alpha/native defaults+padding+source precision with real quota custody; not producer/render oracle"]
fn actual_plain_sources_retain_wide_native_words_and_legacy_class() {
    for gray in [false, true] {
        for sample in [SampleType::U32, SampleType::F64] {
            for lock in [false, true] {
                let encoded = fixture::encoded(gray, sample, lock);
                let before = encoded.clone();
                let expected = fixture::expected(gray, sample);
                let parsed = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
                let precision = if sample == SampleType::U32 { 350 } else { 750 };
                assert_eq!(parsed.precision, precision);
                assert!(parsed.layers.iter().all(|d| !d.group && d.effects.is_empty() && d.mask.is_none()));
                assert_eq!(parsed.layers[0].lock_alpha, lock);
                let imported = fixture::import(gray, sample, lock);
                let q = imported.document().quota().clone();
                {
                    let owner = imported.document();
                    assert!(owner.is_legacy_storage());
                    assert!(!owner.is_readonly_point_storage());
                    assert!(!owner.is_readonly_normal_hierarchy_storage());
                    let doc = owner.document();
                    assert_eq!(doc.mode, if gray { ColorMode::Grayscale } else { ColorMode::Rgb });
                    assert_eq!(doc.depth, sample);
                    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
                    assert_eq!(doc.size, photocraft_geom::Size::new(257, 259));
                    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, precision);
                    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
                    assert_eq!(doc.xcf_state.as_ref().unwrap().provenance.format_version, 13);
                    assert!(doc.xcf_state.as_ref().unwrap().filter_stacks.is_empty());
                    assert!(doc.xcf_state.as_ref().unwrap().filter_masks.is_empty());
                    assert_eq!(fixture::payload(doc), expected);
                    let LayerContent::BackedRaster(native) = &doc.layers[0].content else { panic!("genuine retained native") };
                    let bpp = native.format().bytes_per_pixel();
                    assert_eq!(bpp, if gray { 2 } else { 4 } * if sample == SampleType::U32 { 4 } else { 8 });
                    assert!(native.default_bytes().iter().all(|b| *b == 0));
                    // Actual stored padded tiles are compared to external expected buffers;
                    // no logical-word check is substituted for physical/default custody.
                    for ty in 0..2 {
                        for tx in 0..2 {
                            let pin = native.checked_view().pin_tile(photocraft_geom::TileCoord::new(tx, ty), &fixture::ctl()).unwrap();
                            let NativeTileBytes::FullTile(raw) = pin.encoded() else { panic!("actual source SurfaceStore full tile") };
                            let mut want = native.default_bytes().repeat(65536);
                            for y in 0..256 {
                                for x in 0..256 {
                                    let px = tx * 256 + x;
                                    let py = ty * 256 + y;
                                    if px < 257 && py < 259 {
                                        let from = (py as usize * 257 + px as usize) * bpp;
                                        let at = (y as usize * 256 + x as usize) * bpp;
                                        want[at..at + bpp].copy_from_slice(&expected[from..from + bpp]);
                                    }
                                }
                            }
                            assert_eq!(raw, want);
                        }
                    }
                    if sample == SampleType::F64 {
                        assert_eq!(u64::from_ne_bytes(expected[..8].try_into().unwrap()), (-0f64).to_bits());
                        let words = fixture::data::words(1, gray, 1, 0).0;
                        let word = words[0];
                        assert_ne!(f64::from_bits(word).to_bits(), (f64::from_bits(word) as f32 as f64).to_bits());
                        assert!(expected.chunks_exact(8).any(|w| u64::from_ne_bytes(w.try_into().unwrap()) == (-0f64).to_bits()));
                    } else {
                        let word = u32::from_ne_bytes(expected[..4].try_into().unwrap());
                        assert_eq!(word, 0x80000001);
                        let roundtrip = (((word as f64 / u32::MAX as f64) as f32 as f64) * u32::MAX as f64).round() as u32;
                        assert_ne!(word, roundtrip);
                    }
                }
                let retained = imported.document().clone();
                drop(imported);
                assert!(q.usage().unwrap().resident > 0);
                assert_eq!(fixture::payload(retained.document()), expected);
                drop(retained);
                assert_eq!(q.usage().unwrap().resident, 0);
                assert_eq!(q.usage().unwrap().index, 0);
                assert_eq!(encoded, before);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: real plain Legacy Source import uses original caller Work/cancel/quota on350/750 RGBGray with typed refusal+refund; no fake Source/model or local meter reset"]
fn original_wide_source_controls_and_low_quota_fail_closed() {
    for gray in [false, true] {
        for sample in [SampleType::U32, SampleType::F64] {
            let bytes = Arc::new(fixture::encoded(gray, sample, false));
            let before = bytes.as_ref().clone();
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
            let q = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 });
            let spent = AtomicU64::new(0);
            let work = |n| {
                spent.fetch_add(n, Ordering::Relaxed);
                Ok(())
            };
            let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
            let source =
                photocraft_io::import_owned_staged_plain_bytes_in_quota("controlled-wide.xcf", bytes.clone(), &Default::default(), &limits, q.clone(), &c)
                    .unwrap();
            assert!(source.document().is_legacy_storage());
            assert!(spent.load(Ordering::Relaxed) > 0);
            drop(source);
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
            let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
            let error =
                photocraft_io::import_owned_staged_plain_bytes_in_quota("denied-wide.xcf", bytes.clone(), &Default::default(), &limits, q.clone(), &deny)
                    .unwrap_err();
            assert!(matches!(error, photocraft_io::IoError::Xcf(photocraft_xcf::XcfError { kind: photocraft_xcf::ErrorKind::Limit, .. })));
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
            let stop = || true;
            let interrupt = Interrupt::cancel_only(&stop);
            let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
            let error =
                photocraft_io::import_owned_staged_plain_bytes_in_quota("cancel-wide.xcf", bytes.clone(), &Default::default(), &limits, q.clone(), &cancelled)
                    .unwrap_err();
            assert!(matches!(
                error,
                photocraft_io::IoError::Cancelled | photocraft_io::IoError::Xcf(photocraft_xcf::XcfError { kind: photocraft_xcf::ErrorKind::Cancelled, .. })
            ));
            assert_eq!(q.usage().unwrap().resident, 0);
            assert_eq!(q.usage().unwrap().index, 0);
            let tiny = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 0 });
            assert!(
                photocraft_io::import_owned_staged_plain_bytes_in_quota(
                    "tiny-wide.xcf",
                    bytes.clone(),
                    &Default::default(),
                    &limits,
                    tiny.clone(),
                    &fixture::ctl()
                )
                .is_err()
            );
            assert_eq!(tiny.usage().unwrap().resident, 0);
            assert_eq!(tiny.usage().unwrap().index, 0);
            assert_eq!(bytes.as_ref(), &before);
        }
    }
}
