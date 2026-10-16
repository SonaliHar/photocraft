//! UNRUN synthetic paid quantized Current/Source-preparation contracts.
//! No matching GIMP source/projection claim. Expected code arrays and Normal
//! reference buffers are external harness; native Stores retain real fees.
use super::*;
fn plain(sample: SampleType, size: Size, version: u32) -> Fixture {
    let mut f = fixture(sample, size, false);
    f.doc.source_color_context.as_mut().unwrap().source_precision = if sample == SampleType::U8 { 150 } else { 250 };
    let s = f.doc.xcf_state.as_mut().unwrap();
    for stack in &mut s.filter_stacks {
        stack.nodes.clear();
    }
    s.filter_masks.clear();
    s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    s.provenance.format_version = version;
    reindex(&mut f);
    f
}
fn adopt_plain(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    drop(f.template);
    let owner = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).unwrap();
    (owner, f.rasters, f.fails)
}
fn code_input(which: u32, x: u32, y: u32, sample: SampleType) -> [f64; 4] {
    // Closed native code expectations, independent of sample writer/decoder.
    let phase = (x + y) % 3;
    let a = (x + y) % 5;
    if sample == SampleType::U8 {
        [
            f64::from(32 + 16 * which + 8 * phase) / 255.,
            128. / 255.,
            f64::from(223 - 16 * which) / 255.,
            f64::from(match a {
                0 | 1 => 0,
                2 => 64,
                3 => 191,
                _ => 255,
            }) / 255.,
        ]
    } else {
        [
            f64::from(8192 + 4096 * which + 2048 * phase) / 65535.,
            32768. / 65535.,
            f64::from(57343 - 4096 * which) / 65535.,
            f64::from(match a {
                0 | 1 => 0,
                2 => 16384,
                3 => 49151,
                _ => 65535,
            }) / 65535.,
        ]
    }
}
fn expected(sample: SampleType, x: u32, y: u32) -> [f64; 4] {
    let w = WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
        .unwrap();
    let b = w.normal([0.; 4], code_input(0, x, y, sample), 0.5, GimpOperator::Union).unwrap();
    let middle = w.normal([0.; 4], code_input(1, x, y, sample), 0.75, GimpOperator::Union).unwrap();
    let inner = w.normal([0.; 4], code_input(2, x, y, sample), 1., GimpOperator::Union).unwrap();
    let outer = w.normal(middle, inner, 0.5, GimpOperator::Union).unwrap();
    w.normal(b, outer, 0.375, GimpOperator::Union).unwrap()
}
#[test]
#[ignore = "UNRUN U8/150 and U16/250 actual native codes, version12/13/22 fixed nested Normal, retained empty anchors and no Point programme/cast"]
fn actual_quantized_plain_trees_keep_codes_and_nested_order() {
    for sample in [SampleType::U8, SampleType::U16] {
        for version in [12, 13, 22] {
            let f = plain(sample, Size::new(8, 6), version);
            let anchors = f.doc.xcf_state.as_ref().unwrap().filter_stacks.clone(); // External expected containers.
            let (o, rasters, _) = adopt_plain(f);
            assert!(o.is_readonly_normal_hierarchy_storage());
            let ctx = Context::new_current(&o, Default::default(), &ctl()).unwrap();
            ctx.validate_operative(&ctl()).unwrap();
            assert!(ctx.plan.nodes.iter().filter_map(|n| n.leaf.as_ref()).all(|l| l.point.is_none()));
            assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks, anchors);
            for x in 0..5 {
                let band = ctx.render_band(Rect::new(x, 0, x + 1, 1), &ctl()).unwrap();
                assert_eq!(band.pixels()[0].map(f64::to_bits), expected(sample, x as u32, 0).map(f64::to_bits));
                for (which, raster) in rasters.iter().enumerate() {
                    let view = raster.checked_view();
                    let pin = view.pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
                    let mut raw = [0u8; 8];
                    let bpp = 4 * sample.bytes();
                    pin.read_pixel_native_into(x, 0, &mut raw[..bpp], &ctl()).unwrap();
                    for ch in 0..4 {
                        assert_eq!(
                            try_read_sample_f64(&raw[..bpp], sample, ch).unwrap().to_bits(),
                            code_input(which as u32, x as u32, 0, sample)[ch].to_bits()
                        );
                    }
                }
            }
            assert!(crate::backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument::new_byte_zero(&o, Default::default(), &ctl()).is_err());
            let facade = crate::backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument::new(&o, Default::default(), &ctl()).unwrap();
            assert_eq!(facade.sample_encoding(), SampleEncoding::NonlinearProfile);
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
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Unsupported("test resident"))
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.used = self.used.checked_add(n).ok_or(CompositionError::Unsupported("test work"))?;
        if self.used > self.limit { Err(CompositionError::Unsupported("test work")) } else { Ok(()) }
    }
}
impl HierarchyPreparationBudget for Budget {
    fn reserve_hierarchy_index(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Index, n).map_err(|_| CompositionError::Unsupported("test index"))
    }
}
#[test]
#[ignore = "UNRUN separate paid integer Source on actual Doc; old float/masked endpoints refuse; wrong precision, any disabled node, masks and malformed empty anchors do not infer plain authority"]
fn integer_source_role_is_closed_before_index_and_never_opens_filters() {
    for sample in [SampleType::U8, SampleType::U16] {
        let f = plain(sample, Size::new(8, 6), 22);
        let q = f.q.clone();
        let base = q.usage().unwrap();
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedNormalHierarchyPipeline::new_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
        assert!(LeasedNormalHierarchyPipeline::new_masked_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
        let source = LeasedNormalHierarchyPipeline::new_plain_quantized_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).unwrap();
        assert!(b.used > 0);
        drop(source);
        assert_eq!(q.usage().unwrap(), base);
        let mut limited = Budget { q: q.clone(), used: 0, limit: 1 };
        assert!(LeasedNormalHierarchyPipeline::new_plain_quantized_source_budgeted(&f.doc, &Interrupt::NONE, &mut limited, Arc::clone(&f.retention)).is_err());
        assert_eq!(q.usage().unwrap(), base);
    }
    for fault in 0..4 {
        let mut f = plain(SampleType::U16, Size::new(8, 6), 22);
        match fault {
            0 => f.doc.source_color_context.as_mut().unwrap().source_precision = 650,
            1 => {
                let mut n = fixture(SampleType::F32, Size::new(8, 6), false).doc.xcf_state.unwrap().filter_stacks[0].nodes[0].clone();
                n.enabled = false;
                f.doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.push(n);
            }
            2 => {
                let extra = fixture(SampleType::F32, Size::new(8, 6), false);
                f.doc.xcf_state.as_mut().unwrap().filter_masks.push(extra.doc.xcf_state.as_ref().unwrap().filter_masks[0].clone());
            }
            _ => f.doc.xcf_state.as_mut().unwrap().filter_stacks[0].owner = ItemRef::Layer(LayerId::fresh()),
        }
        let mut b = Budget { q: f.q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedNormalHierarchyPipeline::new_plain_quantized_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
        drop(f.template);
        assert!(AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN quantized runtime bytes at tile boundaries, bothaxis chunks, tinyQ1/depth32 and hidden native proof; original-q warmed Work/cancel and cold-Io refunds"]
fn integer_chunks_and_failed_publication_keep_real_native_custody() {
    let (o, rasters, fails) = adopt_plain(plain(SampleType::U16, Size::new(257, 259), 13));
    let q = o.quota().clone();
    let ctx = Context::new_current(&o, NormalBandLimits { max_rows: 2, max_pixels: 14 }, &ctl()).unwrap();
    let rect = Rect::new(250, 255, 257, 257);
    let band = ctx.render_band(rect, &ctl()).unwrap();
    for y in 255..257 {
        for x in 250..257 {
            assert_eq!(band.pixels()[((y - 255) * 7 + x - 250) as usize].map(f64::to_bits), expected(SampleType::U16, x as u32, y as u32).map(f64::to_bits));
        }
    }
    drop(band);
    ctx.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 2, 1);
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
    assert!(ctx.render_band(r, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), base);
    fails[2].store(false, Ordering::Relaxed);
    let mut hidden = plain(SampleType::U8, Size::new(8, 6), 12);
    hidden.doc.layers[1].visible = false;
    reindex(&mut hidden);
    let (hidden_owner, hidden_rasters, hidden_fails) = adopt_plain(hidden);
    let hidden_context = Context::new_current(&hidden_owner, Default::default(), &ctl()).unwrap();
    hidden_context.validate_operative(&ctl()).unwrap();
    hidden_rasters[2].clear_cache().unwrap();
    let hidden_base = hidden_owner.quota().usage().unwrap();
    hidden_fails[2].store(true, Ordering::Relaxed);
    assert!(hidden_context.validate_operative(&ctl()).is_err());
    assert_eq!(hidden_owner.quota().usage().unwrap(), hidden_base);
    hidden_fails[2].store(false, Ordering::Relaxed);
    let mut f = plain(SampleType::U8, Size::new(8, 6), 12);
    let mut leaf = f.doc.layers.remove(0);
    leaf.opacity = 1.;
    let mut tree = leaf;
    for n in 0..32 {
        tree = group(&format!("integer-depth-{n}"), vec![tree], 1.);
    }
    f.doc.layers = vec![tree];
    f.doc.xcf_state.as_mut().unwrap().filter_stacks.clear();
    reindex(&mut f);
    let (o, _, _) = adopt_plain(f);
    let ctx = Context::new_current(&o, NormalBandLimits { max_rows: 1, max_pixels: 1 }, &ctl()).unwrap();
    assert_eq!(ctx.plan.max_depth, 32);
    assert_eq!(ctx.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), code_input(0, 2, 0, SampleType::U8).map(f64::to_bits));
}
