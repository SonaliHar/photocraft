//! UNRUN source-selected dispatch/custody regressions. No family retry or
//! default/source-eligibility expansion is established by these drafts.
use super::super::{plane::fixture, staged_selected_source::import_staged_selected_bytes_with_budget as selected};
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()
}
fn effect(operation: &str) -> Vec<u8> {
    let mut record = Vec::new();
    for text in ["owned discriminator", "", operation, "0:0"] {
        fixture::u32be(&mut record, text.len() as u32 + 1);
        record.extend_from_slice(text.as_bytes());
        record.push(0);
    }
    fixture::prop(&mut record, 0, &[]);
    fixture::pointer(&mut record, 0, 22);
    record
}
#[test]
fn deterministic_original_family_and_dynamic_prep_keep_old_constants_closed() {
    let bytes = fixture::image(22, 650, 0, 8, 6, &0.5f32.to_be_bytes().repeat(48 * 4));
    let mut file = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::Plain);
    file.layers[0].effects.push(photocraft_xcf::Effect {
        name: String::new(),
        icon: String::new(),
        operation: "not-qualified".into(),
        version: None,
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: Default::default(),
    });
    assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::ThresholdAlpha, "selection does not pretend operation eligibility");
    file.layers.push(Default::default());
    assert_eq!(SourceSelectedPolicy::family(&file), SelectedStagedFamily::Plain, "non-singleton goes directly to old strict plain gate");
    for flag in [false, true] {
        assert!(!<StagedPlane<'_, '_, PlainPolicy> as MappingPlane>::requires_budgeted_rendering_preparation_for(flag));
        assert!(<StagedPlane<'_, '_, ThresholdAlphaPolicy> as MappingPlane>::requires_budgeted_rendering_preparation_for(flag));
        assert_eq!(<StagedPlane<'_, '_, SourceSelectedPolicy> as MappingPlane>::requires_budgeted_rendering_preparation_for(flag), flag);
    }
    let mask = PlaneDescriptor {
        width: 8,
        height: 6,
        channels: 1,
        sample: Sample::F32,
        source_sample: Sample::F32,
        model: Model::Gray,
        alpha: false,
        role: PlaneRole::Mask,
        record_offset: 0,
        offset: (0, 0),
    };
    assert!(SourceSelectedPolicy::accepts(&mask));
    assert!(!PlainPolicy::accepts(&mask));
    let mut channel = mask;
    channel.role = PlaneRole::Channel;
    assert!(!SourceSelectedPolicy::accepts(&channel));
}
#[test]
fn plain_decodes_once_preserves_native_words_and_transfers_actual_receipts() {
    for (precision, file_pixel, native_pixel) in [
        (150, vec![31, 61, 91, 255], vec![31, 61, 91, 255]),
        (
            650,
            [0.25f32, 0.5, 0.75, 1.].into_iter().flat_map(f32::to_be_bytes).collect(),
            [0.25f32, 0.5, 0.75, 1.].into_iter().flat_map(f32::to_ne_bytes).collect(),
        ),
    ] {
        let bytes = Arc::new(fixture::image(13, precision, 0, 1, 1, &file_pixel));
        let weak = Arc::downgrade(&bytes);
        let budget = photocraft_xcf::Budget::new_persistent(limits(), None).unwrap();
        let baseline = [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index].map(|kind| budget.used(kind));
        let starts = AtomicU64::new(0);
        let progress = |p: f32| {
            if p.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let result = selected("same.xcf", bytes, &Interrupt::NONE, &options, &budget).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 2, "one metadata preplan and one decode, no second-family decode");
        assert_eq!(result.family(), SelectedStagedFamily::Plain);
        assert_eq!(result.document().layers[0].surface().unwrap().try_read_interleaved(result.document().bounds()).unwrap(), native_pixel);
        assert!(result.document().xcf_state.as_ref().unwrap().filter_stacks.is_empty());
        let reports = result.report_capacity_with_budget(&budget).unwrap();
        let mut report_fee = budget.reserve(BudgetKind::Index, reports.max(1), "test actual report custody").unwrap();
        let owned = result.detach_owned(&mut [&mut report_fee], &budget).unwrap();
        assert!(weak.upgrade().is_some());
        assert_eq!(report_fee.bytes(), 0);
        assert!(!owned.encoded.is_empty());
        drop(owned);
        assert!(weak.upgrade().is_none());
        for (kind, used) in [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index].into_iter().zip(baseline) {
            assert_eq!(budget.used(kind), used);
        }
    }
}
#[test]
fn unqualified_effect_selects_neighborhood_then_refuses_without_second_decode_or_plain_retry() {
    let options = fixture::FixtureOptions { effect_records: vec![effect("gegl:gaussian-blur")], ..Default::default() };
    let bytes = Arc::new(fixture::image_custom(22, 650, 0, 8, 6, &0.5f32.to_be_bytes().repeat(48 * 4), &options));
    let parsed = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(SourceSelectedPolicy::family(&parsed), SelectedStagedFamily::Neighborhood(NeighborhoodSourceFamily::RasterFinite));
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let starts = AtomicU64::new(0);
    let progress = |p: f32| {
        if p.to_bits() == 0.05f32.to_bits() {
            starts.fetch_add(1, Ordering::Relaxed);
        }
    };
    let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
    let result = selected("effect.xcf", bytes, &Interrupt::NONE, &options, &budget);
    assert!(matches!(result,Err(IoError::Unsupported(ref s)) if s.contains("staged neighborhood source is outside its exact whole-file predicate")));
    assert_eq!(starts.load(Ordering::Relaxed), 2);
    for kind in [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index] {
        assert_eq!(budget.used(kind), 0);
    }
}
#[test]
fn entry_cancel_and_capacity_failure_keep_source_bytes_and_refund_actual_fees() {
    let mut bytes = Vec::with_capacity(8192);
    bytes.extend_from_slice(&fixture::image(13, 150, 0, 1, 1, &[1, 2, 3, 255]));
    let bytes = Arc::new(bytes);
    let original = bytes.as_slice().to_vec();
    let small = limits().lowered(&[("residentBytes", 4096)]).unwrap();
    let budget = photocraft_xcf::Budget::new(small, None);
    assert!(selected("held.xcf", bytes.clone(), &Interrupt::NONE, &DecodeOptions::default(), &budget).is_err());
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let stop = || true;
    assert!(matches!(selected("held.xcf", bytes.clone(), &Interrupt::cancel_only(&stop), &DecodeOptions::default(), &budget), Err(IoError::Cancelled)));
    assert_eq!(bytes.as_slice(), original);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}
#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: requires pinned independently authored Threshold Alpha precision packet"]
fn selected_threshold_native_source_and_mask_match_existing_typed_entry_with_one_decode() {
    use sha2::{Digest, Sha256};
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        let bytes = Arc::new(std::fs::read(root.join(&name)).unwrap());
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let starts = AtomicU64::new(0);
        let progress = |p: f32| {
            if p.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let budget = photocraft_xcf::Budget::new(limits(), None);
        let result = selected(&name, bytes.clone(), &Interrupt::NONE, &options, &budget).unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 2);
        assert_eq!(result.family(), SelectedStagedFamily::ThresholdAlpha);
        let other_budget = photocraft_xcf::Budget::new(limits(), None);
        let typed = super::super::staged_threshold_alpha::import_staged_threshold_alpha_bytes_with_budget(
            &name,
            bytes.clone(),
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &other_budget,
        )
        .unwrap();
        assert_eq!(result.original_bytes(), bytes.as_slice());
        let a = result.document();
        let b = typed.document();
        assert_eq!(
            a.layers[0].surface().unwrap().try_read_interleaved(a.bounds()).unwrap(),
            b.layers[0].surface().unwrap().try_read_interleaved(b.bounds()).unwrap()
        );
        assert_eq!(a.xcf_state.as_ref().unwrap().filter_masks[0].surface, b.xcf_state.as_ref().unwrap().filter_masks[0].surface);
        drop(result);
        for kind in [BudgetKind::Encoded, BudgetKind::Resident, BudgetKind::Index] {
            assert_eq!(budget.used(kind), 0);
        }
    }
}
