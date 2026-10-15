//! UNRUN genuine assigned-ICC NL Union geometry storage drafts.
//! Corpus/reference captures are external harness owners, not product receipts.
use super::*;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
fn corpus() -> PathBuf {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (p, h) in [
        ("normal-mask-geometry-holdout-evidence/left-top/observations.json", "1a565ade2d3f9f3dcef51124cda6b46d656e7785c0958a6bed75043b85f01633"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/observations.json", "52b4a0906fdc09d4f84e264bd9cff45bb6b53457cdb7808248599f5d007c557e"),
        ("normal-mask-geometry-integer-evidence/left-top/observations.json", "b55d75f1fe4b8fe47ff7eef87e2806bbf4d4c27401c392411f16297d29673884"),
        ("normal-mask-geometry-integer-evidence/right-bottom/observations.json", "1044b554b418b68f213e7c04ba055896255a937c55fd01fbcc114922ed039d4f"),
    ] {
        assert_eq!(hash(&std::fs::read(root.join(p)).unwrap()), h);
    }
    root
}
fn fixtures() -> [(&'static str, &'static str); 32] {
    [
        ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.xcf", "fd97c954dbfee8fa69695a69ce1e400499f2854a4f5af2acf75bb4a99d708b9b"),
        ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.edit.xcf", "e6ee1e8f20dcc203230ebe3ca6e97e249bac7180e3fea6c0ffd3aec3c6d1e784"),
        ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.xcf", "d09764e320785b11fe20ad38ae29a0a228b8422891e9dd2e6c114762297a9788"),
        ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.edit.xcf", "41ebbd873c690294822ecc5465ffad0e5e5913e0f2bcfb04801a324acad4290d"),
        ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.xcf", "b356d0639fbc7dff8a81fd8a8444356586088233cc8f423f7cc516a205af2461"),
        ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.edit.xcf", "87c3f95310e5b444c4b4fb07180483c82a62090e33af4e203c301db33955e8b5"),
        ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.xcf", "e4f455f58216921cffc088d0116b21040fae4bc232a20e073d0ac94aa5c5d81c"),
        ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.edit.xcf", "309e2249f263256046ee997d625e9bab3077271dfdf2a90bdfb7f5c31572da9d"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op1.xcf", "cd39ff9707616cf497ca9d91a408dd003362d5543313cc3af7a01d755b147b47"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op1.edit.xcf", "c7ef420ade131f885b34d4af70a1f666b6b38319a1e56a250027211bbe0bf653"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op1.xcf", "543abefd7dc93bc5f1e9f0278883cfea2b99cbc3fa6f0021e5ee9f859946fa77"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op1.edit.xcf", "2acd4181a11caaca0b9fb2e4d86f9cc471e73c7418e734b21fcc97de4e6f700a"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op1.xcf", "03110e9b7421b3797785da01d4db64d61bedd9eaf5517fd1fb8059ceefac420c"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op1.edit.xcf", "e22ca440d4336c93eae88f88b5b7fd6a385f2c763e38ad885f2d73990b04891f"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op1.xcf", "26150114e9cab77f81dde5b76e515a2a4a9f5436dfae123236fdd6560403f9d0"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op1.edit.xcf", "9a9bb62d8451fbb7ea881b23ccbc455d8ced987d94cca068158a9a4ec65b2394"),
        ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.xcf", "4250f944696df427e92e3e4568aede8f7ec49ff8d9fbf26c429cc2210282d5bf"),
        ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.edit.xcf", "e02d72491deadc5cf064eedf300403ec60dbdda4afccc44ac0463c865a1d2a75"),
        ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.xcf", "b7f941d9e7956fd4cbde707da299d92b9c5ad76131613e9e8589ead1a10894b7"),
        ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.edit.xcf", "515d8268a6c2b6edf9b1d83bbce0f9c83ab0d104e9263d44448f40210f00bf4e"),
        ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.xcf", "b39d43a1d3a4de272d655b526c104d90f67bbce6764f5865fe78bec5ab4e31e6"),
        ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.edit.xcf", "6cf19963129f4ecdcfd89b432f5a8c8adb281a484a163cd236c4d9d08c472d53"),
        ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.xcf", "0853c2ff39fc77dcb7aa88a2ab5411789fa08bc202141a9df6c8243e39ed6c4c"),
        ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.edit.xcf", "493e5c191750c4899698ce6ce5561aa8c51e5fab8f6182ceb54dd4b2411564a7"),
        ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op1.xcf", "1a3795ffbfd912dc9141dd5473bc55f46eaae1f3737e8f1a6089bd31b56849b3"),
        ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op1.edit.xcf", "8edb2d6495ae52f0d79f538490431951c0ad32879da4f088ffc27303939f3f73"),
        ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op1.xcf", "ec767073f9e1b1d3d0c5242af22c778cb1ba395d65687d7c81407dff74111ddd"),
        ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op1.edit.xcf", "f0ee58b09ad17524279957b947dafde4077b828f54e05f0c606ec83935f99a36"),
        ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op1.xcf", "10e91bcf488942ff308d5ff96ce3f7bc3b32e307f08dd21d253b0a23029c7d4d"),
        ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op1.edit.xcf", "8d1f60b922fb1c582c9281dd6c76d7ebdd2fd1f144bf56b8300933a237fe9204"),
        ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.xcf", "0649a852c01bf152bf5e7f384fd64896cec4394eaca794c977d8142d168d80ab"),
        ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.edit.xcf", "3b90d5e5a4cbafa30113b66d337cae01aa30c22f940c415c11bf9e922a43ad41"),
    ]
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn q() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
#[ignore = "UNRUN: all 32 SHA-pinned assigned geometry originals/edits, exact native mask world placement"]
fn all_originals_single_decode_real_quota_and_exact_world_native_storage() {
    let root = corpus();
    for (path, pin) in fixtures() {
        let raw = std::fs::read(root.join(path)).unwrap();
        assert_eq!(hash(&raw), pin);
        let original = photocraft_xcf::decode(&raw, &Default::default()).unwrap();
        assert!(qualified_analytic_normal_geometry(&original));
        let quota = q();
        let before = quota.usage().unwrap();
        let starts = AtomicU64::new(0);
        let progress = |v: f32| {
            if v.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        // The separate plain API remains closed; its failure does not run inside the
        // selected importer, whose own progress count must be exactly one.
        assert!(crate::import_owned_staged_plain_bytes_in_quota(path, Arc::new(raw.clone()), &Default::default(), &limits(), q(), &ctl()).is_err());
        let imported = crate::import_owned_xcf_bytes_in_quota(path, Arc::new(raw.clone()), &options, &limits(), quota.clone(), &ctl()).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        let crate::OwnedXcfImport::AnalyticGeometry(imported) = imported else { panic!("distinct sealed geometry family") };
        let owner = imported.document();
        assert!(owner.quota().shares_ledger(&quota));
        owner.validate_masked_analytic_normal_storage(&ctl()).unwrap();
        let doc = owner.document();
        assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), original.icc);
        assert_eq!(doc.icc_profile.as_ref().unwrap().as_slice(), original.icc);
        assert_eq!(doc.xcf_state.as_ref().unwrap().items.len(), 3);
        for (layer, source) in doc.layers.iter().zip(original.layers.iter().rev()) {
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("real native backing") };
            let plane = source.plane.as_ref().unwrap();
            let bpp = backed.checked_view().format().bytes_per_pixel();
            let r = bounds(source).unwrap();
            let mut actual = vec![0; plane.bytes.len()];
            for y in 0..source.height {
                for x in 0..source.width {
                    let at = (y as usize * source.width as usize + x as usize) * bpp;
                    backed.checked_view().read_logical_pixel_native_into(r.x0 + x as i32, r.y0 + y as i32, &mut actual[at..at + bpp], &ctl()).unwrap();
                }
            }
            assert_eq!(actual, plane.bytes);
            if let Some(source_mask) = &source.mask {
                let mask = layer.mask.as_ref().unwrap();
                let words = &source_mask.plane.as_ref().unwrap().bytes;
                assert_eq!(mask.surface.try_read_interleaved(r).unwrap(), *words);
                let expected = Surface::try_from_interleaved_with(mask.surface.format(), r, words, &Interrupt::NONE).unwrap();
                assert_eq!(mask.surface.default_bytes(), expected.default_bytes());
                assert_eq!(mask.surface.tile_count(), expected.tile_count());
                for (coord, tile) in expected.tiles() {
                    assert_eq!(mask.surface.tile(*coord).unwrap().bytes(), tile.bytes());
                }
            } else {
                assert!(layer.mask.is_none());
            }
        }
        assert_eq!(hash(&std::fs::read(root.join(path)).unwrap()), pin);
        drop(imported);
        assert_eq!(quota.usage().unwrap(), before);
    }
}
#[test]
#[ignore = "UNRUN: exact original predicate plus NL/Union restriction, no fallback to plain"]
fn selector_rejects_other_operators_encodings_and_bad_mask_placement() {
    let root = corpus();
    let (path, pin) = fixtures()[0];
    let bytes = std::fs::read(root.join(path)).unwrap();
    assert_eq!(hash(&bytes), pin);
    for fault in 0..5 {
        let mut file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        match fault {
            0 => file.layers[0].composite_mode = 2,
            1 => file.encoding = Encoding::Linear,
            2 => file.layers[0].mask.as_mut().unwrap().offset = (1, 0),
            3 => file.layers[0].mask_enabled = false,
            _ => file.icc.clear(),
        }
        let budget = photocraft_xcf::Budget::new_persistent(limits(), None).unwrap();
        assert!(!qualify(&file, &Interrupt::NONE, &budget).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: predecode cancel/work/actual-quota denial leaves original and quota unchanged"]
fn actual_control_and_preparation_refusals_release_all_private_storage() {
    let root = corpus();
    let (path, pin) = fixtures()[0];
    let raw = Arc::new(std::fs::read(root.join(path)).unwrap());
    assert_eq!(hash(&raw), pin);
    let quota = q();
    let before = quota.usage().unwrap();
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        crate::import_owned_xcf_bytes_in_quota(path, raw.clone(), &Default::default(), &limits(), quota.clone(), &cancelled),
        Err(IoError::Cancelled)
    ));
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(crate::import_owned_xcf_bytes_in_quota(path, raw.clone(), &Default::default(), &limits(), quota.clone(), &work).is_err());
    assert_eq!(quota.usage().unwrap(), before);
    let tiny = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 0 });
    let usage = tiny.usage().unwrap();
    assert!(crate::import_owned_xcf_bytes_in_quota(path, raw.clone(), &Default::default(), &limits(), tiny.clone(), &ctl()).is_err());
    assert_eq!(tiny.usage().unwrap(), usage);
    assert_eq!(hash(&raw), pin);
}
