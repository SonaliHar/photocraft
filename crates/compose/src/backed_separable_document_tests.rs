//! UNRUN generated CURRENT and explicit lower-band regressions. Native source
//! enrollment is covered by separately pinned IO/host drafts. All expected
//! clones, resident surfaces and reference buffers are external harness data.
use super::*;
use crate::{gimp_blend, gimp_composite::WorkingSpace};
use photocraft_doc::{
    BackedLayer, ImportCompositing, Layer, PixelFormat, Size, SourceColorContext,
    import_compositing::{GimpComposition, ResolvedGimpComposition},
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_raster::{
    Surface,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
const MODES: [i32; 21] = [23, 30, 31, 32, 33, 34, 35, 36, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53];
#[path = "backed_separable_general_current_tests.rs"]
mod general_current_tests;
#[path = "backed_separable_general_source_tests.rs"]
mod general_source_tests;
#[path = "backed_separable_legacy_tests.rs"]
mod legacy_tests;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    q: BackingQuota,
    reads: AtomicU64,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn leaf(q: &BackingQuota, mode: ColorMode, sample: SampleType, bounds: Rect) -> (BackedLayer, Arc<Store>) {
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let f = PixelFormat::new(mode, sample, true);
    let mut bytes = vec![0; bounds.width() as usize * bounds.height() as usize * f.bytes_per_pixel()];
    for (i, pixel) in bytes.chunks_exact_mut(f.bytes_per_pixel()).enumerate() {
        for channel in 0..mode.color_channels() {
            photocraft_color::try_write_sample_f64(pixel, sample, channel, 0.2 + (i % 4) as f64 * 0.1).unwrap();
        }
        photocraft_color::try_write_sample_f64(pixel, sample, mode.color_channels(), if i % 5 == 0 { 0. } else { 0.625 }).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(f, bounds, &vec![0; f.bytes_per_pixel()]).unwrap();
    let store = Arc::new(Store { descriptor, bytes, q: q.clone(), reads: AtomicU64::new(0), _fee: fee });
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
    (BackedLayer::new(raster, descriptor, &ctl()).unwrap(), store)
}
fn enc(s: GimpSpace) -> SampleEncoding {
    transfer(s).unwrap()
}
fn fixture(mode: i32, explicit: u8) -> (AccountedDocument, [Arc<Store>; 2]) {
    let q = quota();
    let model = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = Document::new("flat", Size::new(16, 12), ColorMode::Rgb, SampleType::F32);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(Arc::new(Builtin::Srgb.profile().to_bytes()));
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
    doc.source_color_context = Some(SourceColorContext { source_precision: 650, original_icc: None });
    let (base, bs) = leaf(&q, doc.mode, doc.depth, doc.bounds());
    let (upper, us) = leaf(&q, doc.mode, doc.depth, Rect::new(2, -1, 12, 8));
    let mut base = Layer::new("base", LayerContent::BackedRaster(base));
    let mut upper = Layer::new("upper", LayerContent::BackedRaster(upper));
    upper.opacity = 0.7125;
    let (version, rawblend, rawcomposite, blend, composite) = match explicit {
        1 => (12, 2, -1, GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
        2 => (23, 4, -1, GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
        3 => (12, -1, 2, GimpSpace::LinearRgb, GimpSpace::ProfileRgb),
        4 => (23, -1, 4, GimpSpace::LinearRgb, GimpSpace::PerceptualRgb),
        _ => match mode {
            30 | 33 | 34 | 41 => (12, -1, -1, GimpSpace::LinearRgb, GimpSpace::LinearRgb),
            35 | 36 => (12, 0, -1, GimpSpace::ProfileRgb, GimpSpace::LinearRgb),
            _ => (23, -4, -1, GimpSpace::PerceptualRgb, GimpSpace::LinearRgb),
        },
    };
    base.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    upper.import_compositing = Some(ImportCompositing { blend_encoding: enc(blend), composite_encoding: enc(composite), pass_through: false });
    // Canonical grain modes deliberately retain the physical Normal surrogate.
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = version;
    for (index, layer) in [&base, &upper].into_iter().enumerate() {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("fixture") };
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: layer.visible,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(index as u32 + 1),
            source_flags: 0,
            raster_bounds: Some(backed.operative_bounds()),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: format!(
                    "xcf-{version}-{}",
                    if index == 0 {
                        "normal"
                    } else if matches!(mode, 35 | 36) {
                        "authored-equivalent-rgb-space"
                    } else {
                        "authored-blend-auto"
                    }
                ),
                source_mode: if index == 0 { 28 } else { mode },
                legacy: false,
                stored_blend_space: if index == 0 { 0 } else { rawblend },
                stored_composite_space: if index == 0 { -1 } else { rawcomposite },
                stored_operator: if index == 0 { -1 } else { -2 },
                resolved: Some(ResolvedGimpComposition {
                    blend_space: if index == 0 { GimpSpace::LinearRgb } else { blend },
                    composite_space: if index == 0 { GimpSpace::LinearRgb } else { composite },
                    operator: if index == 0 { GimpOperator::Union } else { GimpOperator::ClipToBackdrop },
                }),
                pass_through: false,
            }),
        });
    }
    state.selection.active = Some(ItemRef::Layer(upper.id));
    state.selection.selected.push(ItemRef::Layer(upper.id));
    doc.layers = vec![base, upper];
    doc.xcf_state = Some(state);
    (AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new(model), q, &ctl()).unwrap(), [bs, us])
}
fn native_pixel(layer: &BackedLayer, x: i32, y: i32) -> [f64; 4] {
    if !layer.operative_bounds().contains(x, y) {
        return [0.; 4];
    }
    let mut bytes = [0; 40];
    let f = layer.format();
    let raw = &mut bytes[..f.bytes_per_pixel()];
    layer.checked_view().read_pixel_native_into(x, y, raw, &ctl()).unwrap();
    lower::decode(raw, f).unwrap()
}
#[test]
#[ignore = "UNRUN: all21 canonical modern modes and four explicit Multiply spaces match old resident renderer exactly, preserving native stores"]
fn current_matches_resident_without_replacing_canonical_grain_modes() {
    for (mode, explicit) in MODES.into_iter().map(|m| (m, 0)).chain((1..=4).map(|s| (30, s))) {
        let (owner, stores) = fixture(mode, explicit);
        let mut reference = owner.document().clone();
        for (layer, store) in reference.layers.iter_mut().zip(&stores) {
            layer.content = LayerContent::Raster(Surface::try_from_interleaved(store.descriptor.format(), store.descriptor.bounds(), &store.bytes).unwrap());
        }
        let expected = crate::gimp_render::PreparedRenderer::new(&reference).unwrap().render(reference.bounds(), &mut |_| Ok(())).unwrap();
        let before = stores.each_ref().map(|s| s.bytes.clone());
        let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let band = prepared.render_band(prepared.bounds(), &ctl()).unwrap();
        assert_eq!(band.pixels().len(), 192);
        for (a, b) in band.pixels().iter().zip(&expected.px) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
        assert_eq!(stores.each_ref().map(|s| s.bytes.clone()), before);
    }
}
#[test]
#[ignore = "UNRUN: generic lower bands preserve all five depths/RGB+Gray, explicit spaces/operator/outside coverage and zero opacity via existing kernel"]
fn lower_band_depth_space_operator_reference() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let q = quota();
            let (layer, _store) = leaf(&q, mode, sample, Rect::new(-2, 1, 1, 3));
            let profile = if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() };
            for code in MODES {
                for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
                    for opacity in [0., 0.625] {
                        let input = SeparableStackLayer {
                            layer: &layer,
                            visible: true,
                            opacity,
                            source_mode: code,
                            blend_space: GimpSpace::PerceptualRgb,
                            composite_space: GimpSpace::LinearRgb,
                            operator,
                        };
                        let prepared =
                            PreparedBackedSeparableStack::new(&[input], &q, profile, SampleEncoding::NonlinearProfile, Default::default(), &ctl()).unwrap();
                        prepared.validate_operative(&ctl()).unwrap();
                        let roi = Rect::new(-3, 0, 2, 4);
                        let bg = [0.25, 0.25, 0.25, 0.5];
                        let band = prepared.render_band(roi, bg, &ctl()).unwrap();
                        let blend = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::PerceptualRgb).unwrap();
                        let composite = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
                        for (y, row) in band.pixels().chunks(5).enumerate() {
                            for (x, p) in row.iter().enumerate() {
                                let source = native_pixel(&layer, roi.x0 + x as i32, roi.y0 + y as i32);
                                let expected = composite.separable(&blend, bg, source, opacity, operator, gimp_blend::mode(code).unwrap()).unwrap();
                                assert_eq!(p.map(f64::to_bits), expected.map(f64::to_bits));
                            }
                        }
                    }
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: coherent hidden/current activity still validates native tails and stale implicit marker; derived HDR is not clamped"]
fn hidden_malformed_and_profile_refusals() {
    for defect in 0..3 {
        let (owner, _) = fixture(41, 0);
        let q = owner.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = owner.document().clone();
        if defect == 0 {
            doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
        } else {
            let layer = &mut doc.layers[1];
            layer.visible = false;
            doc.xcf_state.as_mut().unwrap().items[1].visible = false;
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("fixture") };
            let x = backed.operative_bounds().x1 - 1;
            let y = backed.operative_bounds().y1 - 1;
            let values = [if defect == 1 { f32::NAN } else { 0. }, 0.25, 0.25, 0.625];
            let raw: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            layer.content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(x, y, x + 1, y + 1), &raw, &ctl()).unwrap());
        }
        let invalid = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap();
        assert!(PreparedBackedSeparableDocument::new(&invalid, Default::default(), &ctl()).is_err());
    }
    let (owner, _) = fixture(33, 0);
    let q = owner.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = owner.document().clone();
    let LayerContent::BackedRaster(backed) = &doc.layers[1].content else { panic!("fixture") };
    let raw: Vec<u8> = [1.5f32, -0.25, 0.5, 1.].into_iter().flat_map(f32::to_ne_bytes).collect();
    doc.layers[1].content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(3, 1, 4, 2), &raw, &ctl()).unwrap());
    let hdr = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap();
    assert!(PreparedBackedSeparableDocument::new(&hdr, Default::default(), &ctl()).is_ok());
}
#[test]
#[ignore = "UNRUN: warm-cache CURRENT working/header/band fees retained exactly and actual Work/Cancelled roll back same ledger"]
fn retained_fees_and_actual_control() {
    let (owner, _) = fixture(30, 4);
    drop(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap());
    let q = owner.quota();
    let before = q.usage().unwrap();
    {
        let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let held = q.usage().unwrap();
        assert!(held.resident > before.resident);
        assert!(held.index > before.index);
        let output = prepared.render_band(prepared.bounds(), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        drop(output);
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), before);
    let work = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n > 1024 { Err(BackingError::Work) } else { Ok(()) } };
    assert!(matches!(PreparedBackedSeparableDocument::new(&owner, Default::default(), &work), Err(E::Backing(BackingError::Work))));
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(PreparedBackedSeparableDocument::new(&owner, Default::default(), &c), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
#[ignore = "UNRUN: source stage prepares paid unique spaces over mapped resident metadata before native promotion, retains genuine source and never reads native or renders"]
fn source_stage_only_retains_preparation_and_real_source() {
    struct Budget {
        q: BackingQuota,
        work: u64,
    }
    impl crate::gimp_render::PreparationBudget for Budget {
        type Lease = QuotaLease;
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
            self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Invalid("fixture quota"))
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
            self.work = self.work.checked_add(n).ok_or(CompositionError::Invalid("fixture work"))?;
            Ok(())
        }
    }
    let (owner, stores) = fixture(30, 4);
    // This is mapper-shaped source metadata, not a new source eligibility claim.
    let mut mapped = owner.document().clone();
    for (layer, store) in mapped.layers.iter_mut().zip(&stores) {
        layer.content = LayerContent::Raster(Surface::try_from_interleaved(store.descriptor.format(), store.descriptor.bounds(), &store.bytes).unwrap());
    }
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut budget = Budget { q: q.clone(), work: 0 };
    let prepared = LeasedSeparableBlendPipeline::new_budgeted(&mapped, &Interrupt::NONE, &mut budget, owner.clone()).unwrap();
    assert!(budget.work > 0);
    assert!(q.usage().unwrap().resident > before.resident);
    assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
    drop(prepared);
    assert_eq!(q.usage().unwrap(), before);
    mapped.layers[0].visible = false;
    mapped.xcf_state.as_mut().unwrap().items[0].visible = false;
    assert!(LeasedSeparableBlendPipeline::new_budgeted(&mapped, &Interrupt::NONE, &mut budget, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
