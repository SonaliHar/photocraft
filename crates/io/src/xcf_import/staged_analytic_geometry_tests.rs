//! UNRUN genuine assigned-ICC analytic geometry storage drafts.
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
        assert_eq!(starts.load(Ordering::Relaxed), 2, "immutable metadata preplan plus selected decode, without family retry");
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
#[ignore = "UNRUN: exact original predicate, invalid operator/encoding restriction, no fallback to plain"]
fn selector_rejects_unknown_operators_encodings_and_bad_mask_placement() {
    let root = corpus();
    let (path, pin) = fixtures()[0];
    let bytes = std::fs::read(root.join(path)).unwrap();
    assert_eq!(hash(&bytes), pin);
    for fault in 0..5 {
        let mut file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        match fault {
            0 => file.layers[0].composite_mode = 5,
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

fn encoding_fixtures() -> [(&'static str, &'static str); 32] {
    [
        ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op1.xcf", "ff29148ffec4a0c845441d44d26da3aa666e277a77731faf16285cfc69923974"),
        ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op1.edit.xcf", "e9ecc3d9df4e1f16276e78c0785dedebf0686b232818d03319631b5cbd3aca2c"),
        ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op1.xcf", "45a731c7dec5f4004ce4ce95cc08a5d7158643f5f92df45fbb7f8a22565277a8"),
        ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op1.edit.xcf", "6d61f9f84f4a2fcd21cfafa22f7f08f3e931cdcea3a0842895762a3b920bb90f"),
        ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op1.xcf", "2e67b171d0b5d46ca601721611af65287fd897d82c19b74d09038cd647baebba"),
        ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op1.edit.xcf", "58222ceee401e0bf50768065330d16ab8eb4d027a835ad69e297df6b7e6a9831"),
        ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op1.xcf", "fc144090592f149c139cc6f980acdd03664746a4e17568ad64f814a4a0299468"),
        ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op1.edit.xcf", "19127848cfd4e6c0311e4a34bfe436c3e36f38804d76331eb2e09422be0aa3fb"),
        ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op1.xcf", "0bdd36bfb4fbd79702758938c93cb5621646f54433a505e7cb9ceae39037e24b"),
        ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op1.edit.xcf", "4ce316ac4c9c05189e1accd85bb513a864769805567293dfaa199e3acf883007"),
        ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op1.xcf", "b1504db52229023efe1cc8882d50a01221c49adcbd23b639b7df7a17b594f236"),
        ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op1.edit.xcf", "196760af08843723e30ad1f9777c7b93b0f0c92ea0a9551afc0507aabd8e1601"),
        ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op1.xcf", "810c8fde459b5a4030350f9e4954ea07d6e25eebd12e7e4086fc835b2fa993ef"),
        ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op1.edit.xcf", "ea023f69a927e2b37ce235313eae79bf2aabaa96fcd54f2eb9447a7f1ed027d6"),
        ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op1.xcf", "1b1f47f94420ad05c0e627f818feb822502ee4ad74eba2a57973d40ea074dd33"),
        ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op1.edit.xcf", "8e9fd0cceba626f643269525343f0eabc8641ed0ac4cdce27f891fe283841ce9"),
        ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op1.xcf", "3451e9edfcf702864dc8529a48bbd5ec5ac12c5472998d79a1537cdeeb381790"),
        ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op1.edit.xcf", "cceca9a59fc1f108e6e9145270a162530f2091c2d5d5518bfcddd1692b59f79f"),
        ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op1.xcf", "a811e5a2f8f3e46251479b0ae5ea855fa84b6b7d14c9c47da6ed1e1969ba75ec"),
        ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op1.edit.xcf", "c54c497f08b540a86d2e848763cdd0d6cca11ed8d36633b39965f63e7b31e63e"),
        ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op1.xcf", "826f043bb684df4b353d28ad9d19ddb488c8984b4569099b856ddf2c7cfc4251"),
        ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op1.edit.xcf", "a01e88a62507b2e291a237bd19f692fdcd538ded5efbfbf30fa09c77ade69fa9"),
        ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op1.xcf", "d6882a35bf13dca9366e57370afa502ee271eaf7ff3adb24c9c53bf163c6963e"),
        ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op1.edit.xcf", "045de31c546e062c798800f8fa1c2163ac028ad6f09ccc9a439038929f53f4f5"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op1.xcf", "8d1226c0e98e8dc6627b6230693299423cf999d9ba3efe9a67fbe8ce11e63264"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op1.edit.xcf", "0dda201db95c70eacd12a9e602f7a67d20376d36c5c89544827ac391d3e89fec"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op1.xcf", "286000e291b0cb98a4e05abab0c4fe9cd1b46103894c41c98799f6835e799a29"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op1.edit.xcf", "5a0ccdc319a39aa69a2768356ad04038472855effaec7ab6b4d552a66cf29cfd"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op1.xcf", "39a7e533c664dc18899ee21f5beff4b545960f4fe70e1b692532f9e27b4ad48d"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op1.edit.xcf", "937307a868319e3360554f1c78b744e976d11aca8bd3c2f295e6c04dcc6f9160"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op1.xcf", "05093aceeaf301e7637a9b3d4c11d98c1705f9c67050ccfff0aeae6d2e9de45e"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op1.edit.xcf", "d890ebc5275e00da4fadd3faaea86cce679d0e604af557c424d32d2aee0e9625"),
    ]
}
fn encoding_corpus() -> PathBuf {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (path, pin) in [
        ("normal-mask-geometry-linear-evidence/left-top/observations.json", "80d017fc2897fd449b8386c98e73f1d6caaded57f74cd0aaec6d2eddb80822cc"),
        ("normal-mask-geometry-linear-evidence/right-bottom/observations.json", "a4c12094204696a243d8136ebbda762681fe02ae68cea984b2beae1950364566"),
        ("normal-mask-geometry-perceptual-evidence/left-top/observations.json", "6cd4ad61905af74728ced7d2b13e8ea47c16ffc64d4de9cedc6a24f3a9478abc"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/observations.json", "4618d2efdf9c21196634ebd60cc9a92369f9ae0cebe4d17469282c68f7bce1c2"),
    ] {
        assert_eq!(hash(&std::fs::read(root.join(path)).unwrap()), pin);
    }
    root
}
#[test]
#[ignore = "UNRUN:32 genuine Linear/Perceptual Union originals/edits, distinct assigned/current ICC and native words"]
fn original_encoding_geometry_preserves_source_and_generates_actual_current_profile() {
    let root = encoding_corpus();
    for (path, pin) in encoding_fixtures() {
        let raw = std::fs::read(root.join(path)).unwrap();
        assert_eq!(hash(&raw), pin);
        let original = photocraft_xcf::decode(&raw, &Default::default()).unwrap();
        assert!(qualified_analytic_normal_geometry(&original));
        assert!(matches!(
            (original.precision, original.sample, original.encoding),
            (600, Sample::F32, Encoding::Linear)
                | (700, Sample::F64, Encoding::Linear)
                | (675, Sample::F32, Encoding::Perceptual)
                | (775, Sample::F64, Encoding::Perceptual)
        ));
        let quota = q();
        let before = quota.usage().unwrap();
        let starts = AtomicU64::new(0);
        let progress = |v: f32| {
            if v.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let import = crate::import_owned_xcf_bytes_in_quota(path, Arc::new(raw.clone()), &options, &limits(), quota.clone(), &ctl()).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 2, "immutable metadata preplan plus selected decode, without family retry");
        let crate::OwnedXcfImport::AnalyticGeometry(import) = import else { panic!("same sealed geometry family") };
        let owner = import.document();
        owner.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert!(owner.validate_masked_analytic_normal_storage(&ctl()).is_err());
        assert!(owner.quota().shares_ledger(&quota));
        let doc = owner.document();
        let epoch = if original.encoding == Encoding::Linear {
            photocraft_doc::import_compositing::XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH
        } else {
            photocraft_doc::import_compositing::XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH
        };
        for item in &doc.xcf_state.as_ref().unwrap().items {
            if let Some(policy) = &item.composition {
                assert_eq!(policy.compatibility_epoch, epoch);
            }
        }
        assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), original.icc);
        assert_eq!(doc.sample_encoding, Some(encoding(original.encoding)));
        let space = if original.model == Model::Gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
        let transfer =
            if original.encoding == Encoding::Linear { photocraft_cms::xcf_color::Encoding::Linear } else { photocraft_cms::xcf_color::Encoding::Perceptual };
        let expected = photocraft_cms::xcf_color::effective_profile(Some(&original.icc), space, transfer).unwrap().to_bytes();
        assert_eq!(doc.icc_profile.as_ref().unwrap().as_slice(), expected.as_slice());
        assert_ne!(expected.as_slice(), original.icc);
        // Test expected-provider objects and source captures are harness references;
        // product current/source/model/native payloads retain real same-q fees.
        for (layer, source) in doc.layers.iter().zip(original.layers.iter().rev()) {
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("actual backed owner") };
            let p = source.plane.as_ref().unwrap();
            let r = bounds(source).unwrap();
            let bpp = backed.checked_view().format().bytes_per_pixel();
            let mut bytes = vec![0; p.bytes.len()];
            for y in 0..source.height {
                for x in 0..source.width {
                    let at = (y as usize * source.width as usize + x as usize) * bpp;
                    backed.checked_view().read_logical_pixel_native_into(r.x0 + x as i32, r.y0 + y as i32, &mut bytes[at..at + bpp], &ctl()).unwrap();
                }
            }
            assert_eq!(bytes, p.bytes);
            if let Some(mask) = &source.mask {
                let actual = &layer.mask.as_ref().unwrap().surface;
                let expected = Surface::try_from_interleaved_with(actual.format(), r, &mask.plane.as_ref().unwrap().bytes, &Interrupt::NONE).unwrap();
                assert_eq!(actual.default_bytes(), expected.default_bytes());
                assert_eq!(actual.tile_count(), expected.tile_count());
                for (coord, tile) in expected.tiles() {
                    assert_eq!(actual.tile(*coord).unwrap().bytes(), tile.bytes());
                }
            }
        }
        drop(import);
        assert_eq!(quota.usage().unwrap(), before);
        assert_eq!(hash(&std::fs::read(root.join(path)).unwrap()), pin);
    }
}
#[test]
#[ignore = "UNRUN: Linear/Perceptual Union gates preserve source precision/tuple and atomic work refusal"]
fn encoding_geometry_does_not_relabel_mismatched_precision_or_unknown_operator() {
    let root = encoding_corpus();
    let (path, pin) = encoding_fixtures()[0];
    let raw = Arc::new(std::fs::read(root.join(path)).unwrap());
    assert_eq!(hash(&raw), pin);
    for fault in 0..4 {
        let mut file = photocraft_xcf::decode(&raw, &Default::default()).unwrap();
        match fault {
            0 => file.layers[0].composite_mode = 5,
            1 => file.precision = 650,
            2 => file.layers[0].mask_shown = true,
            _ => file.layers[0].mask.as_mut().unwrap().offset = (1, 0),
        }
        let b = photocraft_xcf::Budget::new_persistent(limits(), None).unwrap();
        assert!(!qualify(&file, &Interrupt::NONE, &b).unwrap());
    }
    let quota = q();
    let before = quota.usage().unwrap();
    let spent = AtomicU64::new(0);
    let work = |n: u64| {
        if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > 100000 { Err(BackingError::Work) } else { Ok(()) }
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(crate::import_owned_xcf_bytes_in_quota(path, raw.clone(), &Default::default(), &limits(), quota.clone(), &c).is_err());
    assert_eq!(quota.usage().unwrap(), before);
    assert_eq!(hash(&raw), pin);
}

#[test]
#[ignore = "UNRUN: semantic qualifier work is independently prepaid before original provider, including NL"]
fn semantic_profile_work_denial_keeps_temporary_receipts_and_source_atomic() {
    for (path, pin) in [fixtures()[0], encoding_fixtures()[0]] {
        let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
        let raw = std::fs::read(root.join(path)).unwrap();
        assert_eq!(hash(&raw), pin);
        let file = photocraft_xcf::decode(&raw, &Default::default()).unwrap();
        // These parsed expected/provider references belong to the external harness.
        let profile = photocraft_cms::Profile::parse(&file.icc).unwrap();
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(&profile).unwrap();
        let planning = photocraft_cms::Profile::parse_planning_work_ceiling(file.icc.len()).unwrap();
        let req = photocraft_cms::Profile::parse_requirements_with_control(
            &file.icc,
            photocraft_cms::control::Limits { work_units: planning, ..Default::default() },
            &|| false,
        )
        .unwrap();
        let gate = photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(&file.icc, Default::default(), &|| false).unwrap();
        let before_semantic = 256 + planning + 5 * req.work_units + gate.work_units;
        let allowed = before_semantic + semantic - 1;
        let consumed = AtomicU64::new(0);
        let observer = |n: u64| {
            let now = consumed.fetch_add(n, Ordering::Relaxed) + n;
            if now > allowed {
                Err(XcfError { kind: ErrorKind::Limit, offset: 0, item: String::new(), message: "test semantic prepayment denial".into() })
            } else {
                Ok(())
            }
        };
        let budget = photocraft_xcf::Budget::new_persistent_with_work(limits(), None, Some(&observer)).unwrap();
        let resident = budget.used(BudgetKind::Resident);
        assert!(qualify(&file, &Interrupt::NONE, &budget).is_err());
        assert!(consumed.load(Ordering::Relaxed) > before_semantic);
        assert_eq!(budget.used(BudgetKind::Resident), resident);
        assert_eq!(hash(&raw), pin);
    }
}

fn operator_fixtures(operator: i32) -> Vec<(String, String)> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let mut out = Vec::new();
    for (path, pin) in [
        ("normal-mask-geometry-holdout-evidence/left-top/observations.json", "1a565ade2d3f9f3dcef51124cda6b46d656e7785c0958a6bed75043b85f01633"),
        ("normal-mask-geometry-holdout-evidence/right-bottom/observations.json", "52b4a0906fdc09d4f84e264bd9cff45bb6b53457cdb7808248599f5d007c557e"),
        ("normal-mask-geometry-integer-evidence/left-top/observations.json", "b55d75f1fe4b8fe47ff7eef87e2806bbf4d4c27401c392411f16297d29673884"),
        ("normal-mask-geometry-integer-evidence/right-bottom/observations.json", "1044b554b418b68f213e7c04ba055896255a937c55fd01fbcc114922ed039d4f"),
        ("normal-mask-geometry-linear-evidence/left-top/observations.json", "80d017fc2897fd449b8386c98e73f1d6caaded57f74cd0aaec6d2eddb80822cc"),
        ("normal-mask-geometry-linear-evidence/right-bottom/observations.json", "a4c12094204696a243d8136ebbda762681fe02ae68cea984b2beae1950364566"),
        ("normal-mask-geometry-perceptual-evidence/left-top/observations.json", "6cd4ad61905af74728ced7d2b13e8ea47c16ffc64d4de9cedc6a24f3a9478abc"),
        ("normal-mask-geometry-perceptual-evidence/right-bottom/observations.json", "4618d2efdf9c21196634ebd60cc9a92369f9ae0cebe4d17469282c68f7bce1c2"),
    ] {
        let proof = std::fs::read(root.join(path)).unwrap();
        assert_eq!(hash(&proof), pin);
        let json: serde_json::Value = serde_json::from_slice(&proof).unwrap();
        for row in json["records"].as_array().unwrap() {
            let name = row["source_file"].as_str().unwrap();
            assert!(!name.contains(['/', '\\']));
            if !name.contains(&format!("-op{operator}.")) {
                continue;
            }
            let relative = PathBuf::from(path).parent().unwrap().join(name);
            let pin = row["source_sha256"].as_str().unwrap().to_string();
            assert_eq!(hash(&std::fs::read(root.join(&relative)).unwrap()), pin);
            out.push((relative.to_string_lossy().into_owned(), pin));
        }
    }
    assert_eq!(out.len(), 64);
    out
}
fn check_nonunion_operator(operator: i32) {
    use photocraft_doc::{import_compositing::GimpOperator, item_identity::ItemRef};
    let expected_op = match operator {
        2 => GimpOperator::ClipToBackdrop,
        3 => GimpOperator::ClipToLayer,
        4 => GimpOperator::Intersection,
        _ => panic!("test operator"),
    };
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (path, pin) in operator_fixtures(operator) {
        let raw = std::fs::read(root.join(&path)).unwrap();
        let original = photocraft_xcf::decode(&raw, &Default::default()).unwrap();
        assert!(qualified_analytic_normal_geometry(&original));
        assert_eq!(original.layers[0].composite_mode, operator);
        let quota = q();
        let before = quota.usage().unwrap();
        let starts = AtomicU64::new(0);
        let progress = |v: f32| {
            if v.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let import = crate::import_owned_xcf_bytes_in_quota(&path, Arc::new(raw.clone()), &options, &limits(), quota.clone(), &ctl()).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 2, "immutable metadata preplan plus selected decode, without family retry");
        let crate::OwnedXcfImport::AnalyticGeometry(import) = import else { panic!("same private geometry carrier") };
        let owner = import.document();
        owner.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        assert!(owner.validate_masked_analytic_normal_storage(&ctl()).is_err());
        assert!(owner.quota().shares_ledger(&quota));
        assert!(import.source_read_only());
        assert!(!import.preview_only());
        let doc = owner.document();
        let state = doc.xcf_state.as_ref().unwrap();
        let item = state.items.iter().find(|v| v.item == ItemRef::Layer(doc.layers[1].id)).unwrap();
        let policy = item.composition.as_ref().unwrap();
        assert_eq!(policy.stored_operator, operator);
        assert_eq!(policy.checked_resolved().unwrap().operator, expected_op);
        // A standalone malformed policy clone has no owner/source authority. Its
        // contradictory cached resolution must refuse, and the real owner stays intact.
        let mut wrong = policy.clone();
        wrong.resolved.as_mut().unwrap().operator = GimpOperator::Union;
        assert!(wrong.checked_resolved().is_err());
        let mut unknown = original.clone();
        unknown.layers[0].composite_mode = 5;
        let b = photocraft_xcf::Budget::new_persistent(limits(), None).unwrap();
        assert!(!qualify(&unknown, &Interrupt::NONE, &b).unwrap());
        assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), original.icc);
        let ce = match original.encoding {
            Encoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
            Encoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
            Encoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        };
        let space = if original.model == Model::Gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
        let effective = photocraft_cms::xcf_color::effective_profile(Some(&original.icc), space, ce).unwrap().to_bytes();
        assert_eq!(doc.icc_profile.as_ref().unwrap().as_slice(), effective.as_slice());
        for (layer, source) in doc.layers.iter().zip(original.layers.iter().rev()) {
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("real charged raster") };
            let p = source.plane.as_ref().unwrap();
            let r = bounds(source).unwrap();
            let bpp = backed.checked_view().format().bytes_per_pixel();
            let mut native = vec![0; p.bytes.len()];
            for y in 0..source.height {
                for x in 0..source.width {
                    let at = (y as usize * source.width as usize + x as usize) * bpp;
                    backed.checked_view().read_logical_pixel_native_into(r.x0 + x as i32, r.y0 + y as i32, &mut native[at..at + bpp], &ctl()).unwrap();
                }
            }
            assert_eq!(native, p.bytes);
            if let Some(source_mask) = &source.mask {
                let mask = &layer.mask.as_ref().unwrap().surface;
                let expected = Surface::try_from_interleaved_with(mask.format(), r, &source_mask.plane.as_ref().unwrap().bytes, &Interrupt::NONE).unwrap();
                assert_eq!(mask.default_bytes(), expected.default_bytes());
                assert_eq!(mask.tile_count(), expected.tile_count());
                for (coord, tile) in expected.tiles() {
                    assert_eq!(mask.tile(*coord).unwrap().bytes(), tile.bytes());
                }
            }
        }
        owner.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
        drop(import);
        assert_eq!(quota.usage().unwrap(), before);
        assert_eq!(hash(&std::fs::read(root.join(&path)).unwrap()), pin);
    }
}
#[test]
#[ignore = "UNRUN:64 actual ClipToBackdrop original/edit sources, exact profile/native/mask/resolved controls"]
fn original_clip_to_backdrop_geometry_uses_same_sealed_source_owner() {
    check_nonunion_operator(2);
}
#[test]
#[ignore = "UNRUN:64 actual ClipToLayer original/edit sources, exact profile/native/mask/resolved controls"]
fn original_clip_to_layer_geometry_uses_same_sealed_source_owner() {
    check_nonunion_operator(3);
}
#[test]
#[ignore = "UNRUN:64 actual Intersection original/edit sources, exact profile/native/mask/resolved controls"]
fn original_intersection_geometry_uses_same_sealed_source_owner() {
    check_nonunion_operator(4);
}
