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
fn ctl() -> ReadControl<'static> {
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
fn fixture(
    mode: ColorMode,
    sample: SampleType,
    stored: SampleEncoding,
) -> (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>) {
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 0, index: 16 << 20 });
    // Actual source/map/default/model fixture allocation is prospectively paid.
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(quota.reserve(QuotaKind::Resident, 2 << 20).unwrap());
    let profile = if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() };
    let mut doc = Document::new("current recipe", Size::new(8, 6), mode, sample);
    doc.icc_profile = Some(profile.to_bytes());
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
        photocraft_color::try_write_sample_f64(pixel, sample, mode.color_channels(), if i % 2 == 0 { 0.25 } else { 0.75 }).unwrap();
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
                name: "gimp:threshold-alpha".into(),
                stored_version: "0:0".into(),
                schema_epoch: crate::gimp_filters::THRESHOLD_ALPHA_PRECISION_EPOCH.into(),
            },
            arguments: vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) }],
            source_display_name: None,
            source_icon: None,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: Some(GimpComposition {
                compatibility_epoch: crate::gimp_filters::THRESHOLD_ALPHA_PRECISION_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: 2061861457,
                stored_composite_space: 2061861457,
                stored_operator: 2061861457,
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
fn adopt(doc: Document, guard: Arc<dyn Any + Send + Sync>, q: BackingQuota) -> AccountedDocument {
    AccountedDocument::from_owned_readonly_threshold_alpha_storage(doc, guard, q, &ctl()).unwrap()
}
#[test]
fn current_rgb_gray_native_bands_preserve_colors_all_depths_and_encodings() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                let (doc, q, guard, reads, p, raw) = fixture(mode, sample, stored);
                let owner = adopt(doc, guard, q);
                let before = reads.load(Ordering::Relaxed);
                let prepared = PreparedBackedThresholdAlphaContext::new(&owner, p, NormalBandLimits::default(), &ctl()).unwrap();
                assert_eq!(reads.load(Ordering::Relaxed), before, "context metadata/default checks do not certify/read source padding");
                let band = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
                let format = band.format();
                let color_bytes = mode.color_channels() * sample.bytes();
                for (i, (got, input)) in band.bytes().chunks_exact(format.bytes_per_pixel()).zip(raw.chunks_exact(format.bytes_per_pixel())).enumerate() {
                    assert_eq!(&got[..color_bytes], &input[..color_bytes]);
                    assert_eq!(photocraft_color::try_read_sample_f64(got, sample, mode.color_channels()).unwrap(), if i % 2 == 0 { 0. } else { 1. });
                }
                assert_eq!(prepared.layer_id(), owner.document().layers[0].id);
                assert_eq!(
                    owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value,
                    FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits()))
                );
            }
        }
    }
}
#[test]
fn storage_only_marker_does_not_grant_contradictory_outer_or_nonempty_mask_semantics() {
    for bad in 0..4 {
        let (mut doc, q, guard, reads, p, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
        let state = doc.xcf_state.as_mut().unwrap();
        match bad {
            0 => state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
            1 => state.filter_stacks[0].nodes[0].source_clipping = Some(2),
            2 => state.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[f64::from_bits(1)]).unwrap(),
            _ => doc.layers[0].fill_opacity = f32::NAN,
        }
        let owner = adopt(doc, guard, q);
        assert!(PreparedBackedThresholdAlphaContext::new(&owner, p, NormalBandLimits::default(), &ctl()).is_err());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}
#[test]
fn profile_bits_work_and_cancellation_fail_privately_without_native_reads() {
    let (doc, q, guard, reads, p, _) = fixture(ColorMode::Grayscale, SampleType::F32, SampleEncoding::Linear);
    let owner = adopt(doc, guard, q.clone());
    let before = q.usage().unwrap();
    assert!(PreparedBackedThresholdAlphaContext::new(&owner, Builtin::GrayGamma22.profile(), NormalBandLimits::default(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(
        PreparedBackedThresholdAlphaContext::new(&owner, p, NormalBandLimits::default(), &c),
        Err(BackedNormalError::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), before);
    let work = |_: u64| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(PreparedBackedThresholdAlphaContext::new(&owner, p, NormalBandLimits::default(), &c).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
fn shared_validator_rejects_f64_storage_and_preserves_disabled_pointstack_behavior() {
    let (mut doc, _, _, _, p, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let nodes = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
    nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits()));
    assert!(crate::gimp_filters::validated_enabled_threshold_alpha(nodes, 3, SampleEncoding::NonlinearProfile, 22).is_err());
    nodes[0].enabled = false;
    assert!(crate::gimp_filters::validated_current_threshold_alpha(nodes, 3, SampleEncoding::NonlinearProfile, 22).is_err());
    assert!(
        crate::gimp_filters::PointStack::from_nodes_with_interrupt(nodes, p, SampleEncoding::NonlinearProfile, 22, &Interrupt::NONE).is_ok(),
        "old disabled branch remains untouched"
    );
}

#[path = "backed_threshold_alpha_disabled_tests.rs"]
mod disabled;
