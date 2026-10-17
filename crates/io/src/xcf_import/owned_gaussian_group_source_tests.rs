//! UNRUN consuming bridge/census regressions. Authored source bytes are external
//! harness inputs; they do not turn a synthetic model into original evidence.
use super::*;
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 })
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap()
}
fn packet(mode: u32, edit: bool) -> Arc<Vec<u8>> {
    let (name, pin) = match (mode, edit) {
        (28, false) => ("group-28-group-mask.xcf", "740064c2b008a9785801b9de4425f685dc2618a7fcc938d37728723dd10ccc49"),
        (28, true) => ("group-28-group-mask.edit.xcf", "bf9f61f61cedbfdd6e3e3c25cb5066e4deef0c46eef16629d6546cfd1018706b"),
        (61, false) => ("group-61-group-mask.xcf", "879665fb811715607fbc19d67bade30b74a37b70dc5ffdf539d762d251c189e3"),
        (61, true) => ("group-61-group-mask.edit.xcf", "d4d2b2f40c31664915ad63c8a24ef10317a1eb2f769ec39a8f81cf08317b56eb"),
        _ => panic!("only pinned positive/negative packets"),
    };
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = std::fs::read(root.join("composition-matrix").join(name)).unwrap();
    assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), pin);
    Arc::new(bytes)
}
fn leaves(doc: &Document) -> [&photocraft_doc::BackedLayer; 3] {
    let LayerContent::BackedRaster(base) = &doc.layers[0].content else { panic!("actual backdrop store") };
    let LayerContent::Group(group) = &doc.layers[1].content else { panic!("actual group, no projection") };
    let LayerContent::BackedRaster(first) = &group.children[0].content else { panic!("actual child store") };
    let LayerContent::BackedRaster(second) = &group.children[1].content else { panic!("actual child store") };
    [base, first, second]
}
#[test]
#[ignore = "UNRUN: actual single-decode source promotion and flat retained title lifetime on original ledger"]
fn original_and_edit_keep_three_stores_two_masks_and_source_fees_through_title() {
    for edit in [false, true] {
        let bytes = packet(28, edit);
        let q = quota();
        let baseline = q.usage().unwrap();
        // External harness capture is explicitly paid through the consuming call.
        let capture = q.reserve(QuotaKind::Resident, bytes.len() as u64 * 2 + 128).unwrap();
        let starts = AtomicU64::new(0);
        let progress = |p: f32| {
            if p.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let selected = crate::import_owned_xcf_bytes_in_quota(
            "gaussian-group.xcf",
            bytes,
            &DecodeOptions { progress: Some(&progress), ..Default::default() },
            &limits(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 2, "immutable metadata preplan plus selected decode, without family retry");
        let crate::OwnedXcfImport::IsolatedGaussianGroup(imported) = selected else { panic!("sealed distinct source family") };
        assert!(imported.source_read_only() && !imported.preview_only());
        let owner = imported.document();
        assert!(owner.is_readonly_gaussian_group_storage());
        owner.validate_gaussian_group_storage(&ctl()).unwrap();
        let title = owner.try_with_document_name("continued.pcraft", &ctl()).unwrap();
        assert!(title.is_readonly_gaussian_group_storage());
        assert_eq!(owner.document().xcf_state, title.document().xcf_state);
        for (a, b) in leaves(owner.document()).into_iter().zip(leaves(title.document())) {
            assert_eq!(a, b);
            assert!(b.quota().shares_ledger(&q));
        }
        assert!(Arc::ptr_eq(&owner.document().layers[1].mask.as_ref().unwrap().surface, &title.document().layers[1].mask.as_ref().unwrap().surface));
        drop(imported);
        drop(capture);
        // Stores outlive the consumed decoder, source wrapper and caller capture.
        for leaf in leaves(title.document()) {
            let bounds = leaf.operative_bounds();
            let mut raw = [0; 16];
            leaf.checked_view().read_logical_pixel_native_into(bounds.x0, bounds.y0, &mut raw, &ctl()).unwrap();
        }
        drop(title);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN: mode61 source refusal and cumulative-work failures release actual preparation/codec fees"]
fn negative_mode_and_work_denial_do_not_retain_a_partial_bridge() {
    for edit in [false, true] {
        let q = quota();
        let bytes = packet(61, edit);
        let capture = q.reserve(QuotaKind::Resident, bytes.len() as u64 * 2 + 128).unwrap();
        let before = q.usage().unwrap();
        assert!(crate::import_owned_xcf_bytes_in_quota("negative.xcf", bytes, &Default::default(), &limits(), q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        drop(capture);
    }
    for cap in [0, 4096, 1 << 20] {
        let q = quota();
        let bytes = packet(28, false);
        let capture = q.reserve(QuotaKind::Resident, bytes.len() as u64 * 2 + 128).unwrap();
        let before = q.usage().unwrap();
        let used = AtomicU64::new(0);
        let work = |n| {
            let old = used.fetch_add(n, Ordering::Relaxed);
            if old.checked_add(n).is_some_and(|v| v <= cap) { Ok(()) } else { Err(photocraft_raster::backing::BackingError::Work) }
        };
        assert!(
            crate::import_owned_xcf_bytes_in_quota(
                "limited.xcf",
                bytes,
                &Default::default(),
                &limits(),
                q.clone(),
                &ReadControl { interrupt: &Interrupt::NONE, work: &work }
            )
            .is_err()
        );
        assert_eq!(q.usage().unwrap(), before);
        drop(capture);
    }
}
