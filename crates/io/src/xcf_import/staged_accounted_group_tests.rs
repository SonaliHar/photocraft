//! UNRUN pinned original-group source staging; harness captures are external.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
pub(in crate::xcf_import) fn records() -> (std::path::PathBuf, Vec<serde_json::Value>) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("pinned full corpus")).join("accounted-normal-group-evidence");
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "2ec12176aa53eed6a4d65e21802b1ef8c2075613a14101beaf5ccba478e46b53");
    let packet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let rows = packet["records"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 24);
    (root, rows)
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()
}
pub(in crate::xcf_import) fn read(root: &std::path::Path, row: &serde_json::Value) -> Arc<Vec<u8>> {
    let bytes = std::fs::read(root.join(row["source"]["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["source"]["sha256"].as_str().unwrap());
    assert_eq!(bytes.len() as u64, row["source"]["bytes"].as_u64().unwrap());
    Arc::new(bytes)
}
fn find<'a>(layers: &'a [Layer], name: &str) -> Option<&'a Layer> {
    for layer in layers {
        if layer.name == name {
            return Some(layer);
        }
        if let LayerContent::Group(group) = &layer.content {
            if let Some(found) = find(&group.children, name) {
                return Some(found);
            }
        }
    }
    None
}
#[test]
#[ignore = "UNRUN: all24 pinned sources, one staged decode, no group projection, exact three leaves/mask, genuine receipt transfer"]
fn original_group_single_decode_and_authoritative_native_custody() {
    let (root, rows) = records();
    for row in rows {
        let bytes = read(&root, &row);
        let original = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert!(accounted_group::qualifies(&original, &Interrupt::NONE).unwrap());
        assert!(original.layers[0].plane.is_none());
        let weak = Arc::downgrade(&bytes);
        let budget = photocraft_xcf::Budget::new_persistent(limits(), None).unwrap();
        let kinds = [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index];
        let baseline = kinds.map(|k| budget.used(k));
        let starts = AtomicU64::new(0);
        let progress = |v: f32| {
            if v.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let staged = staged_selected_source::import_staged_selected_bytes_with_budget("group.xcf", bytes, &Interrupt::NONE, &options, &budget).unwrap();
        assert_eq!(staged.family(), staged_source::SelectedStagedFamily::IsolatedNormalGroup);
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        let doc = staged.document();
        let group = find(&doc.layers, &original.layers[0].name).unwrap();
        assert!(matches!(&group.content, LayerContent::Group(g) if g.children.len()==2));
        assert!(group.surface().is_none(), "no synthesized group projection");
        for source in original.layers.iter().skip(1) {
            let layer = find(&doc.layers, &source.name).unwrap();
            assert_eq!(layer.surface().unwrap().try_read_interleaved(bounds(source).unwrap()).unwrap(), source.plane.as_ref().unwrap().bytes);
        }
        if let Some(mask) = &original.layers[0].mask {
            let actual = group.mask.as_ref().unwrap();
            assert_eq!(actual.surface.try_read_interleaved(bounds(&original.layers[0]).unwrap()).unwrap(), mask.plane.as_ref().unwrap().bytes);
        } else {
            assert!(group.mask.is_none());
        }
        assert!(doc.xcf_state.as_ref().unwrap().items.iter().filter_map(|x| x.composition.as_ref()).all(|x| x.compatibility_epoch == accounted_group::EPOCH));
        let reports = staged.report_capacity_with_budget(&budget).unwrap();
        let mut fee = budget.reserve(BudgetKind::Index, reports.max(1), "test reports").unwrap();
        let owned = staged.detach_owned(&mut [&mut fee], &budget).unwrap();
        assert_eq!(fee.bytes(), 0);
        assert!(weak.upgrade().is_some());
        drop(owned);
        assert!(weak.upgrade().is_none());
        for (kind, value) in kinds.into_iter().zip(baseline) {
            assert_eq!(budget.used(kind), value);
        }
    }
}
#[test]
#[ignore = "UNRUN: shared eager/controlled predicate equality and exact source-field refusals"]
fn exact_group_predicate_preserves_old_source_controls() {
    let (root, rows) = records();
    let bytes = read(&root, &rows[0]);
    let source = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    for change in 0..16 {
        let mut file = source.clone();
        match change {
            0 => file.version = 22,
            1 => file.encoding = Encoding::Linear,
            2 => file.precision = 675,
            3 => file.icc.clear(),
            4 => file.layers[0].mode = 61,
            5 => file.layers[0].composite_mode = 2,
            6 => file.layers[1].offset = (5, 3),
            7 => file.layers[1].path = vec![0, 1],
            8 => file.layers[0].opacity = f32::NAN,
            9 => file.layers[1].visible = false,
            10 => file.layers[0].mask_shown = true,
            11 => file.layers[0].mask_editing = true,
            12 => file.layers[0].mask.as_mut().unwrap().offset = (1, 0),
            13 => file.layers[3].opacity = 0.5,
            14 => file.layers[2].group = true,
            _ => file.layers[0].plane = file.layers[3].plane.clone(),
        }
        let budget = photocraft_xcf::Budget::new(limits(), None);
        assert!(!accounted_group::qualifies(&file, &Interrupt::NONE).unwrap());
        assert!(!accounted_group::qualifies_with(&file, &Interrupt::NONE, Some(&budget)).unwrap());
        for kind in [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index] {
            assert_eq!(budget.used(kind), 0);
        }
    }
}
#[test]
#[ignore = "UNRUN: cancellation, actual work observer refusal, capacity failure destroy payload before actual source fees"]
fn private_failure_refunds_real_fees_without_retry() {
    let (root, rows) = records();
    let bytes = read(&root, &rows[0]);
    for mode in 0..3 {
        let stop = || mode == 0;
        let work = |_| Err(XcfError { kind: ErrorKind::Cancelled, offset: 0, item: String::new(), message: "parent work refusal".into() });
        let small = limits().lowered(&[("residentBytes", 4096)]).unwrap();
        let budget =
            photocraft_xcf::Budget::new_persistent_with_work(if mode == 2 { small } else { limits() }, None, if mode == 1 { Some(&work) } else { None })
                .unwrap();
        let baseline = [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index].map(|k| budget.used(k));
        let control = Interrupt::cancel_only(&stop);
        let result = staged_selected_source::import_staged_selected_bytes_with_budget("group.xcf", bytes.clone(), &control, &Default::default(), &budget);
        assert!(result.is_err());
        if mode < 2 {
            assert!(matches!(result, Err(IoError::Cancelled)));
        }
        for (kind, used) in [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index].into_iter().zip(baseline) {
            assert_eq!(budget.used(kind), used);
        }
        assert_eq!(hash(&bytes), rows[0]["source"]["sha256"].as_str().unwrap());
    }
}
