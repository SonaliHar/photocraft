//! UNRUN paid synthetic U32 native tree; no producer source or pixel oracle.
//! Expected code arrays/WorkingSpace are external harness, not quota receipts.
use super::*;
const CODES: [u32; 7] = [0, 1, 0x7fffffff, 0x80000000, 0x80000001, u32::MAX - 1, u32::MAX];
fn codes(which: usize, x: u32, y: u32) -> [u32; 4] {
    let phase = (x + y) as usize;
    [CODES[(phase + which) % 7], CODES[(phase + 2 * which + 1) % 7], CODES[(phase + 3 * which + 2) % 7], CODES[phase % 7]]
}
fn values(which: usize, x: u32, y: u32) -> [f64; 4] {
    codes(which, x, y).map(|v| f64::from(v) / f64::from(u32::MAX))
}
fn u32_fixture(size: Size, version: u32) -> Fixture {
    let mut f = fixture(SampleType::U32, size, false);
    f.doc.source_color_context.as_mut().unwrap().source_precision = 350;
    let s = f.doc.xcf_state.as_mut().unwrap();
    for stack in &mut s.filter_stacks {
        stack.nodes.clear();
    }
    s.filter_masks.clear();
    s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    s.provenance.format_version = version;
    let q = f.q.clone();
    let format = f.doc.pixel_format();
    let bounds = f.doc.bounds();
    // Existing prospective model8MiB+Index8MiB covers map/row replacements.
    // Each actual new native buffer separately reserves2*len+header BEFORE allocation.
    fn install(
        layers: &mut [Layer],
        next: &mut usize,
        format: PixelFormat,
        bounds: Rect,
        q: &BackingQuota,
        rasters: &mut [BackedRaster],
        fails: &[Arc<AtomicBool>],
    ) {
        for l in layers {
            if let LayerContent::Group(g) = &mut l.content {
                install(&mut g.children, next, format, bounds, q, rasters, fails);
            } else {
                let which = *next;
                let len = bounds.width() as usize * bounds.height() as usize * 16;
                let fee = q.reserve(QuotaKind::Resident, (len as u64) * 2 + 512).unwrap();
                let mut bytes = vec![0; len];
                for y in 0..bounds.height() {
                    for x in 0..bounds.width() {
                        let at = ((y * bounds.width() + x) as usize) * 16;
                        for (ch, word) in codes(which, x, y).iter().enumerate() {
                            bytes[at + ch * 4..at + ch * 4 + 4].copy_from_slice(&word.to_ne_bytes());
                        }
                    }
                }
                let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 16]).unwrap();
                let store = Arc::new(Store { descriptor, bytes, q: q.clone(), fail: Arc::clone(&fails[which]), _fee: fee });
                let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
                l.content = LayerContent::BackedRaster(BackedLayer::new(raster.clone(), descriptor, &ctl()).unwrap());
                rasters[which] = raster;
                *next += 1;
            }
        }
    }
    let mut count = 0;
    install(&mut f.doc.layers, &mut count, format, bounds, &q, &mut f.rasters, &f.fails);
    assert_eq!(count, 3);
    reindex(&mut f);
    f
}
fn adopt_u32(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    drop(f.template);
    let o = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).unwrap();
    (o, f.rasters, f.fails)
}
fn expected(x: u32, y: u32) -> [f64; 4] {
    let w = WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
        .unwrap();
    let bottom = w.normal([0.; 4], values(0, x, y), 0.5, GimpOperator::Union).unwrap();
    let middle = w.normal([0.; 4], values(1, x, y), 0.75, GimpOperator::Union).unwrap();
    let inner = w.normal([0.; 4], values(2, x, y), 1., GimpOperator::Union).unwrap();
    let outer = w.normal(middle, inner, 0.5, GimpOperator::Union).unwrap();
    w.normal(bottom, outer, 0.375, GimpOperator::Union).unwrap()
}
#[test]
#[ignore = "UNRUN U32 exact native boundarycodes including adjacent midpoint/near1alpha through version12/13/22 actual nested Normal; no F32 read, PointPlan or rewritten native storage"]
fn u32_native_codes_remain_distinct_in_nested_wide_normal() {
    for version in [12, 13, 22] {
        let fixture = u32_fixture(Size::new(8, 6), version);
        let anchors = fixture.doc.xcf_state.as_ref().unwrap().filter_stacks.clone(); // External expected containers under original model prehold.
        let (o, rasters, _) = adopt_u32(fixture);
        let ctx = Context::new_current(&o, Default::default(), &ctl()).unwrap();
        ctx.validate_operative(&ctl()).unwrap();
        assert!(ctx.plan.nodes.iter().filter_map(|n| n.leaf.as_ref()).all(|l| l.point.is_none()));
        let mut adjacent = false;
        for x in 0..7 {
            for (which, raster) in rasters.iter().enumerate() {
                let mut raw = [0; 16];
                raster.read_pixel_into(x, 0, &mut raw, &ctl()).unwrap();
                for (ch, code) in codes(which, x as u32, 0).iter().enumerate() {
                    assert_eq!(u32::from_ne_bytes(raw[ch * 4..ch * 4 + 4].try_into().unwrap()), *code);
                    let got = try_read_sample_f64(&raw, SampleType::U32, ch).unwrap();
                    assert_eq!(got.to_bits(), (f64::from(*code) / f64::from(u32::MAX)).to_bits());
                    if *code == 0x80000001 {
                        assert_ne!(got.to_bits(), f64::from(got as f32).to_bits());
                        adjacent = true;
                    }
                }
            }
            assert_eq!(ctx.render_band(Rect::new(x, 0, x + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected(x as u32, 0).map(f64::to_bits));
        }
        assert!(adjacent);
        assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks, anchors);
    }
    let (o, _, _) = adopt_u32(u32_fixture(Size::new(257, 259), 22));
    let ctx = Context::new_current(&o, NormalBandLimits { max_rows: 2, max_pixels: 14 }, &ctl()).unwrap();
    let band = ctx.render_band(Rect::new(250, 255, 257, 257), &ctl()).unwrap();
    for y in 255..257 {
        for x in 250..257 {
            assert_eq!(band.pixels()[((y - 255) * 7 + x - 250) as usize].map(f64::to_bits), expected(x as u32, y as u32).map(f64::to_bits));
        }
    }
}
struct Budget {
    q: BackingQuota,
    used: u64,
    limit: u64,
}
impl PreparationBudget for Budget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Unsupported("test Resident"))
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.used = self.used.checked_add(n).ok_or(CompositionError::Unsupported("test Work"))?;
        if self.used > self.limit { Err(CompositionError::Unsupported("test Work")) } else { Ok(()) }
    }
}
impl HierarchyPreparationBudget for Budget {
    fn reserve_hierarchy_index(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Index, n).map_err(|_| CompositionError::Unsupported("test Index"))
    }
}
#[test]
#[ignore = "UNRUN U32 explicit Source versus oldfloat/masked, shared trueIndex work, hidden all-leaf proof, bothpass Work/cancel and cold lastleaf Io same-q cleanup"]
fn u32_source_and_inactive_controls_keep_plain_authority() {
    let f = u32_fixture(Size::new(8, 6), 22);
    let q = f.q.clone();
    let base = q.usage().unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedNormalHierarchyPipeline::new_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
    assert!(LeasedNormalHierarchyPipeline::new_masked_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
    let p = LeasedNormalHierarchyPipeline::new_plain_quantized_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).unwrap();
    assert!(b.used > 0);
    drop(p);
    assert_eq!(q.usage().unwrap(), base);
    let mut low = Budget { q: q.clone(), used: 0, limit: 1 };
    assert!(LeasedNormalHierarchyPipeline::new_plain_quantized_source_budgeted(&f.doc, &Interrupt::NONE, &mut low, Arc::clone(&f.retention)).is_err());
    assert_eq!(q.usage().unwrap(), base);
    drop(f);
    let mut f = u32_fixture(Size::new(8, 6), 13);
    f.doc.layers[1].visible = false;
    reindex(&mut f);
    let (o, rasters, fails) = adopt_u32(f);
    let q = o.quota().clone();
    let ctx = Context::new_current(&o, Default::default(), &ctl()).unwrap();
    ctx.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 7, 1);
    drop(ctx.render_band(r, &ctl()).unwrap());
    let base = q.usage().unwrap();
    let total = AtomicU64::new(0);
    drop(
        ctx.render_band(
            r,
            &ReadControl {
                interrupt: &Interrupt::NONE,
                work: &|n| {
                    total.fetch_add(n, Ordering::Relaxed);
                    Ok(())
                },
            },
        )
        .unwrap(),
    );
    let used = AtomicU64::new(0);
    assert!(
        ctx.render_band(
            r,
            &ReadControl {
                interrupt: &Interrupt::NONE,
                work: &|n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total.load(Ordering::Relaxed) / 2 {
                    Err(BackingError::Work)
                } else {
                    Ok(())
                }
            }
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), base);
    let cancel = || true;
    let i = Interrupt::cancel_only(&cancel);
    assert!(ctx.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), base);
    rasters[2].clear_cache().unwrap();
    let base = q.usage().unwrap();
    fails[2].store(true, Ordering::Relaxed);
    assert!(ctx.validate_operative(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), base);
    fails[2].store(false, Ordering::Relaxed);
}
