//! UNRUN genuine isolated small-Auto Gaussian group engine workflows. Source/native buffers are external harness captures; no new numerical oracle.
use super::*;
use photocraft_doc::{Layer, LayerContent};
use photocraft_raster::{
    Interrupt,
    backing::{
        NativePlaneDescriptor,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: [(&str, &str, usize); 2] = [
    ("composition-matrix/group-28-group-mask.xcf", "740064c2b008a9785801b9de4425f685dc2618a7fcc938d37728723dd10ccc49", 10313),
    ("composition-matrix/group-28-group-mask.edit.xcf", "bf9f61f61cedbfdd6e3e3c25cb5066e4deef0c46eef16629d6546cfd1018706b", 10514),
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
fn imported(case: usize, q: &BackingQuota) -> photocraft_io::OwnedIsolatedGaussianGroupXcfImport {
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
    let photocraft_io::OwnedXcfImport::IsolatedGaussianGroup(owner) = result else { panic!("single selected original group carrier") };
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
#[ignore = "UNRUN: genuine mode28 Gaussian groups typed source/title/native continuation and paid six-item stable-filter discovery"]
fn gaussian_groups_preserve_original_owners_and_current_metadata() {
    for case in 0..CASES.len() {
        let q = quota();
        let imported = imported(case, &q);
        let owner = imported.document();
        assert!(owner.is_readonly_gaussian_group_storage());
        assert!(is_current_isolated_gaussian_group_candidate(owner.document(), &ctl()).unwrap());
        let mut bypass = Session::new();
        assert!(bypass.add_accounted_document(owner.clone(), None, &ctl()).is_err());
        let mut s = Session::new();
        s.add_owned_isolated_gaussian_group_xcf_import(&imported, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
        drop(imported);
        let initial = s.active().unwrap().doc.clone();
        let words = native(&s);
        let source = s.active().unwrap().source.clone();
        let state = initial.xcf_state.as_ref().unwrap();
        assert_eq!(state.items.len(), 6);
        assert_eq!(state.filter_masks.len(), 1);
        assert_eq!(state.filter_stacks.len(), 1);
        let node = &state.filter_stacks[0].nodes[0];
        assert_eq!(node.operation.schema_epoch, "xcf22-isolated-small-auto-v1");
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        for params in [serde_json::Value::Null, serde_json::json!({"offset":1,"limit":2})] {
            let reply = crate::accounted_xcf_inspect::prepare(&s, &params, &ctl()).unwrap();
            assert_eq!(reply.value()["items"]["total"], 6);
            assert_eq!(reply.value()["filters"]["total"], 1);
            assert!(reply.quota().shares_ledger(&q));
        }
        let reply = crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":node.id.0.to_string()}), &ctl()).unwrap();
        assert_eq!(reply.value()["argumentCount"], 5);
        drop(reply);
        assert!(s.active().unwrap().doc.shares_state(&initial));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        assert!(s.execute("xcf.filter.enable", serde_json::json!({"filterId":node.id.0.to_string(),"enabled":false})).is_err());
        let saved = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "Gaussian-current.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |snapshot, _| {
                photocraft_format::owned_document::save_current_document_archive(snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|w| w.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        assert_eq!(native(&s), words);
        assert_eq!(s.active().unwrap().source, source);
        assert!(!s.active().unwrap().history.can_undo());
        let mut generic = Session::new();
        assert!(generic.add_owned_native_archive(archive(&saved, &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "refuse", None, &ctl()).is_err());
        let mut reopened = Session::new();
        reopened.add_owned_native_isolated_gaussian_group_archive(archive(&saved, &q), "native", Some("actual/Gaussian.pcraft"), &ctl()).unwrap();
        assert_eq!(native(&reopened), words);
        assert_eq!(reopened.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
        assert_eq!(reopened.active().unwrap().doc.icc_profile, initial.icc_profile);
        assert_eq!(reopened.active().unwrap().doc.source_color_context, initial.source_color_context);
        assert!(reopened.active().unwrap().doc.quota().unwrap().shares_ledger(&q));
    }
}
#[test]
#[ignore = "UNRUN: strict Gaussian group cancellation/quota failure before source/native publication and unchanged installed query state"]
fn gaussian_group_failures_do_not_publish_partial_tabs() {
    let q = quota();
    let imported = imported(0, &q);
    let mut s = Session::new();
    let stop = || true;
    let c = ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) };
    assert!(s.add_owned_isolated_gaussian_group_xcf_import(&imported, "cancel", None, &c).is_err());
    assert!(s.documents().is_empty());
    let usage = q.usage().unwrap();
    let hold = q.reserve(photocraft_raster::backing::quota::QuotaKind::Index, (32u64 << 20) - usage.index).unwrap();
    assert!(s.add_owned_isolated_gaussian_group_xcf_import(&imported, "quota", None, &ctl()).is_err());
    assert!(s.documents().is_empty());
    drop(hold);
    s.add_owned_isolated_gaussian_group_xcf_import(&imported, "actual", None, &ctl()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.clone();
    let history = s.active().unwrap().history.clone();
    assert!(crate::accounted_xcf_inspect::prepare(&s, &serde_json::Value::Null, &c).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
}
