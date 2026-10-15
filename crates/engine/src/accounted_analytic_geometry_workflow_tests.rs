//! UNRUN authentic 32-case nonlinear Union assigned-ICC geometry workflows.
//! Source and archive Vecs are external captures. Production import/native/model
//! owners retain actual quota receipts; no rendering-equality oracle is invented.
use super::*;
use photocraft_doc::LayerContent;
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
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
const CASES: [(&str, &str, usize); 32] = [
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.xcf", "fd97c954dbfee8fa69695a69ce1e400499f2854a4f5af2acf75bb4a99d708b9b", 9339),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.edit.xcf", "e6ee1e8f20dcc203230ebe3ca6e97e249bac7180e3fea6c0ffd3aec3c6d1e784", 9339),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.xcf", "d09764e320785b11fe20ad38ae29a0a228b8422891e9dd2e6c114762297a9788", 13710),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.edit.xcf", "41ebbd873c690294822ecc5465ffad0e5e5913e0f2bcfb04801a324acad4290d", 13706),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.xcf", "b356d0639fbc7dff8a81fd8a8444356586088233cc8f423f7cc516a205af2461", 5247),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.edit.xcf", "87c3f95310e5b444c4b4fb07180483c82a62090e33af4e203c301db33955e8b5", 5247),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.xcf", "e4f455f58216921cffc088d0116b21040fae4bc232a20e073d0ac94aa5c5d81c", 6690),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.edit.xcf", "309e2249f263256046ee997d625e9bab3077271dfdf2a90bdfb7f5c31572da9d", 6686),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op1.xcf", "cd39ff9707616cf497ca9d91a408dd003362d5543313cc3af7a01d755b147b47", 12177),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op1.edit.xcf", "c7ef420ade131f885b34d4af70a1f666b6b38319a1e56a250027211bbe0bf653", 12177),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op1.xcf", "543abefd7dc93bc5f1e9f0278883cfea2b99cbc3fa6f0021e5ee9f859946fa77", 18326),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op1.edit.xcf", "2acd4181a11caaca0b9fb2e4d86f9cc471e73c7418e734b21fcc97de4e6f700a", 18327),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op1.xcf", "03110e9b7421b3797785da01d4db64d61bedd9eaf5517fd1fb8059ceefac420c", 6469),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op1.edit.xcf", "e22ca440d4336c93eae88f88b5b7fd6a385f2c763e38ad885f2d73990b04891f", 6469),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op1.xcf", "26150114e9cab77f81dde5b76e515a2a4a9f5436dfae123236fdd6560403f9d0", 8478),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op1.edit.xcf", "9a9bb62d8451fbb7ea881b23ccbc455d8ced987d94cca068158a9a4ec65b2394", 8479),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.xcf", "4250f944696df427e92e3e4568aede8f7ec49ff8d9fbf26c429cc2210282d5bf", 6409),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.edit.xcf", "e02d72491deadc5cf064eedf300403ec60dbdda4afccc44ac0463c865a1d2a75", 6409),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.xcf", "b7f941d9e7956fd4cbde707da299d92b9c5ad76131613e9e8589ead1a10894b7", 10477),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.edit.xcf", "515d8268a6c2b6edf9b1d83bbce0f9c83ab0d104e9263d44448f40210f00bf4e", 10477),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.xcf", "b39d43a1d3a4de272d655b526c104d90f67bbce6764f5865fe78bec5ab4e31e6", 4261),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.edit.xcf", "6cf19963129f4ecdcfd89b432f5a8c8adb281a484a163cd236c4d9d08c472d53", 4261),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.xcf", "0853c2ff39fc77dcb7aa88a2ab5411789fa08bc202141a9df6c8243e39ed6c4c", 6385),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.edit.xcf", "493e5c191750c4899698ce6ce5561aa8c51e5fab8f6182ceb54dd4b2411564a7", 6385),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op1.xcf", "1a3795ffbfd912dc9141dd5473bc55f46eaae1f3737e8f1a6089bd31b56849b3", 8032),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op1.edit.xcf", "8edb2d6495ae52f0d79f538490431951c0ad32879da4f088ffc27303939f3f73", 8032),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op1.xcf", "ec767073f9e1b1d3d0c5242af22c778cb1ba395d65687d7c81407dff74111ddd", 13722),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op1.edit.xcf", "f0ee58b09ad17524279957b947dafde4077b828f54e05f0c606ec83935f99a36", 13722),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op1.xcf", "10e91bcf488942ff308d5ff96ce3f7bc3b32e307f08dd21d253b0a23029c7d4d", 5076),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op1.edit.xcf", "8d1f60b922fb1c582c9281dd6c76d7ebdd2fd1f144bf56b8300933a237fe9204", 5076),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.xcf", "0649a852c01bf152bf5e7f384fd64896cec4394eaca794c977d8142d168d80ab", 8014),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.edit.xcf", "3b90d5e5a4cbafa30113b66d337cae01aa30c22f940c415c11bf9e922a43ad41", 8014),
];
const PROOFS: [(&str, &str); 4] = [
    ("normal-mask-geometry-holdout-evidence/left-top/observations.json", "1a565ade2d3f9f3dcef51124cda6b46d656e7785c0958a6bed75043b85f01633"),
    ("normal-mask-geometry-holdout-evidence/right-bottom/observations.json", "52b4a0906fdc09d4f84e264bd9cff45bb6b53457cdb7808248599f5d007c557e"),
    ("normal-mask-geometry-integer-evidence/left-top/observations.json", "b55d75f1fe4b8fe47ff7eef87e2806bbf4d4c27401c392411f16297d29673884"),
    ("normal-mask-geometry-integer-evidence/right-bottom/observations.json", "1044b554b418b68f213e7c04ba055896255a937c55fd01fbcc114922ed039d4f"),
];

fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap())
}
fn source(path: &str, pin: &str, len: usize) -> Vec<u8> {
    let b = std::fs::read(root().join(path)).unwrap();
    assert_eq!(b.len(), len);
    assert_eq!(hash(&b), pin);
    b
}
fn imported(path: &str, bytes: &[u8], q: &BackingQuota) -> photocraft_io::OwnedAnalyticGeometryXcfImport {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        path,
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::AnalyticGeometry(owner) = result else { panic!("exact original analytic geometry route without fallback") };
    owner
}
fn native(s: &Session) -> (Vec<NativePlaneDescriptor>, String) {
    let doc = &s.active().unwrap().doc;
    let mut descriptors = Vec::new();
    let mut h = Sha256::new();
    for layer in &doc.layers {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("actual native owner") };
        descriptors.push(backed.descriptor());
        h.update(backed.descriptor().default_bytes());
        for coord in backed.descriptor().bounds().tiles() {
            let p = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
            match p.encoded() {
                NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => h.update(b),
            }
        }
        if let Some(mask) = &layer.mask {
            h.update(mask.surface.default_bytes());
            for (_, tile) in mask.surface.tiles() {
                h.update(tile.bytes());
            }
        }
    }
    (descriptors, h.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn archive(bytes: &[u8], q: &BackingQuota) -> OwnedDocumentArchive {
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
        .unwrap()
}
#[test]
#[ignore = "UNRUN: 32 pinned original/edit NLUnion geometries, typed source/fullcurrent validation, paid SaveAs and strict native reopen"]
fn analytic_source_saveas_and_native_reopen_keep_exact_mask_profile_geometry_and_source_guard() {
    for (path, pin) in PROOFS {
        assert_eq!(hash(&std::fs::read(root().join(path)).unwrap()), pin);
    }
    for (path, pin, len) in CASES {
        let bytes = source(path, pin, len);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let owner = imported(path, &bytes, &q);
        let mut s = Session::new();
        s.add_owned_analytic_geometry_xcf_import(&owner, path, Some(path), &ctl()).unwrap();
        drop(owner);
        let initial = s.active().unwrap().doc.clone();
        let raw = native(&s);
        assert!(is_current_analytic_geometry_candidate(&initial, &ctl()).unwrap());
        assert!(!initial.accounted().unwrap().is_readonly_point_storage());
        let source_identity = s.active().unwrap().source.clone();
        assert_eq!(source_identity.as_ref().unwrap().origin, photocraft_io::ImportOrigin::xcf());
        assert!(s.active().unwrap().source_read_only);
        assert!(s.active().unwrap().path.is_none());
        let revision = s.active().unwrap().revision;
        let history_count = s.active().unwrap().history.past_len();
        let prepared = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "analytic-current.pcraft", &ctl()).unwrap();
        assert_eq!(prepared.snapshot().xcf_state, initial.xcf_state);
        for (a, b) in initial.layers.iter().zip(&prepared.snapshot().layers) {
            match (&a.mask, &b.mask) {
                (Some(a), Some(b)) => assert!(Arc::ptr_eq(&a.surface, &b.surface)),
                (None, None) => {}
                _ => panic!("mask changed during paid titlecopy"),
            }
        }
        let saved = prepared
            .write_and_publish(&mut s, &ctl(), |snapshot, _| {
                photocraft_format::owned_document::save_current_document_archive(snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|v| v.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        let current = s.active().unwrap();
        assert_eq!(current.doc.name, "analytic-current.pcraft");
        assert_eq!(current.path.as_deref(), Some("analytic-current.pcraft"));
        assert_eq!(current.source, source_identity);
        assert!(current.source_read_only);
        assert_eq!(current.revision, revision);
        assert_eq!(current.history.past_len(), history_count);
        assert!(!current.history.can_undo());
        assert_eq!(native(&s), raw);
        let saved_current = current.doc.clone();
        let mut generic = Session::new();
        assert!(
            generic
                .add_owned_native_archive(archive(&saved, &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "must-not-fallback.pcraft", None, &ctl())
                .is_err()
        );
        assert!(generic.documents().is_empty());
        let mut reopened = Session::new();
        reopened
            .add_owned_native_analytic_geometry_archive(archive(&saved, &q), "actual-native.pcraft", Some("authorized/actual-native.pcraft"), &ctl())
            .unwrap();
        let state = reopened.active().unwrap();
        assert_eq!(state.doc.xcf_state, saved_current.xcf_state);
        assert_eq!(state.doc.name, saved_current.name);
        assert_eq!(state.doc.depth, initial.depth);
        assert_eq!(state.doc.mode, initial.mode);
        assert_eq!(state.doc.icc_profile, initial.icc_profile);
        assert_eq!(state.doc.source_color_context, initial.source_color_context);
        assert_eq!(state.doc.working_profile_interpretation, initial.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        assert!(state.doc.quota().unwrap().shares_ledger(&q));
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/actual-native.pcraft"));
        assert_eq!(state.path.as_deref(), Some("authorized/actual-native.pcraft"));
        assert!(!state.source_read_only);
        // The saved locator/fingerprint remains inert exclusion evidence; only
        // this authorized native path supplies current writable identity.
        assert_eq!(state.doc.xcf_state.as_ref().unwrap().provenance, initial.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(hash(&bytes), pin);
    }
}
#[test]
#[ignore = "UNRUN: actual analytic geometry source/native publication cancellation/quota/duplicate identity and failed save roll back all state"]
fn analytic_current_publication_and_save_failures_keep_real_owners_and_never_fallback() {
    for (path, pin, len) in CASES {
        let bytes = source(path, pin, len);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let owner = imported(path, &bytes, &q);
        let before = q.usage().unwrap();
        let mut s = Session::new();
        let stop = || true;
        let token = Interrupt::cancel_only(&stop);
        let cancel = ReadControl { interrupt: &token, work: &|_| Ok(()) };
        assert!(s.add_owned_analytic_geometry_xcf_import(&owner, path, None, &cancel).is_err());
        assert!(s.documents().is_empty());
        assert_eq!(q.usage().unwrap(), before);
        let fee = q.reserve(QuotaKind::Index, (32 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.add_owned_analytic_geometry_xcf_import(&owner, path, None, &ctl()).is_err());
        assert!(s.documents().is_empty());
        drop(fee);
        s.add_owned_analytic_geometry_xcf_import(&owner, path, Some(path), &ctl()).unwrap();
        let initial = s.active().unwrap().doc.clone();
        let raw = native(&s);
        let used = q.usage().unwrap();
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let source_identity = s.active().unwrap().source.clone();
        let journal = s.journal.clone();
        assert!(s.add_owned_analytic_geometry_xcf_import(&owner, path, None, &ctl()).is_err());
        assert_eq!(s.documents().len(), 1);
        let prepared = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "failed.pcraft", &ctl()).unwrap();
        assert!(prepared.write_and_publish(&mut s, &ctl(), |_, _| Err::<(), _>(EngineError::Other("intentional writer refusal".into()))).is_err());
        assert!(s.active().unwrap().doc.shares_state(&initial));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().source, source_identity);
        assert!(s.active().unwrap().path.is_none());
        assert_eq!(s.journal, journal);
        assert_eq!(native(&s), raw);
        assert_eq!(q.usage().unwrap(), used);
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert_eq!(is_current_analytic_geometry_candidate(&initial, &deny), Err(BackingError::Work));
        assert_eq!(is_current_analytic_geometry_candidate(&initial, &cancel), Err(BackingError::Cancelled));
    }
}
