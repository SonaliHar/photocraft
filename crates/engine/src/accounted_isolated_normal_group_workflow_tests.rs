//! UNRUN genuine source/native isolated-group adoption. Encoded source/archive
//! captures and native fingerprints are external harness data, not product fees.
//! This tests current ownership/publication; no independent rendering oracle is invented.
use super::*;
use photocraft_doc::{Layer, LayerContent};
use photocraft_raster::{
    Interrupt,
    backing::{
        NativePlaneDescriptor,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Arc};
const CASES: [(&str, &str, usize); 24] = [
    ("accounted-normal-group-evidence/rgb-u16-visible-mask.xcf", "8ec65015ae6a86f7a66b2d3064c8025aafba3dd71dc4a6bed2aff316c380a503", 7701),
    ("accounted-normal-group-evidence/rgb-u16-visible-disabled-mask.xcf", "4b9d6b2922928d1e27d41bc28c2313ff9c9b9fe5293b1adcc4aa73253e1e5816", 7701),
    ("accounted-normal-group-evidence/rgb-u16-hidden-mask.xcf", "dd4e111678558c7a00d415fce919da6ab0849233e3a1064e3b39ce570e69c167", 7701),
    ("accounted-normal-group-evidence/rgb-u16-visible-no-mask.xcf", "4e27fc1ab7beacbd09ec5bde448161e9a9c063c5476a802282d16948c0fdf620", 7084),
    ("accounted-normal-group-evidence/rgb-f32-visible-mask.xcf", "67cfaded2d625e954701141ae482707efdb3efdb9c9be502c5dbd749c8f1734b", 11265),
    ("accounted-normal-group-evidence/rgb-f32-visible-disabled-mask.xcf", "8d15064582554b8c3fcaaf1f0caf36056045ea651a7550a4a82c467862e0e040", 11265),
    ("accounted-normal-group-evidence/rgb-f32-hidden-mask.xcf", "d9f29b14e0f5010a7b924d139e124525c6f998d96b0ca64571f4fe402801c630", 11265),
    ("accounted-normal-group-evidence/rgb-f32-visible-no-mask.xcf", "a19edeb4c37f0f93ae5ee0c05792d3f72585fdd6e39660eba2a4c2cb8f0380e7", 10609),
    ("accounted-normal-group-evidence/rgb-f64-visible-mask.xcf", "1205a864a9f30ae939bd779cd2d43b605b5faa85891b23754035d865389ba7ba", 14977),
    ("accounted-normal-group-evidence/rgb-f64-visible-disabled-mask.xcf", "30c6393c4558fa817e88d2471fec08f9a9bf0de63d82868baf43152f55771a47", 14977),
    ("accounted-normal-group-evidence/rgb-f64-hidden-mask.xcf", "f3cc9c7b470b05b9f562536dcd0b8a9a790f766f5b799a2b55e5aff3d8b4594e", 14977),
    ("accounted-normal-group-evidence/rgb-f64-visible-no-mask.xcf", "78d82562f2a2bf061e3892c6e578e7848af0c6fdf0a7c7e4d792aca1eb7ccc79", 14351),
    ("accounted-normal-group-evidence/gray-u16-visible-mask.xcf", "df0b24864ccd0e70f1aeed335342e4000ccd90cc4a0c894052e44ab989f6cab6", 5275),
    ("accounted-normal-group-evidence/gray-u16-visible-disabled-mask.xcf", "007c807b38447068bc14bdbb64183dece3c34fd79952197d79b24fa34437193e", 5275),
    ("accounted-normal-group-evidence/gray-u16-hidden-mask.xcf", "fbb09753819b80cb8be23dcce947abd598ac0da4c2d9cd3ea4518ffb7ab44185", 5275),
    ("accounted-normal-group-evidence/gray-u16-visible-no-mask.xcf", "f13540e940f0b672f09dbf671780b3b8a8586127203021ad86cc3dbcb0a78945", 4658),
    ("accounted-normal-group-evidence/gray-f32-visible-mask.xcf", "422e647d1c9719ec9ed83a54bef8f3644401006caa5d761c8f3a407a496c57cc", 6605),
    ("accounted-normal-group-evidence/gray-f32-visible-disabled-mask.xcf", "8bc3200bf55a140d5a68d401dd8a0b3be06d69128b3d66c79b039dea7fe9443c", 6605),
    ("accounted-normal-group-evidence/gray-f32-hidden-mask.xcf", "4adcdac87047b8f62c930c7edc47aeb5c7557a53248dca73eed168e380835a9f", 6605),
    ("accounted-normal-group-evidence/gray-f32-visible-no-mask.xcf", "3af473843c208e537ba9ff834d58f5ee5710a65faf7a63ade00e578f93b414dc", 5949),
    ("accounted-normal-group-evidence/gray-f64-visible-mask.xcf", "acc7e720fa0e7a250e57505c15fd608f9e88a08aa870dcdcd056bb0f78b73d05", 7716),
    ("accounted-normal-group-evidence/gray-f64-visible-disabled-mask.xcf", "75568b3a2aa6c3c0db58bc2f8632ae4a9203d4e1a6424b0d7c204663d1ef08da", 7716),
    ("accounted-normal-group-evidence/gray-f64-hidden-mask.xcf", "a95e87a3f938fe787f030e68c623163b5f33d31fe208c5c44fbf44024a7ca28e", 7716),
    ("accounted-normal-group-evidence/gray-f64-visible-no-mask.xcf", "e9e68e899e1c4c379a1079ceec6dc108cd5c587466566976594e341bbddca060", 7090),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap())
}
fn imported(case: usize, q: &BackingQuota) -> photocraft_io::OwnedIsolatedNormalGroupXcfImport {
    let (path, pin, len) = CASES[case];
    let bytes = std::fs::read(root().join(path)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(hash(&bytes), pin);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        path,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::IsolatedNormalGroup(owner) = result else { panic!("single selected original group carrier") };
    owner
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn native(s: &Session) -> (Vec<NativePlaneDescriptor>, String) {
    fn visit(layers: &[Layer], depth: usize, descriptors: &mut Vec<NativePlaneDescriptor>, digest: &mut Sha256) {
        assert!(depth <= 1);
        for layer in layers {
            match &layer.content {
                LayerContent::BackedRaster(backed) => {
                    descriptors.push(backed.descriptor());
                    digest.update(backed.descriptor().default_bytes());
                    for coord in backed.operative_bounds().tiles() {
                        let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                        digest.update(coord.tx.to_ne_bytes());
                        digest.update(coord.ty.to_ne_bytes());
                        match pin.encoded() {
                            NativeTileBytes::FullTile(bytes) => digest.update(bytes),
                            NativeTileBytes::DefaultPixel(pixel) => {
                                for _ in 0..65536 {
                                    digest.update(pixel);
                                }
                            }
                        }
                    }
                }
                LayerContent::Group(group) => visit(&group.children, depth + 1, descriptors, digest),
                _ => panic!("closed group content"),
            }
            if let Some(mask) = &layer.mask {
                digest.update(mask.surface.default_bytes());
                for (coordinate, tile) in mask.surface.tiles() {
                    digest.update(coordinate.tx.to_ne_bytes());
                    digest.update(coordinate.ty.to_ne_bytes());
                    digest.update(tile.bytes());
                }
            }
        }
    }
    let mut descriptors = Vec::new();
    let mut digest = Sha256::new();
    visit(&s.active().unwrap().doc.layers, 0, &mut descriptors, &mut digest);
    (descriptors, digest.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
fn archive(bytes: &[u8], q: &BackingQuota) -> OwnedDocumentArchive {
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
        .unwrap()
}
#[test]
#[ignore = "UNRUN:24 original assignedNL isolated groups, typed source/title/native continuation, hierarchy/native/mask/profile/source custody"]
fn original_groups_require_typed_adoption_and_preserve_current_identity_through_native() {
    let proof = std::fs::read(root().join("accounted-normal-group-evidence/observations.json")).unwrap();
    assert_eq!(hash(&proof), "2ec12176aa53eed6a4d65e21802b1ef8c2075613a14101beaf5ccba478e46b53");
    for case in 0..CASES.len() {
        let q = quota();
        let imported = imported(case, &q);
        let owner = imported.document();
        assert!(is_current_isolated_normal_group_candidate(owner.document(), &ctl()).unwrap());
        let mut bypass = Session::new();
        assert!(bypass.add_accounted_document(owner.clone(), None, &ctl()).is_err());
        assert!(bypass.documents().is_empty());
        let mut s = Session::new();
        s.add_owned_isolated_normal_group_xcf_import(&imported, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
        drop(imported);
        let initial = s.active().unwrap().doc.clone();
        let words = native(&s);
        let source = s.active().unwrap().source.clone();
        assert!(s.active().unwrap().source_read_only);
        assert!(s.active().unwrap().path.is_none());
        assert!(matches!(initial.layers[1].content, LayerContent::Group(_)));
        let mask = initial.layers[1].mask.as_ref().map(|mask| mask.surface.clone());
        let saved = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "group-current.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |snapshot, _| {
                photocraft_format::owned_document::save_current_document_archive(snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|writer| writer.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        assert!(!s.active().unwrap().history.can_undo());
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(native(&s), words);
        if let Some(mask) = mask {
            assert!(Arc::ptr_eq(&mask, &s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().surface));
        }
        let current = s.active().unwrap().doc.clone();
        assert_eq!(current.name, "group-current.pcraft");
        let mut ordinary = Session::new();
        assert!(
            ordinary
                .add_owned_native_archive(archive(&saved, &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "must-refuse.pcraft", None, &ctl())
                .is_err()
        );
        assert!(ordinary.documents().is_empty());
        let mut reopened = Session::new();
        reopened
            .add_owned_native_isolated_normal_group_archive(archive(&saved, &q), "actual-native.pcraft", Some("authorized/actual-native.pcraft"), &ctl())
            .unwrap();
        let state = reopened.active().unwrap();
        assert_eq!(state.doc.name, current.name);
        assert_eq!(state.doc.xcf_state, current.xcf_state);
        assert_eq!(state.doc.icc_profile, initial.icc_profile);
        assert_eq!(state.doc.source_color_context, initial.source_color_context);
        assert_eq!(state.doc.depth, initial.depth);
        assert_eq!(state.doc.mode, initial.mode);
        assert_eq!(native(&reopened), words);
        assert_eq!(state.path.as_deref(), Some("authorized/actual-native.pcraft"));
        assert!(!state.source_read_only);
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/actual-native.pcraft"));
        assert!(state.doc.quota().unwrap().shares_ledger(&q));
    }
}
#[test]
#[ignore = "UNRUN: group paid metadata pagination/title query returns4/5items0filters while retaining reply without mutation"]
fn group_current_metadata_and_native_title_keep_exact_upstream_schema() {
    for case in 0..CASES.len() {
        let q = quota();
        let imported = imported(case, &q);
        let mut s = Session::new();
        s.add_owned_isolated_normal_group_xcf_import(&imported, "group.xcf", None, &ctl()).unwrap();
        crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "Query title.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |_, _| Ok(()))
            .unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let state = before.xcf_state.as_ref().unwrap();
        let expected_rows = 4 + usize::from(before.layers[1].mask.is_some());
        assert_eq!(state.items.len(), expected_rows);
        for params in [serde_json::Value::Null, serde_json::json!({"offset":1,"limit":1}), serde_json::json!({"offset":4,"limit":128})] {
            // Comparison metadata is harness-owned and separately prepaid.
            let oracle = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
            let (offset, limit) = crate::accounted_xcf_inspect::pagination(&params, &ctl()).unwrap();
            let expected = crate::xcf_inspect_cmds::build_inspect_reply(&before, state, &before.effective_resolution(), offset, limit, None).unwrap();
            let baseline = q.usage().unwrap();
            let reply = crate::accounted_plain_xcf_inspect::prepare(&s, &params, &ctl()).unwrap();
            assert_eq!(reply.value(), &expected);
            assert_eq!(reply.value()["items"]["total"], expected_rows);
            assert_eq!(reply.value()["filters"]["total"], 0);
            assert!(reply.quota().shares_ledger(&q));
            assert!(q.usage().unwrap().index > baseline.index);
            drop(reply);
            assert_eq!(q.usage().unwrap(), baseline);
            drop(oracle);
        }
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
    }
}
#[test]
#[ignore = "UNRUN: group cancellation/work/quota/duplicate and failed writer preserve every installed publication field"]
fn group_private_failures_do_not_publish_or_fallback() {
    for case in [0, 3, 4, 8, 12, 16, 20] {
        let q = quota();
        let imported = imported(case, &q);
        let mut s = Session::new();
        let cancelled = Interrupt::cancel_only(&|| true);
        let cancel = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
        assert!(s.add_owned_isolated_normal_group_xcf_import(&imported, "cancel.xcf", None, &cancel).is_err());
        assert!(s.documents().is_empty());
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(s.add_owned_isolated_normal_group_xcf_import(&imported, "denied.xcf", None, &denied).is_err());
        assert!(s.documents().is_empty());
        let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.add_owned_isolated_normal_group_xcf_import(&imported, "quota.xcf", None, &ctl()).is_err());
        assert!(s.documents().is_empty());
        drop(hold);
        s.add_owned_isolated_normal_group_xcf_import(&imported, "group.xcf", Some(CASES[case].0), &ctl()).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let source = s.active().unwrap().source.clone();
        let journal = s.journal.clone();
        let revision = s.active().unwrap().revision;
        assert!(s.add_owned_isolated_normal_group_xcf_import(&imported, "duplicate.xcf", None, &ctl()).is_err());
        assert_eq!(s.documents().len(), 1);
        let save = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "fail.pcraft", &ctl()).unwrap();
        assert!(save.write_and_publish(&mut s, &ctl(), |_, _| Err::<(), _>(EngineError::Other("writer denied".into()))).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().revision, revision);
    }
}

#[test]
#[ignore = "UNRUN: genuine prepaid CURRENT-only wrong ICC relation keeps exact group hint and refuses every generic/strict adoption without fallback"]
fn malformed_selected_group_never_retries_a_generic_native_or_session_path() {
    let q = quota();
    let imported = imported(0, &q);
    let source = imported.document();
    // Fixed four/five-row, three-leaf source model: no tile payload clone. A
    // genuine Index+Resident fee precedes copied metadata and changed ICC bytes.
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    let mut current = doc.icc_profile.as_ref().unwrap().as_ref().clone();
    current[24] ^= 1;
    doc.icc_profile = Some(Arc::new(current));
    let owner = photocraft_doc::accounted_document::AccountedDocument::from_owned(doc, Arc::new((source.clone(), fee)), q.clone(), &ctl()).unwrap();
    assert!(is_current_isolated_normal_group_candidate(owner.document(), &ctl()).unwrap());
    let mut ordinary = Session::new();
    assert!(ordinary.add_accounted_document(owner.clone(), None, &ctl()).is_err());
    assert!(ordinary.documents().is_empty());
    let snapshot = crate::DocumentSnapshot::from_accounted(owner).unwrap();
    let bytes = photocraft_format::owned_document::save_current_document_archive(&snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
        .unwrap()
        .into_inner();
    let mut ordinary = Session::new();
    assert!(
        ordinary.add_owned_native_archive(archive(&bytes, &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "generic.pcraft", None, &ctl()).is_err()
    );
    assert!(ordinary.documents().is_empty());
    let mut strict = Session::new();
    assert!(strict.add_owned_native_isolated_normal_group_archive(archive(&bytes, &q), "strict.pcraft", None, &ctl()).is_err());
    assert!(strict.documents().is_empty());
}

#[path = "accounted_isolated_normal_group_edit_tests.rs"]
mod edits;
