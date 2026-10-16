//! UNRUN lower-layer actual retained native COW/Current contracts. No Source,
//! engine publication, producer projection or arbitrary native padding claim.
//! Expected arrays/WorkingSpace are external harness; input patches have real fees.
use super::*;
use photocraft_doc::accounted_document::AccountedEdit;
struct Patch {
    bytes: Vec<u8>,
    _fee: QuotaLease,
}
fn patch(format: PixelFormat, rect: Rect, values: [f64; 4], q: &BackingQuota) -> Patch {
    let len = rect.width() as usize * rect.height() as usize * format.bytes_per_pixel();
    let fee = q.reserve(QuotaKind::Resident, (len as u64) * 2 + 128).unwrap();
    let mut bytes = vec![0; len];
    for pixel in bytes.chunks_exact_mut(format.bytes_per_pixel()) {
        for (ch, value) in values.iter().enumerate() {
            photocraft_color::try_write_sample_f64(pixel, format.sample, ch, *value).unwrap();
        }
    }
    Patch { bytes, _fee: fee }
}
fn plain(sample: SampleType) -> Fixture {
    let mut f = fixture(sample, Size::new(8, 6), false);
    f.doc.source_color_context.as_mut().unwrap().source_precision = match sample {
        SampleType::U8 => 150,
        SampleType::U16 => 250,
        SampleType::U32 => 350,
        SampleType::F32 => 650,
        SampleType::F64 => 750,
    };
    let s = f.doc.xcf_state.as_mut().unwrap();
    for stack in &mut s.filter_stacks {
        stack.nodes.clear();
    }
    s.filter_masks.clear();
    s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    reindex(&mut f);
    f
}
fn adopt_plain(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    drop(f.template);
    let o = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).unwrap();
    (o, f.rasters, f.fails)
}
fn leaf<'a>(o: &'a AccountedDocument, n: usize) -> (LayerId, &'a BackedLayer) {
    let index = o.checked_readonly_normal_hierarchy_current_index(&ctl()).unwrap();
    let layer = index.rows().iter().filter(|r| matches!(r.layer.content, LayerContent::BackedRaster(_))).nth(n).unwrap().layer;
    let LayerContent::BackedRaster(native) = &layer.content else { unreachable!() };
    (layer.id, native)
}
fn decoded(sample: SampleType, p: [f64; 4]) -> [f64; 4] {
    match sample {
        SampleType::F64 => p,
        SampleType::F32 => p.map(|v| f64::from(v as f32)),
        SampleType::U8 => p.map(|v| f64::from((v * 255.).round() as u8) / 255.),
        SampleType::U16 => p.map(|v| f64::from((v * 65535.).round() as u16) / 65535.),
        SampleType::U32 => p.map(|v| f64::from((v * f64::from(u32::MAX)).round() as u32) / f64::from(u32::MAX)),
    }
}
fn expected(sample: SampleType, x: u32, changed: Option<[f64; 4]>) -> [f64; 4] {
    let w = WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
        .unwrap();
    let a = decoded(sample, input(0, x, 0, sample));
    let b = decoded(sample, input(1, x, 0, sample));
    let c = changed.unwrap_or_else(|| decoded(sample, input(2, x, 0, sample)));
    let bottom = w.normal([0.; 4], a, 0.5, GimpOperator::Union).unwrap();
    let middle = w.normal([0.; 4], b, 0.75, GimpOperator::Union).unwrap();
    let inner = w.normal([0.; 4], c, 1., GimpOperator::Union).unwrap();
    let outer = w.normal(middle, inner, 0.5, GimpOperator::Union).unwrap();
    w.normal(bottom, outer, 0.375, GimpOperator::Union).unwrap()
}
#[test]
#[ignore = "UNRUN all5depth real nonfirst native COW pre/post fixed nestedNormal, exact siblings/source/default/anchors; named sharednoop still requires fullOLD/STAGED and same-q render controls"]
fn native_cow_render_and_noop_keep_full_current_authority() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let (o, _, _) = adopt_plain(plain(sample));
        let q = o.quota().clone();
        let old = Context::new_current(&o, Default::default(), &ctl()).unwrap();
        old.validate_operative(&ctl()).unwrap();
        let (id, native) = leaf(&o, 2);
        let descriptor = native.descriptor();
        let rect = Rect::new(1, 0, 3, 1);
        let values = [0.8 + 2f64.powi(-42), 0.2 + 2f64.powi(-43), 0.4 + 2f64.powi(-41), 0.625];
        let p = patch(o.document().pixel_format(), rect, values, &q);
        assert!(o.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &p.bytes }, &ctl()).is_err());
        o.validate_readonly_normal_hierarchy_native_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &p.bytes }, &ctl()).unwrap();
        let staged = o.try_edit_readonly_normal_hierarchy_native(AccountedEdit::NativePixels { layer: id, rect, bytes: &p.bytes }, &ctl()).unwrap();
        let current = Context::new_current(&staged, Default::default(), &ctl()).unwrap();
        current.validate_operative(&ctl()).unwrap();
        assert_eq!(staged.document().xcf_state, o.document().xcf_state);
        assert_eq!(staged.document().source_color_context, o.document().source_color_context);
        assert!(Arc::ptr_eq(staged.document().icc_profile.as_ref().unwrap(), o.document().icc_profile.as_ref().unwrap()));
        assert_eq!(leaf(&staged, 2).1.descriptor(), descriptor);
        for n in [0, 1] {
            assert_eq!(leaf(&staged, n).1, leaf(&o, n).1);
        }
        for x in 0..4 {
            let r = Rect::new(x, 0, x + 1, 1);
            assert_eq!(old.render_band(r, &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected(sample, x as u32, None).map(f64::to_bits));
            assert_eq!(
                current.render_band(r, &ctl()).unwrap().pixels()[0].map(f64::to_bits),
                expected(sample, x as u32, if (1..3).contains(&x) { Some(decoded(sample, values)) } else { None }).map(f64::to_bits)
            );
        }
        let noop = staged.try_edit_readonly_normal_hierarchy_native(AccountedEdit::NativePixels { layer: id, rect, bytes: &p.bytes }, &ctl()).unwrap();
        assert!(noop.shares_model(&staged));
        let noop_context = Context::new_current(&noop, Default::default(), &ctl()).unwrap();
        noop_context.validate_operative(&ctl()).unwrap();
        let r = Rect::new(0, 0, 4, 1);
        drop(current.render_band(r, &ctl()).unwrap());
        let base = q.usage().unwrap();
        let total = AtomicU64::new(0);
        drop(
            current
                .render_band(
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
            current
                .render_band(
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
        let stop = || true;
        let i = Interrupt::cancel_only(&stop);
        assert!(current.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), base);
    }
}
#[test]
#[ignore = "UNRUN actual borrowed F64 L2 finiteHDR patch can stage structurally yet hidden STAGED unitCurrent refuses; sharednoop cannot hide OLD/STAGED provider failure, no publication/source claim"]
fn finite_hdr_and_hidden_noop_fail_complete_current_validation() {
    let mut f = plain(SampleType::F64);
    f.doc.layers[1].visible = false;
    reindex(&mut f);
    let (o, rasters, fails) = adopt_plain(f);
    let q = o.quota().clone();
    let old = Context::new_current(&o, Default::default(), &ctl()).unwrap();
    old.validate_operative(&ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let (id, native) = leaf(&o, 2);
    let brush = photocraft_paint::BrushSettings { size: 4., hardness: 1., aliased: true, pressure_size: false, opacity: 1., flow: 1., ..Default::default() };
    let points = [photocraft_paint::StrokePoint::new(2.5, 2.5, 1.)];
    let color = photocraft_paint::native_stroke_controlled::NativeWideColor::new(native.format(), &[4., 2., 0.25, 1.]).unwrap();
    let plan = photocraft_paint::native_stroke_controlled::prepare_clipped_native_wide_stroke(
        native.checked_view(),
        native.operative_bounds(),
        &brush,
        &points,
        color,
        false,
        1.,
        photocraft_paint::native_stroke_controlled::NativeStrokeLimits::STANDARD,
        &q,
        &ctl(),
    )
    .unwrap();
    let p = plan.render(&ctl()).unwrap();
    assert!(p.changed());
    assert!(p.bytes().chunks_exact(32).any(|raw| (0..3).any(|ch| photocraft_color::try_read_sample_f64(raw, SampleType::F64, ch).unwrap() > 1.)));
    let staged = o.try_edit_readonly_normal_hierarchy_native(AccountedEdit::NativePixels { layer: id, rect: p.bounds(), bytes: p.bytes() }, &ctl()).unwrap();
    let current = Context::new_current(&staged, Default::default(), &ctl()).unwrap();
    assert!(current.validate_operative(&ctl()).is_err());
    assert_eq!(staged.document().xcf_state, o.document().xcf_state);
    drop(current);
    drop(staged);
    drop(p);
    old.validate_operative(&ctl()).unwrap();
    assert_eq!(q.usage().unwrap(), baseline);
    let (id0, _) = leaf(&o, 0);
    let rect = Rect::new(1, 0, 2, 1);
    let same = patch(o.document().pixel_format(), rect, input(0, 1, 0, SampleType::F64), &q);
    let noop = o.try_edit_readonly_normal_hierarchy_native(AccountedEdit::NativePixels { layer: id0, rect, bytes: &same.bytes }, &ctl()).unwrap();
    assert!(noop.shares_model(&o));
    let noop_context = Context::new_current(&noop, Default::default(), &ctl()).unwrap();
    noop_context.validate_operative(&ctl()).unwrap();
    rasters[2].clear_cache().unwrap();
    let cold = q.usage().unwrap();
    fails[2].store(true, Ordering::Relaxed);
    assert!(old.validate_operative(&ctl()).is_err());
    assert!(noop_context.validate_operative(&ctl()).is_err());
    assert_eq!(q.usage().unwrap(), cold);
    fails[2].store(false, Ordering::Relaxed);
}
