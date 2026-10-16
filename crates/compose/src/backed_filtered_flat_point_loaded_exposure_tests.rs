//! Ignored UNRUN paid synthetic Point+mixed V2 contracts; no producer oracle.
use super::*;
use crate::backed_stock_point_context::{PreparedBackedStockPointContext, PreparedStockPointPipeline};
fn set_loaded(n: &mut photocraft_doc::xcf_filters::FilterNode, point: bool, black: u32, stops: f32) {
    let epoch = if point { crate::EXPOSURE_LOADED_POINT_EPOCH } else { crate::EXPOSURE_LOADED_MIXED_EPOCH };
    n.operation = OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: epoch.into() };
    n.arguments = vec![FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(black)) }, number("exposure", stops)];
    n.composition.as_mut().unwrap().compatibility_epoch = epoch.into();
}
fn single(sample: SampleType, black: u32, enabled: bool) -> Fixture {
    let mut f = ordered(sample, &[&[0]], enabled);
    set_loaded(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], true, black, 0.5);
    f
}
fn point_owner(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    let o = AccountedDocument::new_readonly_point_storage(f.document, f.retention, f.quota, &ctl()).unwrap();
    (o, f.rasters, f.fails)
}
fn native(p: [f64; 4], sample: SampleType) -> Vec<u8> {
    // External expected/raw request buffer; no production storage grant.
    let mut b = vec![0; 4 * sample.bytes()];
    for (c, v) in p.into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut b, sample, c, v).unwrap();
    }
    b
}
fn expected(p: [f64; 4], stops: f64, black: f64) -> [f64; 4] {
    if stops == 0. && black == 0. {
        return p;
    }
    let curve = photocraft_cms::srgb_trc();
    let mut o = p;
    for i in 0..3 {
        let v = curve.eval64(p[i]);
        let linear = if black == 0. { v * stops.exp2() } else { (v - black) / ((-stops).exp2() - black) };
        o[i] = curve.eval_inverse64(linear);
    }
    o
}
fn resident_single(sample: SampleType, black: u32) -> Fixture {
    let mut f = single(sample, black, true);
    let format = f.document.pixel_format();
    let bounds = f.document.bounds();
    // Existing prospective8MiB Resident+Index guards precede this real one-tile
    // Surface and its tiny raw staging buffer, including possible later COW tile.
    let mut bytes = Vec::new();
    let len = usize::try_from(f.document.size.area()).unwrap() * format.bytes_per_pixel();
    bytes.try_reserve_exact(len).unwrap();
    assert!(bytes.capacity() <= 2 * len);
    for y in 0..6 {
        for x in 0..8 {
            bytes.extend_from_slice(&native(input(0, x, y, sample), sample));
        }
    }
    let surface = Surface::try_from_interleaved_with(format, bounds, &bytes, &Interrupt::NONE).unwrap();
    assert!(surface.resident_accounted_bytes().unwrap() < 4 << 20);
    f.document.layers[0].content = LayerContent::Raster(surface);
    f
}
#[test]
#[ignore = "UNRUN ordinary actualPoint1root1loadedExposure and Source resident fullproof, bothdepth/endpoints/inward, strictoldV1 preserved/rawargs unchanged, no mixedtopology workaround"]
fn ordinary_singleton_has_a_real_paid_route() {
    for sample in [SampleType::F32, SampleType::F64] {
        for black in [0x3dcc_cccd, 0xbdcc_cccd, 0x3dcc_cccc, 0xbdcc_cccc, 0] {
            let f = single(sample, black, true);
            let raw = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone(); // External expectation.
            let (o, _, _) = point_owner(f);
            let basis = photocraft_cms::Builtin::Srgb.profile();
            let c = PreparedBackedStockPointContext::new(&o, basis, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            let pipeline = PreparedStockPointPipeline::prepare(o.document(), basis, o.quota(), &ctl()).unwrap();
            let effective = crate::LoadedExposureParameters::from_words(0.5f32.to_bits(), black).unwrap().effective().values()[1];
            for x in [0u32, 2, 4, 7] {
                let p = input(0, x, 0, sample);
                let bytes = native(p, sample);
                assert_eq!(
                    pipeline.evaluate_native(&bytes, o.document().pixel_format(), &ctl()).unwrap().map(f64::to_bits),
                    expected(p, 0.5, effective).map(f64::to_bits)
                );
            }
            assert!(o.is_readonly_point_storage());
            assert!(!o.is_readonly_filtered_flat_point_storage());
            assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, raw);
            let f = resident_single(sample, black);
            let baseline = f.quota.usage().unwrap();
            let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
            if matches!(black, 0x3dcc_cccd | 0xbdcc_cccd) {
                let p = PreparedStockPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap();
                assert!(f.quota.usage().unwrap().resident > baseline.resident);
                drop(p);
            } else {
                // V2 CURRENT retains valid edited words; original Source V2
                // is selected only by the two exact outward endpoint words.
                assert!(PreparedStockPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
            }
            assert_eq!(f.quota.usage().unwrap(), baseline);
        }
    }
    // Actual paid130x65 ordinary Point: no fake mixed root or topology change.
    for sample in [SampleType::F32, SampleType::F64] {
        let mut f = fixture(sample, Size::new(130, 65), &[Some(0), None], None);
        f.document.layers.truncate(1);
        let id = f.document.layers[0].id;
        let state = f.document.xcf_state.as_mut().unwrap();
        let mask = state.filter_masks[0].id;
        state.items.retain(|r| r.item == ItemRef::Layer(id) || r.item == ItemRef::Mask(mask));
        state.filter_stacks.truncate(1);
        state.filter_masks.truncate(1);
        set_loaded(&mut state.filter_stacks[0].nodes[0], true, 0x3dcc_cccd, 0.5);
        let (o, _, _) = point_owner(f);
        let c = PreparedBackedStockPointContext::new(&o, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).unwrap();
        c.validate_operative(&ctl()).unwrap();
        assert_eq!(c.descriptor().bounds(), Rect::new(0, 0, 130, 65));
    }
    assert!(crate::ExposureParameters::from_words(0.5f32.to_bits(), 0x3dcc_cccd).is_err());
    let mut f = single(SampleType::F64, 0x3dcc_cccd, true);
    let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    n.operation.schema_epoch = crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into();
    n.composition.as_mut().unwrap().compatibility_epoch = crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
    let (o, _, _) = point_owner(f);
    assert!(PreparedBackedStockPointContext::new(&o, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN actualmixed V2 namedordered reference/solepaidstage; loadedblack sourcewords preserved, oldmixedV1 strict, disable/neutral and sharedbridge retain their own policies"]
fn mixed_uses_effective_parameters_without_rewriting_the_stack() {
    let (nl, linear) = parts();
    for sample in [SampleType::F32, SampleType::F64] {
        for black in [0x3dcc_cccd, 0xbdcc_cccd] {
            let mut f = ordered(sample, &[&[0, 2, 0], &[]], true);
            set_loaded(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], false, black, 0.5);
            let raw = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.clone(); // Harness expectation only.
            let (o, _, _, _) = adopt(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            let p = input(0, 2, 0, sample);
            let reference = expected(bridge(step(2, bridge(step(0, p, false, &nl, &linear)), true, &nl, &linear)), 0.5, 0.);
            assert_eq!(root_value(&c, 0, 2, 0).map(f64::to_bits), reference.map(f64::to_bits));
            assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes, raw);
            let q = o.quota().clone();
            let before = q.usage().unwrap();
            let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
            let paid = LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).unwrap();
            drop(paid);
            assert_eq!(q.usage().unwrap(), before);
            for enabled in [true, false] {
                let mut f = ordered(sample, &[&[0], &[]], enabled);
                set_loaded(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], false, black, 0.);
                let (o, _, _, _) = adopt(f);
                let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                c.validate_operative(&ctl()).unwrap();
                let p = input(0, 4, 0, sample);
                assert_eq!(root_value(&c, 0, 4, 0).map(f64::to_bits), p.map(f64::to_bits));
            }
        }
    }
    for black in [0, 0x3dcc_cccc, 0xbdcc_cccc] {
        let mut f = ordered(SampleType::F64, &[&[0], &[]], true);
        set_loaded(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], false, black, 0.5);
        let o = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).unwrap();
        let c = Context::new(&o, Default::default(), &ctl()).unwrap();
        c.validate_operative(&ctl()).unwrap();
        let p = input(0, 2, 0, SampleType::F64);
        let effective = crate::LoadedExposureParameters::from_words(0.5f32.to_bits(), black).unwrap().effective().values()[1];
        assert_eq!(root_value(&c, 0, 2, 0).map(f64::to_bits), expected(p, 0.5, effective).map(f64::to_bits));
        let q = o.quota().clone();
        let before = q.usage().unwrap();
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let mut f = ordered(SampleType::F64, &[&[0], &[]], true);
    set_loaded(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], false, 0x3dcc_cccd, 0.5);
    let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    n.operation.schema_epoch = crate::EXPOSURE_MIXED_EPOCH.into();
    n.composition.as_mut().unwrap().compatibility_epoch = crate::EXPOSURE_MIXED_EPOCH.into();
    let (o, _, _, _) = adopt(f);
    assert!(Context::new(&o, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN Point SourceBYTEzero padding/mask/sourceactivity and fullinactiveprofile/native proof; actualconstructor/runtime Workcancel refund and nativeprovider coldIo without publication"]
fn full_proof_and_same_control_survive_loaded_endpoints() {
    for fault in 0..6 {
        let mut f = resident_single(SampleType::F64, 0x3dcc_cccd);
        match fault {
            0 => {
                let LayerContent::Raster(s) = &mut f.document.layers[0].content else { panic!("resident") };
                s.try_write_interleaved(Rect::new(200, 200, 201, 201), &native([0.25, 0., 0., 0.], SampleType::F64)).unwrap();
            }
            1 => {
                f.document.xcf_state.as_mut().unwrap().filter_masks[0]
                    .surface
                    .try_write_interleaved(Rect::new(200, 200, 201, 201), &(-0f64).to_ne_bytes())
                    .unwrap();
            }
            2 => {
                f.document.layers[0].visible = false;
                let id = f.document.layers[0].id;
                f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
            }
            3 => f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false,
            4 => f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            _ => {
                let LayerContent::Raster(s) = &mut f.document.layers[0].content else { panic!("resident") };
                s.try_write_interleaved(Rect::new(7, 5, 8, 6), &native([f64::NAN, 0., 0., 1.], SampleType::F64)).unwrap();
            }
        }
        let before = f.quota.usage().unwrap();
        let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
        assert!(PreparedStockPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
        assert_eq!(f.quota.usage().unwrap(), before);
    }
    // A count/payload-only negative proves the inherited16MiB admission runs
    // before any huge physical scan. Shared zero payload is genuinely prepaid;
    // outgrid coordinates are deliberately downstream-invalid, never published.
    let mut f = single(SampleType::F64, 0x3dcc_cccd, true);
    let mask = &mut f.document.xcf_state.as_mut().unwrap().filter_masks[0];
    let tile = Arc::new(mask.surface.try_default_tile().unwrap());
    mask.surface.put_tiles((0..32).map(|tx| (photocraft_geom::TileCoord { tx, ty: 0 }, tile.clone())));
    assert!(mask.surface.resident_payload_bytes().unwrap() > 16 << 20);
    let before = f.quota.usage().unwrap();
    let mut b = Budget { q: f.quota.clone(), used: 0, limit: 1 << 20 };
    assert!(matches!(
        PreparedStockPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()),
        Err(CompositionError::Unsupported(_))
    ));
    assert!(b.used < 1 << 20);
    assert_eq!(f.quota.usage().unwrap(), before);
    assert_eq!(f.reads.load(Ordering::Relaxed), 0);
    let mut f = single(SampleType::F64, 0xbdcc_cccd, false);
    f.document.layers[0].visible = false;
    f.document.layers[0].opacity = 0.;
    let id = f.document.layers[0].id;
    let state = f.document.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
    state.filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &(-0f64).to_ne_bytes()).unwrap();
    let (o, _, _) = point_owner(f);
    assert!(PreparedBackedStockPointContext::new(&o, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).is_err());
    for fault in 0..3 {
        let mut f = single(SampleType::F64, 0x3dcc_cccd, false);
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        match fault {
            0 => {
                let LayerContent::BackedRaster(r) = &f.document.layers[0].content else { panic!("backed") };
                let edited = r.try_edited_native(Rect::new(7, 5, 8, 6), &native([f64::NAN, 0., 0., 1.], SampleType::F64), &ctl()).unwrap();
                f.document.layers[0].content = LayerContent::BackedRaster(edited);
            }
            1 => f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            _ => {
                let mask = &mut f.document.xcf_state.as_mut().unwrap().filter_masks[0];
                mask.surface = Surface::try_with_default_bytes(mask.surface.format(), &(-0f64).to_ne_bytes()).unwrap();
            }
        }
        let (o, _, _) = point_owner(f);
        if let Ok(c) = PreparedBackedStockPointContext::new(&o, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()) {
            assert!(c.validate_operative(&ctl()).is_err());
        }
    }
    let (o, rasters, fails) = point_owner(single(SampleType::F64, 0xbdcc_cccd, true));
    let q = o.quota().clone();
    let c = PreparedBackedStockPointContext::new(&o, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    c.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let total = used.load(Ordering::Relaxed);
    let used = AtomicU64::new(0);
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) >= total { Err(BackingError::Work) } else { Ok(()) };
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let cancel = || true;
    let i = Interrupt::cancel_only(&cancel);
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    rasters[0].clear_cache().unwrap();
    fails[0].store(true, Ordering::Relaxed);
    let before = q.usage().unwrap();
    assert!(c.render_wide_band(Rect::new(0, 0, 1, 1), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    fails[0].store(false, Ordering::Relaxed);
    drop(c);
    let before = q.usage().unwrap();
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(PreparedStockPointPipeline::prepare(o.document(), photocraft_cms::Builtin::Srgb.profile(), &q, &deny).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
