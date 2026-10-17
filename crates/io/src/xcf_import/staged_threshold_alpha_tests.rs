//! UNRUN: staged policy/retention regressions; no additional source enrollment.
use super::super::staged_threshold_alpha::import_staged_threshold_alpha_bytes_with_budget as open;
use super::*;

fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default()).unwrap()
}
fn descriptor(role: PlaneRole, model: Model, alpha: bool, channels: usize) -> PlaneDescriptor {
    PlaneDescriptor { width: 8, height: 6, channels, sample: Sample::F32, source_sample: Sample::F32, model, alpha, role, record_offset: 0, offset: (0, 0) }
}

#[test]
fn private_policy_preserves_plain_role_and_closes_threshold_mask_native_layout() {
    let raster = descriptor(PlaneRole::Raster, Model::Rgb, true, 4);
    assert!(PlainPolicy::accepts(&raster));
    assert!(ThresholdAlphaPolicy::accepts(&raster));
    let mask = descriptor(PlaneRole::Mask, Model::Gray, false, 1);
    assert!(!PlainPolicy::accepts(&mask));
    assert!(ThresholdAlphaPolicy::accepts(&mask));
    let mut bad = mask.clone();
    bad.role = PlaneRole::Channel;
    assert!(!ThresholdAlphaPolicy::accepts(&bad));
    bad = mask.clone();
    bad.alpha = true;
    assert!(!ThresholdAlphaPolicy::accepts(&bad));
    bad = mask.clone();
    bad.channels = 2;
    assert!(!ThresholdAlphaPolicy::accepts(&bad));
    bad = mask.clone();
    bad.sample = Sample::U16;
    assert!(!ThresholdAlphaPolicy::accepts(&bad));
    bad = mask;
    bad.source_sample = Sample::F16;
    assert!(!ThresholdAlphaPolicy::accepts(&bad));
    assert!(!<StagedPlane<'_, '_, PlainPolicy> as MappingPlane>::requires_budgeted_rendering_preparation());
    assert!(<StagedPlane<'_, '_, ThresholdAlphaPolicy> as MappingPlane>::requires_budgeted_rendering_preparation());
}

#[test]
fn entry_cancel_and_spare_capacity_denial_refund_without_source_mutation() {
    let mut bytes = Vec::with_capacity(8192);
    bytes.extend_from_slice(b"gimp xcf v022\0");
    let bytes = Arc::new(bytes);
    let original = bytes.as_slice().to_vec();
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let cancelled = || true;
    assert!(matches!(open("test.xcf", bytes.clone(), &Interrupt::cancel_only(&cancelled), &DecodeOptions::default(), &budget), Err(IoError::Cancelled)));
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    let small = limits().lowered(&[("residentBytes", 4096)]).unwrap();
    let budget = photocraft_xcf::Budget::new(small, None);
    assert!(open("test.xcf", bytes.clone(), &Interrupt::NONE, &DecodeOptions::default(), &budget).is_err());
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Index), 0);
    assert_eq!(bytes.as_slice(), original);
}

#[test]
fn actual_mask_move_prepays_physical_padding_and_retains_registry_fee() {
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let surface_bytes = (photocraft_geom::TILE_SIZE * photocraft_geom::TILE_SIZE * 4) as u64;
    let token = Rc::new(LeaseToken {
        _surface: budget.reserve(BudgetKind::Resident, surface_bytes, "test real Surface envelope").unwrap(),
        _metadata: budget.reserve(BudgetKind::Index, 256, "test real plane header").unwrap(),
        budget: &budget,
    });
    let mut surface = Surface::new(format);
    surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap();
    let retained = token.clone();
    let mut plane = StagedPlane::<ThresholdAlphaPolicy> {
        descriptor: descriptor(PlaneRole::Mask, Model::Gray, false, 1),
        masked_mixed_plan: false,
        hierarchy_plan: false,
        hierarchy_coverage: false,
        storage: Storage::Ready(surface),
        _token: token,
        _policy: PhantomData,
    };
    let mut local = Budget { used: 0, max: usize::MAX, peak: 0, shared: None };
    let before = budget.used(BudgetKind::Work);
    let surface = plane.current_filter_mask_surface(Rect::new(0, 0, 8, 6), &mut local, &Interrupt::NONE).unwrap();
    assert_eq!(budget.used(BudgetKind::Work) - before, (surface_bytes / 4 + 1) * 32);
    assert!(plane.current_filter_mask_surface(Rect::new(0, 0, 8, 6), &mut local, &Interrupt::NONE).is_err());
    drop(plane);
    assert_eq!(budget.used(BudgetKind::Resident), surface_bytes + 256); // Index reservations also debit the inclusive Resident ledger.
    drop(surface);
    assert_eq!(budget.used(BudgetKind::Resident), surface_bytes + 256); // Index reservations also debit the inclusive Resident ledger.
    drop(retained);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Index), 0);
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: requires pinned independently authored Threshold Alpha precision corpus"]
fn qualified_saved_sources_retain_original_native_masks_and_fees_until_holder_drop() {
    use sha2::{Digest, Sha256};
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        let encoded = Arc::new(std::fs::read(root.join(&name)).unwrap());
        assert_eq!(hash(&encoded), manifest[&name].as_str().unwrap());
        let original = encoded.clone();
        let budget = photocraft_xcf::Budget::new_persistent(limits(), None).unwrap();
        let baseline = [BudgetKind::Resident, BudgetKind::Index, BudgetKind::Encoded].map(|kind| budget.used(kind));
        let candidate = open(&name, encoded, &Interrupt::NONE, &DecodeOptions::default(), &budget).unwrap();
        assert_eq!(candidate.original_bytes(), original.as_slice());
        let raw = photocraft_xcf::decode(&original, &DecodeOptions::default()).unwrap();
        let doc = candidate.document();
        let source = raw.layers[0].plane.as_ref().unwrap();
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), source.bytes);
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks[0].nodes[0].operation.schema_epoch, "xcf-22-stock-threshold-alpha-precision-v1");
        let mask = raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap();
        assert_eq!(state.filter_masks[0].surface.try_read_interleaved(doc.bounds()).unwrap(), mask.bytes);
        assert!(budget.used(BudgetKind::Resident) >= original.len() as u64);
        assert!(budget.used(BudgetKind::Index) > 0);
        assert_eq!(budget.used(BudgetKind::Encoded), original.len() as u64);
        // The real paid preparation API must refuse an exhausted actual source
        // ledger while the candidate's decoder/Surface receipts stay live.
        let held = budget.used(BudgetKind::Resident);
        let rest = budget.limits().get(BudgetKind::Resident) - held;
        let blocker = budget.reserve(BudgetKind::Resident, rest, "test deny preparation headroom").unwrap();
        let mut adapter = super::super::SharedPreparation { shared: &budget, failure: None };
        assert!(photocraft_compose::gimp_render::PreparedRenderer::new_accounted(doc, &Interrupt::NONE, &mut adapter, &candidate).is_err());
        assert!(adapter.failure.is_some());
        drop(blocker);
        assert_eq!(budget.used(BudgetKind::Resident), held);
        let stop = || true;
        let mut adapter = super::super::SharedPreparation { shared: &budget, failure: None };
        assert!(photocraft_compose::gimp_render::PreparedRenderer::new_accounted(doc, &Interrupt::cancel_only(&stop), &mut adapter, &candidate).is_err());
        assert_eq!(budget.used(BudgetKind::Resident), held);
        drop(original);
        assert!(!candidate.original_bytes().is_empty());
        let held = candidate.report_capacity_with_budget(&budget).unwrap();
        let mut extra = budget.reserve(BudgetKind::Index, held.max(1), "test real report owner").unwrap();
        let owned = candidate.detach_owned(&mut [&mut extra], &budget).unwrap();
        assert!(!owned.encoded.is_empty());
        assert!(budget.used(BudgetKind::Resident) > 0);
        assert_eq!(extra.bytes(), 0, "actual caller receipt moved into owned batch");
        drop(owned);
        for (kind, before) in [BudgetKind::Resident, BudgetKind::Index, BudgetKind::Encoded].into_iter().zip(baseline) {
            assert_eq!(budget.used(kind), before);
        }
    }
}
