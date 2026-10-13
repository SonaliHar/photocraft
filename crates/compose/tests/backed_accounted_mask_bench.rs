//! Scoped generated ordinary masked CPU timing, not real-file import/save qualification.
use photocraft_compose::{backed_document::PreparedBackedDocument, backed_normal::NormalBandLimits};
use photocraft_doc::{accounted_document::AccountedDocument, import_compositing::*, item_identity::*, xcf_state::*, *};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{quota::*, *},
};
use std::sync::Arc;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Constant {
    descriptor: NativePlaneDescriptor,
}
impl NativePlaneStore for Constant {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(out.len() as u64)?;
        let size = self.descriptor.format().bytes_per_pixel() as u64;
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        };
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.descriptor.default_bytes()[((offset + i as u64) % size) as usize];
        }
        Ok(())
    }
}
fn encode(v: f64) -> f64 {
    if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1. / 2.4) - 0.055 }
}
fn generated_case(size: Size, width: u32, height: u32) {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 4 << 20, scratch: 0 });
    // Empty mask map/default box and the fixed model/provider owners are prepaid.
    let source = q.reserve(QuotaKind::Resident, 16 << 10).unwrap();
    let mut doc = Document::new("generated 24MP ordinary mask", size, ColorMode::Rgb, SampleType::F64);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let profile = photocraft_cms::Builtin::Srgb.profile();
    doc.icc_profile = Some(profile.to_bytes());
    let rect = doc.bounds();
    let mut state = XcfState::default();
    state.provenance.format_version = 12;
    for (index, pixel) in [[1., 0., 0., 1.], [0., 1., 0., 0.5]].into_iter().enumerate() {
        let bytes: Vec<u8> = pixel.into_iter().flat_map(f64::to_ne_bytes).collect();
        let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), rect, &bytes).unwrap();
        let backed = BackedRaster::new(Arc::new(Constant { descriptor }), q.clone(), 1, &ctl()).unwrap();
        let mut layer = Layer::new("generated source", LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &ctl()).unwrap()));
        if index == 1 {
            layer.opacity = 0.625;
            let surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &0.5f64.to_ne_bytes()).unwrap();
            layer.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
            state.items.push(ItemState {
                item: ItemRef::Mask(MaskId(1)),
                name: "generated mask".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: None,
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(layer.id),
                    bounds: rect,
                    offset: [0, 0],
                    outside_f64_bits: 0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: true,
                }),
            });
        }
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(rect),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: if index == 0 { -1 } else { 1 },
                stored_operator: if index == 0 { -1 } else { 1 },
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
                pass_through: false,
            }),
        });
        doc.layers.push(layer);
    }
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
    let owner = AccountedDocument::from_owned(doc, Arc::new(source), q.clone(), &ctl()).unwrap();
    let original_mask = owner.document().layers[1].mask.as_ref().unwrap().surface.clone();
    let prepared = PreparedBackedDocument::new_accounted(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let canceled = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(prepared.render_reduced(width, height, &canceled).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let start = std::time::Instant::now();
    let output = prepared.render_reduced(width, height, &ctl()).unwrap();
    let elapsed = start.elapsed();
    let expected = [encode(0.84375), encode(0.15625), 0., 1.];
    assert_eq!(output.pixels().len(), (width * height) as usize);
    for actual in output.pixels() {
        for (a, b) in actual.iter().zip(expected) {
            assert!((f64::from(*a) - b).abs() < 2e-7, "{actual:?} vs {expected:?}");
        }
    }
    println!(
        "generated ordinary resident-mask {}x{} -> {}x{} compute_seconds={:.9} retained_logical={:?}",
        size.width,
        size.height,
        width,
        height,
        elapsed.as_secs_f64(),
        q.usage().unwrap()
    );
    assert!(Arc::ptr_eq(&original_mask, &owner.document().layers[1].mask.as_ref().unwrap().surface));
    assert_eq!(original_mask.default_bytes(), 0.5f64.to_ne_bytes());
    drop(output);
    drop(prepared);
    drop(original_mask);
    drop(owner);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn generated_ordinary_resident_mask_matches_independent_gamma_equation_and_refunds() {
    generated_case(Size::new(2, 1), 2, 1);
}
#[test]
#[ignore = "generated accounted ordinary 24MP resident-mask reduction release benchmark; no decode/save or corpus qualification"]
fn generated_24mp_accounted_ordinary_resident_mask_reduction_checks_every_output_and_refunds() {
    generated_case(Size::new(6000, 4000), 1200, 800);
}
