//! UNRUN paid synthetic source-record association contracts, not original-XCF import/projection or provenance recovery.
use super::*;
use photocraft_doc::xcf_provenance::OpaqueRecord;
fn record(owner: ItemRef, tag: u32, value: u32, order: u32, offset: u64) -> OpaqueRecord {
    let bytes = Vec::from(value.to_be_bytes());
    let digest = blake3::hash(&bytes).to_hex().to_string();
    OpaqueRecord {
        owner: Some(owner),
        namespace: "gimp/xcf/source-property".into(),
        identity: format!("{tag}@{offset}:4"),
        source_version: 22,
        order,
        flags: 0,
        bytes: Arc::new(bytes),
        blake3: digest,
        rendering_required: false,
    }
}
fn groups(f: &mut Fixture) {
    // Original ordered fixture holds prospective Index8MiB+Resident8MiB BEFORE these small record/String/Arc allocations.
    let state = f.document.xcf_state.as_mut().unwrap();
    state.opaque.clear();
    let mut offset = 1000u64;
    for layer in &f.document.layers {
        let owner = ItemRef::Layer(layer.id);
        for (order, (tag, value)) in [(7, 28), (33, layer.opacity.to_bits())].into_iter().enumerate() {
            state.opaque.push(record(owner, tag, value, order as u32, offset));
            offset += 32;
        }
        if let Some(stack) = state.filter_stacks.iter().find(|s| s.owner == owner) {
            for node in &stack.nodes {
                for (order, (tag, value)) in
                    [(7, 62), (33, (f64::from_bits(node.opacity_f64_bits) as f32).to_bits()), (8, u32::from(node.enabled))].into_iter().enumerate()
                {
                    state.opaque.push(record(owner, tag, value, order as u32, offset));
                    offset += 32;
                }
            }
        }
    }
}
fn rewrite_word(raw: &mut OpaqueRecord, word: u32) {
    raw.bytes = Arc::new(Vec::from(word.to_be_bytes()));
    raw.blake3 = blake3::hash(&raw.bytes).to_hex().to_string();
}
#[test]
#[ignore = "UNRUN sixkernels+loaded actualDoc SourceOpacity grouping/promotion/declaredmix vs strictSourceunit, signedzero/nonunit neighbor and oldL1SOURCEpromotion; syntheticrecords only, actualoriginalXCF ownedIO/hosts"]
fn source_opacity_preparation_and_current_continuation() {
    let (nl, linear) = parts();
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in 0..7 {
            for amount in [0., -0., 0.375] {
                for enabled in [true, false] {
                    let mut f = fixture_opacity(sample, kind, amount, enabled);
                    groups(&mut f);
                    let source = f.document.xcf_state.as_ref().unwrap().opaque.clone();
                    let q = f.quota.clone();
                    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                    let paid =
                        LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap();
                    drop(paid);
                    let baseline = q.usage().unwrap();
                    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
                    assert_eq!(q.usage().unwrap(), baseline);
                    // This is the unchanged SOURCE structural factory, not Current adoption as source authority.
                    let (o, _, _, _) = adopt(f);
                    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                    c.validate_operative(&ctl()).unwrap();
                    for x in 0u32..5 {
                        let p = input(0, x, 0, sample);
                        let expected = if enabled { mix_reference(p, old_leaf(kind, p, false, &nl, &linear), amount) } else { p };
                        assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                    }
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().opaque, source);
                }
            }
        }
    }
    let mut f = fixture_opacity(SampleType::F64, 0, 1., true);
    groups(&mut f);
    let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_ok());
    // A Current valid loadedV2 strictzero edit must NOT become an original V2 source role.
    let mut f = fixture_opacity(SampleType::F64, 6, 0.375, true);
    let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.arguments = vec![number("exposure", 0.5), number("black-level", 0.)];
    groups(&mut f);
    let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
    let (o, _, _, _) = current(f);
    Context::new(&o, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).unwrap();
    let mut f = ordered(SampleType::F64, &[&[0]], true);
    f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = 0.375f64.to_bits();
    groups(&mut f);
    let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
    let mut f = ordered(SampleType::F64, &[&[0], &[]], true);
    f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.clear();
    f.document.xcf_state.as_mut().unwrap().filter_masks.clear();
    let owners: Vec<_> = f.document.layers.iter().map(|l| ItemRef::Layer(l.id)).collect();
    f.document.xcf_state.as_mut().unwrap().items.retain(|r| owners.contains(&r.item));
    groups(&mut f);
    let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
}
#[test]
#[ignore = "UNRUN sourceproperty group duplicate/overlap/order/owner/tag33/hash/promotion/legacy/cap negatives beforeproviderreads; no false equal-opacityoperation reorder reconstruction"]
fn raw_groups_are_not_a_value_multiset() {
    for fault in 0..13 {
        let mut f = fixture_opacity(SampleType::F64, 5, 0.375, true);
        groups(&mut f);
        let state = f.document.xcf_state.as_mut().unwrap();
        match fault {
            0 => rewrite_word(&mut state.opaque[3], 0.5f32.to_bits()),
            1 => {
                state.opaque[3].identity = "33@1096:8".into();
            }
            2 => {
                state.opaque[3].blake3 = "0".repeat(64);
            }
            3 => {
                state.opaque[3].order = 2;
            }
            4 => {
                state.opaque[3].identity = "033@1096:4".into();
            }
            5 => {
                state.opaque[3].owner = Some(ItemRef::Layer(LayerId::fresh()));
            }
            6 => {
                state.opaque[3].identity = "33@18446744073709551615:4".into();
            }
            7 => {
                state.filter_stacks[0].nodes[0].opacity_f64_bits = f64::from_bits(0.375f64.to_bits() + 1).to_bits();
            }
            8 => rewrite_word(&mut state.opaque[3], f32::NAN.to_bits()),
            9 => {
                state.opaque.remove(3);
            }
            10 => {
                let owner = state.filter_stacks[0].owner;
                state.opaque.insert(5, record(owner, 6, 255, 3, 9999));
            }
            11 => {
                state.opaque[3].identity = "33@1000:4".into();
            }
            _ => {
                let raw = state.opaque[0].clone();
                state.opaque.resize(4097, raw);
            }
        }
        let q = f.quota.clone();
        let before = q.usage().unwrap();
        let reads = f.reads.load(Ordering::Relaxed);
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(
            LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err(),
            "fault{fault}"
        );
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(f.reads.load(Ordering::Relaxed), reads);
    }
    // Two equal-opacity zero-arg nodes have indistinguishable values, but repeated physical source intervals must refuse.
    let mut f = ordered(SampleType::F64, &[&[0, 0], &[]], true);
    for n in &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes {
        set_kind(n, 5);
        n.opacity_f64_bits = 0.375f64.to_bits();
    }
    groups(&mut f);
    let state = f.document.xcf_state.as_mut().unwrap();
    for j in 0..3 {
        state.opaque[5 + j] = state.opaque[2 + j].clone();
    }
    let q = f.quota.clone();
    let before = q.usage().unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    // Swapping equal-opacity operation names is not reconstructible from these raw fields; the sealed original mapper is responsible for actual node association.
}
#[test]
#[ignore = "UNRUN actualsource allinactive still fullnative/mask/profile/parameterproof, genuinegroupIndex+scratch/work/cancel/sourcelease refunds; no perrootglobalrescan or freeCurrentfactory"]
fn inactive_source_proof_and_control_are_unconditional() {
    for fault in 0..4 {
        let mut f = fixture_opacity(SampleType::F64, 6, -0., false);
        if fault == 3 {
            f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), None], Some(0));
            let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
            let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
            f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
            let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            set_kind(node, 6);
            node.opacity_f64_bits = (-0f64).to_bits();
            node.enabled = false;
        }
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        match fault {
            0 => {
                f.document.xcf_state.as_mut().unwrap().filter_masks[0]
                    .surface
                    .try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes())
                    .unwrap();
            }
            1 => {
                f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
            }
            2 => {
                f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "9:9".into();
            }
            _ => {}
        }
        groups(&mut f);
        let q = f.quota.clone();
        let before = q.usage().unwrap();
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        if fault == 3 {
            // Source preparation defers backed provider reads to mandatory
            // operative promotion; exercise that actual phase for native NaN.
            drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
            assert_eq!(q.usage().unwrap(), before);
            let (owner, _, _, _) = current(f);
            let c = Context::new(&owner, Default::default(), &ctl()).unwrap();
            assert!(c.validate_operative(&ctl()).is_err());
        } else {
            assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
    }
    let mut f = fixture_opacity(SampleType::F64, 2, 0.375, true);
    groups(&mut f);
    let q = f.quota.clone();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
    let used = b.used;
    let baseline = q.usage().unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: used - 1 };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let observe = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let i = Interrupt::cancel_only(&observe);
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &i, &mut b, f.retention.clone()).unwrap());
    let total = polls.load(Ordering::Relaxed);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= total - 1;
    let i = Interrupt::cancel_only(&cancel);
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &i, &mut b, f.retention.clone()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    f.rasters[1].clear_cache().unwrap();
    let before = q.usage().unwrap();
    f.fails[1].store(true, Ordering::Relaxed);
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    // The sealed prepared-source object has no renderer/publication API. Real
    // source IO already visited native bytes; a preexisting BackedRaster is
    // revisited by mandatory CURRENT operative proof before Session handoff.
    drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
    assert_eq!(q.usage().unwrap(), before);
    f.fails[1].store(false, Ordering::Relaxed);
    let (owner, rasters, fails, _) = current(f);
    // Warm the same real owner before the cold OTHER-provider failure baseline.
    let warm = Context::new(&owner, Default::default(), &ctl()).unwrap();
    warm.validate_operative(&ctl()).unwrap();
    drop(warm);
    rasters[1].clear_cache().unwrap();
    let before = q.usage().unwrap();
    fails[1].store(true, Ordering::Relaxed);
    let context = Context::new(&owner, Default::default(), &ctl()).unwrap();
    assert!(matches!(
        context.validate_operative(&ctl()),
        Err(E::Backing(BackingError::Io)) | Err(E::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))
    ));
    drop(context);
    assert_eq!(q.usage().unwrap(), before);
    fails[1].store(false, Ordering::Relaxed);
}

#[path = "backed_filtered_flat_point_affine_tests.rs"]
mod affine;

#[path = "backed_filtered_flat_point_stock_rgb_tests.rs"]
mod stock_rgb;
