//! UNRUN current-recipe/storage-owner tests. These fixtures supply no original
//! source correspondence, evidence enrollment or full backing-padding proof.
use super::*;
use photocraft_doc::{
    BackedLayer, Document, FilterMask, ImportCompositing, Layer, PixelFormat, SampleType, Size, SourceColorContext, XcfState,
    import_compositing::{GimpComposition, ResolvedGimpComposition},
    item_identity::MaskId,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackedRaster, BackingError, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits},
    },
};
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
pub(crate) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    reads: Arc<AtomicU64>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let at = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
pub(crate) type Fixture = (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>);
pub(crate) fn fixture(mode: ColorMode, sample: SampleType, stored: SampleEncoding) -> Fixture {
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 0, index: 16 << 20 });
    // Actual source/map/default/model fixture allocation is prospectively paid.
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(quota.reserve(QuotaKind::Resident, 2 << 20).unwrap());
    let profile = if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() };
    let mut doc = Document::new("current recipe", Size::new(8, 6), mode, sample);
    let effective = photocraft_cms::xcf_color::effective_profile(None, profile.color_space, crate::gimp_composite::encoding(stored)).unwrap();
    doc.icc_profile = Some(effective.to_bytes());
    doc.working_profile_interpretation = if mode == ColorMode::Rgb {
        WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
    } else {
        WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
    };
    doc.sample_encoding = Some(stored);
    doc.source_color_context = Some(SourceColorContext {
        original_icc: None,
        source_precision: match (sample, stored) {
            (SampleType::U16, SampleEncoding::Linear) => 200,
            (SampleType::U16, _) => 250,
            (SampleType::F32, SampleEncoding::Linear) => 600,
            (SampleType::F32, _) => 650,
            (SampleType::F64, SampleEncoding::Linear) => 700,
            _ => 750,
        },
    });
    let format = PixelFormat::new(mode, sample, true);
    let mut raw = vec![0; 48 * format.bytes_per_pixel()];
    for (i, pixel) in raw.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
        for channel in 0..mode.color_channels() {
            let color = if i % 3 == 0 { -0.0 } else { 0.5 + 2f64.powi(-45) };
            photocraft_color::try_write_sample_f64(pixel, sample, channel, color).unwrap();
        }
        photocraft_color::try_write_sample_f64(
            pixel,
            sample,
            mode.color_channels(),
            if i % 4 == 0 {
                -0.0
            } else if i % 2 == 0 {
                0.25
            } else {
                0.75
            },
        )
        .unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, doc.bounds(), &vec![0; format.bytes_per_pixel()]).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let fee = quota.reserve(QuotaKind::Resident, raw.capacity() as u64 + 256).unwrap();
    let store = Arc::new(Store { descriptor, bytes: raw.clone(), reads: reads.clone(), quota: quota.clone(), _fee: fee });
    let raster = BackedRaster::new(store, quota.clone(), 1, &ctl()).unwrap();
    let mut layer = Layer::new("native", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    layer.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    let node = FilterId(100);
    let mask = MaskId(101);
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(layer.id),
        revision: 0,
        nodes: vec![FilterNode {
            id: node,
            operation: OperationIdentity {
                name: "gimp:threshold".into(),
                stored_version: "0:0".into(),
                schema_epoch: crate::gimp_filters::THRESHOLD_PRECISION_EPOCH.into(),
            },
            arguments: vec![
                FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 0 } },
                FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
                FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())) },
            ],
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: Some(GimpComposition {
                compatibility_epoch: crate::gimp_filters::THRESHOLD_PRECISION_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1565818287,
                stored_composite_space: -1565818287,
                stored_operator: -1565818287,
                resolved: None,
                pass_through: false,
            }),
        }],
    });
    state.items.push(ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: "xcf-12-normal".into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 0,
            stored_composite_space: -1,
            stored_operator: -1,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
            pass_through: false,
        }),
    });
    state.items.push(ItemState {
        item: ItemRef::Mask(mask),
        name: "selection".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(doc.bounds()),
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(layer.id),
            bounds: doc.bounds(),
            offset: [0, 0],
            outside_f64_bits: 0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
    });
    state.filter_masks.push(FilterMask {
        id: mask,
        owner: node,
        bounds: doc.bounds(),
        surface: Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false)),
    });
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    (doc, quota, guard, reads, profile, raw)
}
pub(crate) fn adopt(doc: Document, guard: Arc<dyn Any + Send + Sync>, q: BackingQuota) -> AccountedDocument {
    AccountedDocument::new_readonly_threshold_storage(doc, guard, q, &ctl()).unwrap()
}

#[test]
fn current_native_threshold_all_depths_raw_alpha_and_disabled_identity() {
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        for enabled in [true, false] {
            let (mut doc, q, guard, reads, profile, raw) = fixture(ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
            let owner = adopt(doc, guard, q);
            let context = PreparedBackedThresholdContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
            assert_eq!(reads.load(Ordering::Relaxed), 0);
            let band = context.render_band(owner.document().bounds(), &ctl()).unwrap();
            let bpp = band.format().bytes_per_pixel();
            for (i, (got, original)) in band.bytes().chunks_exact(bpp).zip(raw.chunks_exact(bpp)).enumerate() {
                assert_eq!(&got[3 * sample.bytes()..], &original[3 * sample.bytes()..]);
                if enabled {
                    for channel in 0..3 {
                        assert_eq!(photocraft_color::try_read_sample_f64(got, sample, channel).unwrap(), if i % 3 == 0 { 0. } else { 1. });
                    }
                } else {
                    assert_eq!(got, original);
                }
            }
        }
    }
}
#[test]
fn operative_validation_obeys_actual_lower_limits_and_accumulates_work() {
    let (doc, q, guard, reads, p, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let owner = adopt(doc, guard, q.clone());
    let work = AtomicU64::new(0);
    let debit = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    let context = PreparedBackedThresholdContext::new(&owner, p, NormalBandLimits { max_rows: 1, max_pixels: 3 }, &control).unwrap();
    context.validate_operative(&control).unwrap();
    let before = q.usage().unwrap();
    context.validate_operative(&control).unwrap();
    let first = work.load(Ordering::Relaxed);
    context.validate_operative(&control).unwrap();
    assert!(work.load(Ordering::Relaxed) > first);
    assert!(reads.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap().resident, before.resident, "private bands dropped between visits; source cache is retained separately");
    assert!(context.render_band(Rect::new(0, 0, 8, 1), &control).is_err());
}
#[test]
fn real_storage_class_does_not_grant_profile_marker_or_physical_mask_semantics() {
    for bad in 0..4 {
        let (mut doc, q, guard, reads, p, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
        match bad {
            0 => doc.working_profile_interpretation = WorkingProfileInterpretation::Icc,
            1 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[f64::from_bits(1)]).unwrap(),
            2 => doc.layers[0].fill_opacity = 0.5,
            _ => doc.layers[0].import_compositing.as_mut().unwrap().composite_encoding = SampleEncoding::NonlinearProfile,
        }
        let owner = adopt(doc, guard, q.clone());
        assert!(PreparedBackedThresholdContext::new(&owner, p, NormalBandLimits::default(), &ctl()).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}
#[test]
fn cancellation_work_and_profile_mismatch_drop_private_context_fees() {
    let (doc, q, guard, _, p, _) = fixture(ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
    let owner = adopt(doc, guard, q.clone());
    let before = q.usage().unwrap();
    assert!(PreparedBackedThresholdContext::new(&owner, Builtin::SGray.profile(), NormalBandLimits::default(), &ctl()).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(PreparedBackedThresholdContext::new(&owner, p, NormalBandLimits::default(), &c).is_err());
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(PreparedBackedThresholdContext::new(&owner, p, NormalBandLimits::default(), &c).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
