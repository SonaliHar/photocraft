//! UNRUN exact-CMS consumer regressions; no source qualification or size claim.
use super::*;
use photocraft_raster::backing::quota::QuotaLimits;

fn transform() -> Arc<photocraft_cms::Transform> {
    Arc::new(
        photocraft_cms::Transform::with_options(photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Builtin::LabD50.profile(), Default::default())
            .unwrap(),
    )
}
fn context(to_lab: Arc<photocraft_cms::Transform>, grid: bool) -> QueryContext {
    QueryContext { to_lab, grid, foreground: [0.2, 0.3, 0.4, 1.0], backed: None }
}
fn prepared(transform: &photocraft_cms::Transform) -> photocraft_cms::transform::PreparedRgbLabF32<'_> {
    transform.prepare_rgb_lab_f32_with_control(Default::default(), &|| false).unwrap()
}
fn owner() -> Arc<Retention> {
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 8 << 20, scratch: 0 });
    let doc = Document::new("private schedule fixture", photocraft_geom::Size::new(513, 1), photocraft_color::ColorMode::Rgb, SampleType::F32);
    Arc::new(Retention {
        source: DocumentSnapshot::from_resident(Arc::new(doc)).unwrap(),
        revision: 1,
        active: None,
        view: Default::default(),
        foreground: [0.2, 0.3, 0.4, 1.],
        work: Arc::new(Work { used: AtomicU64::new(0) }),
        _header: quota.reserve(QuotaKind::Index, 4096).unwrap(),
        _profile: quota.reserve(QuotaKind::Resident, 1 << 20).unwrap(),
        _transform: quota.reserve(QuotaKind::Resident, 1 << 20).unwrap(),
        _query: quota.reserve(QuotaKind::Resident, 65536).unwrap(),
        quota,
    })
}

#[test]
fn exact_point_recipe_keeps_legacy_grid_order_clamp_and_lazy_foreground() {
    let transform = transform();
    let plan = prepared(&transform);
    let plus = [sel::RangeSample { color: [-0.1, 0.2, 1.2], at: Some((1., 0.)) }];
    let minus = [sel::RangeSample { color: [0.5, 0.4, 0.3], at: Some((2., 0.)) }];
    for grid in [false, true] {
        let context = context(transform.clone(), grid);
        for params in [json!({}), json!({"colors":[[0.1,0.2,0.3]]}), json!({"order":["-","+"],"localized":true})] {
            let (adds, subtracts): (&[sel::RangeSample], &[sel::RangeSample]) = if params.get("order").is_some() { (&plus, &minus) } else { (&[], &[]) };
            let old = with_context(&context, &params, Rect::new(0, 0, 513, 1), adds, subtracts, &Interrupt::NONE).unwrap();
            let new = with_context_conversion(&context, &params, Rect::new(0, 0, 513, 1), adds, subtracts, &Interrupt::NONE, Some((&plan, Default::default())))
                .unwrap();
            assert_eq!(old.range.lo.map(f32::to_bits), new.range.lo.map(f32::to_bits));
            assert_eq!(old.range.hi.map(f32::to_bits), new.range.hi.map(f32::to_bits));
        }
    }
}

#[test]
fn exact_512_chunks_keep_legacy_coverage_alpha_edges_localization_and_remaining_work() {
    let transform = transform();
    let plan = prepared(&transform);
    let query = with_context(
        &context(transform.clone(), false),
        &json!({"fuzziness":40,"localized":true}),
        Rect::new(0, 0, 513, 1),
        &[sel::RangeSample { color: [0.2, 0.3, 0.4], at: Some((17., 0.)) }],
        &[],
        &Interrupt::NONE,
    )
    .unwrap();
    let pixels: Vec<_> = (0..513).map(|i| [if i % 2 == 0 { -0.1 } else { 1.2 }, 0.3, 0.4, if i == 512 { f32::from_bits(0x7fc00123) } else { 0.75 }]).collect();
    let old = coverage_inner(&query, &pixels, 513, 0.5, [13., 0.], Default::default(), &Interrupt::NONE).unwrap();
    let (total, cms) = coverage_work(&query, pixels.len(), Some((&plan, Default::default())), &Interrupt::NONE).unwrap();
    assert_eq!(total - cms, 513 * (512 + 48));
    let exact = coverage_inner_conversion(
        &query,
        &pixels,
        513,
        0.5,
        [13., 0.],
        sel::ColorRangeLimits { work_units: total, ..Default::default() },
        &Interrupt::NONE,
        Some((&plan, Default::default())),
    )
    .unwrap();
    assert_eq!(old.iter().map(|p| p.to_bits()).collect::<Vec<_>>(), exact.iter().map(|p| p.to_bits()).collect::<Vec<_>>());
    assert!(
        coverage_inner_conversion(
            &query,
            &pixels,
            513,
            1.,
            [0., 0.],
            sel::ColorRangeLimits { work_units: total - 1, ..Default::default() },
            &Interrupt::NONE,
            Some((&plan, Default::default()))
        )
        .is_err()
    );
}

#[test]
fn prepare_and_each_owned_coverage_debit_same_parent_without_reset_or_refund() {
    let owner = owner();
    let transform = transform();
    let mut query =
        with_context(&context(transform.clone(), false), &json!({"color":[0.2,0.3,0.4]}), Rect::new(0, 0, 513, 1), &[], &[], &Interrupt::NONE).unwrap();
    query.backed = Some(owner.clone());
    let parent_units = AtomicU64::new(0);
    let count = |n| {
        parent_units.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let parent = ReadControl { interrupt: &Interrupt::NONE, work: &count };
    let baseline = owner.quota.usage().unwrap();
    let pixels = [[0.2, 0.3, 0.4, 1.]; 2];
    for _ in 0..2 {
        let before = owner.work.used.load(Ordering::Relaxed);
        let plan = prepared(&transform);
        let (expected, _) = coverage_work(&query, 2, Some((&plan, Default::default())), &Interrupt::NONE).unwrap();
        let coverage = owned_coverage(&query, &pixels, 2, 1., [0., 0.], Default::default(), &Interrupt::NONE, Some(&parent)).unwrap();
        assert_eq!(owner.work.used.load(Ordering::Relaxed) - before, expected + photocraft_cms::Transform::RGB_LAB_F32_PREPARATION_WORK);
        assert_eq!(parent_units.load(Ordering::Relaxed), owner.work.used.load(Ordering::Relaxed));
        assert!(owner.quota.usage().unwrap().resident > baseline.resident);
        drop(coverage);
        assert_eq!(owner.quota.usage().unwrap(), baseline);
    }
    let before = owner.work.used.load(Ordering::Relaxed);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(exact_plan(&transform, &owner, owner._query.bytes(), &Interrupt::NONE, Some(&deny)).is_err());
    assert_eq!(owner.work.used.load(Ordering::Relaxed) - before, photocraft_cms::Transform::RGB_LAB_F32_PREPARATION_WORK);
    assert_eq!(owner.quota.usage().unwrap(), baseline);
}
