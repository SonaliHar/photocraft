//! UNRUN CURRENT display bridge drafts. The shared recipe fixture below carries
//! no original-source, full-padding or rendering enrollment proof.
use super::*;
use photocraft_cms::Builtin;
use photocraft_doc::{
    BackedLayer, Document, FilterMask, ImportCompositing, Layer, PixelFormat, SampleType, Size, SourceColorContext, XcfState,
    import_compositing::{GimpComposition, ResolvedGimpComposition},
    item_identity::MaskId,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_doc::{
    LayerContent, SampleEncoding, WorkingProfileInterpretation,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::{FilterId, ItemRef},
};
use photocraft_raster::backing::NativePlaneDescriptor;
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
fn current_document_display_native_depths_gray_repeat_and_alpha_zero_policy() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                let (doc, q, guard, reads, profile, raw) = fixture(mode, sample, stored);
                let owner = adopt(doc, guard, q);
                let prepared = PreparedBackedThresholdAlphaDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
                assert_eq!(reads.load(Ordering::Relaxed), 0);
                let rect = Rect::new(0, 0, 8, 6);
                let output = prepared.render_band(rect, &ctl()).unwrap();
                assert_eq!(output.rect(), rect);
                assert_eq!(output.pixels().len(), 48);
                for (i, (got, input)) in output.pixels().iter().zip(raw.chunks_exact(mode.color_channels().saturating_add(1) * sample.bytes())).enumerate() {
                    if i % 2 == 0 {
                        assert!(got.iter().all(|v| v.to_bits() == 0));
                    } else {
                        assert_eq!(got[3], 1.0);
                        for c in 0..3 {
                            let expected = photocraft_color::try_read_sample_f64(input, sample, if mode == ColorMode::Grayscale { 0 } else { c }).unwrap();
                            assert_eq!(got[c].to_bits(), expected.to_bits());
                        }
                    }
                }
                // Derived output is display storage, not a mutation of raw native owners.
                let LayerContent::BackedRaster(layer) = &owner.document().layers[0].content else { panic!() };
                let mut pixel = vec![0; layer.format().bytes_per_pixel()];
                layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &ctl()).unwrap();
                assert_eq!(pixel, raw[..pixel.len()]);
            }
        }
    }
}
#[test]
fn contained_roi_uses_actual_row_coordinates_and_output_outlives_preparation() {
    let (doc, q, guard, _, profile, raw) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let owner = adopt(doc, guard, q.clone());
    let before = q.usage().unwrap();
    let output = {
        let prepared = PreparedBackedThresholdAlphaDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
        prepared.render_band(Rect::new(1, 1, 4, 3), &ctl()).unwrap()
    };
    assert_eq!(output.pixels().len(), 6);
    for (j, pixel) in output.pixels().iter().enumerate() {
        let x = 1 + j % 3;
        let y = 1 + j / 3;
        let index = y * 8 + x;
        let raw = &raw[index * 32..index * 32 + 32];
        if index % 2 == 0 {
            assert!(pixel.iter().all(|v| v.to_bits() == 0));
        } else {
            assert_eq!(pixel[0].to_bits(), photocraft_color::try_read_sample_f64(raw, SampleType::F64, 0).unwrap().to_bits());
        }
    }
    assert!(q.usage().unwrap().resident > before.resident);
    drop(output);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn quota_roi_work_and_cancel_failures_publish_no_band_and_preserve_source() {
    let (doc, q, guard, reads, profile, raw) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let owner = adopt(doc, guard, q.clone());
    let prepared = PreparedBackedThresholdAlphaDocument::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    let before = q.usage().unwrap();
    for rect in [Rect::EMPTY, Rect::new(-1, 0, 8, 6), Rect::new(0, 0, 9, 6), Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX)] {
        assert!(prepared.render_band(rect, &ctl()).is_err());
    }
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), before);
    let blockade = q.reserve(QuotaKind::Resident, (64u64 << 20) - before.resident).unwrap();
    assert!(matches!(prepared.render_band(Rect::new(0, 0, 8, 6), &ctl()), Err(BackedNormalError::Quota(_))));
    assert_eq!(reads.load(Ordering::Relaxed), 0, "output reservation must precede backing reads");
    drop(blockade);
    assert_eq!(q.usage().unwrap(), before);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(prepared.render_band(Rect::new(0, 0, 8, 6), &denied), Err(BackedNormalError::Backing(BackingError::Work))));
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(prepared.render_band(Rect::new(0, 0, 8, 6), &cancelled), Err(BackedNormalError::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let LayerContent::BackedRaster(layer) = &owner.document().layers[0].content else { panic!() };
    let mut pixel = [0; 32];
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &ctl()).unwrap();
    assert_eq!(pixel, raw[..32]);
}
#[test]
fn zero_backdrop_factoring_matches_original_branch_for_signedzero_and_opacity() {
    use crate::gimp_composite::{WorkingSpace, normal_over_zero_backdrop};
    let working = WorkingSpace::new(Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
    for color in [-0.0, 0.0, 0.5000000000000001, 1.0] {
        for alpha in [-0.0, 0.0, 0.25, 1.0] {
            for opacity in [-0.0, 0.0, 0.5, 1.0] {
                let source = [color, 0.25, 0.75, alpha];
                // Independent original Normal branch: alpha==0 returns exact +zero
                // backdrop; otherwise ao=alpha+0*(1-alpha), then raw source colors.
                let a = alpha * opacity;
                let expected = if a == 0.0 { [0.0; 4] } else { [source[0], source[1], source[2], a + 0.0 * (1.0 - a)] };
                let got = normal_over_zero_backdrop(source, opacity).unwrap();
                assert_eq!(got.map(f64::to_bits), expected.map(f64::to_bits));
                assert_eq!(working.normal([0.0; 4], source, opacity, GimpOperator::Union).unwrap().map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
    assert!(normal_over_zero_backdrop([f64::NAN, 0.0, 0.0, 1.0], 1.0).is_err());
    assert!(normal_over_zero_backdrop([0.0, 0.0, 0.0, 2.0], 1.0).is_err());
    assert!(normal_over_zero_backdrop([0.0; 4], f64::INFINITY).is_err());
}
