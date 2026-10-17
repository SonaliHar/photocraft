//! Deferred Source transport contracts; generated data is not a producer oracle.
#[path = "accounted_plain_hierarchy_paint_test_support.rs"]
mod fixture;
use photocraft_doc::LayerContent;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[test]
#[ignore = "UNRUN/uncompiled: genuine generated all-five-depth hierarchy Source transport/native words/defaults/tree/custody; no GIMP or numerical oracle"]
fn generated_all_depth_sources_keep_exact_words_and_real_custody() {
    for depth in 0..5 {
        let encoded = fixture::bytes(depth);
        let original = encoded.clone();
        let parsed = photocraft_xcf::decode(&encoded, &Default::default()).unwrap();
        assert_eq!(parsed.precision, fixture::precision(depth));
        assert!(parsed.layers.iter().all(|l| l.effects.is_empty() && l.mask.is_none()));
        let source = fixture::import(depth);
        let q = source.document().quota().clone();
        assert!(source.document().is_readonly_normal_hierarchy_storage());
        assert!(!source.document().is_legacy_storage());
        let owner = source.document().clone();
        let doc = owner.document();
        fixture::observed(doc, depth);
        let target = fixture::named(doc, "LeafA");
        let raw = fixture::raw(target);
        assert_eq!(raw, fixture::data::native(depth, 2, 0));
        if depth == 2 {
            assert_eq!(u32::from_ne_bytes(raw[..4].try_into().unwrap()), 0x80000000);
            assert_eq!(u32::from_ne_bytes(raw[16..20].try_into().unwrap()), 0x80000001);
        }
        if depth == 3 {
            assert_eq!(u32::from_ne_bytes(raw[..4].try_into().unwrap()), (-0f32).to_bits());
            assert_eq!(u32::from_ne_bytes(raw[12..16].try_into().unwrap()), (-0f32).to_bits());
            assert_eq!(u32::from_ne_bytes(raw[16..20].try_into().unwrap()), 0x3e800002);
        }
        if depth == 4 {
            assert_eq!(u64::from_ne_bytes(raw[..8].try_into().unwrap()), (-0f64).to_bits());
            assert_eq!(u64::from_ne_bytes(raw[24..32].try_into().unwrap()), (-0f64).to_bits());
            let v = f64::from_bits(u64::from_ne_bytes(raw[32..40].try_into().unwrap()));
            assert_ne!(v.to_bits(), (v as f32 as f64).to_bits());
        }
        for layer in fixture::layers(doc) {
            if let LayerContent::BackedRaster(b) = &layer.content {
                assert!(b.default_bytes().iter().all(|x| *x == 0));
                assert_eq!(b.format().sample, fixture::sample(depth));
                // Four retained native256 tiles, distinct from XCF64 source storage tiles.
                for y in 0..2 {
                    for x in 0..2 {
                        let pin = b.checked_view().pin_tile(photocraft_geom::TileCoord::new(x, y), &fixture::ctl()).unwrap();
                        let photocraft_raster::checked_view::NativeTileBytes::FullTile(bytes) = pin.encoded() else { panic!("actual Source tile") };
                        assert_eq!(bytes.len(), 65536 * b.format().bytes_per_pixel());
                    }
                }
            }
        }
        let payload = fixture::payload(doc);
        drop(source);
        assert!(q.usage().unwrap().resident > 0);
        assert_eq!(fixture::payload(owner.document()), payload);
        // The retained q ledger still owns its aggregate header; all payload/owner fees must refund.
        drop(owner);
        assert_eq!(q.usage().unwrap().resident, BackingQuota::aggregate_header_bytes());
        assert_eq!(q.usage().unwrap().index, BackingQuota::aggregate_header_bytes());
        assert_eq!(q.usage().unwrap().scratch, 0);
        assert_eq!(encoded, original);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: all-five real Source imports preserve original Work/cancel/quota refusal+refund; external generated/comparison buffers are not owner receipts"]
fn source_control_and_quota_remain_original() {
    for depth in 0..5 {
        let bytes = Arc::new(fixture::bytes(depth));
        let original = bytes.as_ref().clone();
        let q = BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 32 << 20), ("scratchBytes", 0)])
            .unwrap();
        let used = AtomicU64::new(0);
        let work = |n| {
            used.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let source = photocraft_io::import_owned_xcf_bytes_in_quota("five-depth.xcf", bytes.clone(), &Default::default(), &limits, q.clone(), &c).unwrap();
        assert!(used.load(Ordering::Relaxed) > 0);
        assert!(source.document().is_readonly_normal_hierarchy_storage());
        drop(source);
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        let e = photocraft_io::import_owned_xcf_bytes_in_quota("denied.xcf", bytes.clone(), &Default::default(), &limits, q.clone(), &deny).unwrap_err();
        assert!(matches!(e, photocraft_io::IoError::Xcf(photocraft_xcf::XcfError { kind: photocraft_xcf::ErrorKind::Limit, .. })));
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let cancel = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        let e = photocraft_io::import_owned_xcf_bytes_in_quota("cancel.xcf", bytes.clone(), &Default::default(), &limits, q.clone(), &cancel).unwrap_err();
        assert!(matches!(
            e,
            photocraft_io::IoError::Cancelled | photocraft_io::IoError::Xcf(photocraft_xcf::XcfError { kind: photocraft_xcf::ErrorKind::Cancelled, .. })
        ));
        assert_eq!(q.usage().unwrap().resident, 0);
        assert_eq!(q.usage().unwrap().index, 0);
        let tiny = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 0 });
        assert!(
            photocraft_io::import_owned_xcf_bytes_in_quota("tiny.xcf", bytes.clone(), &Default::default(), &limits, tiny.clone(), &fixture::ctl()).is_err()
        );
        assert_eq!(tiny.usage().unwrap().resident, 0);
        assert_eq!(tiny.usage().unwrap().index, 0);
        assert_eq!(bytes.as_ref(), &original);
    }
}
