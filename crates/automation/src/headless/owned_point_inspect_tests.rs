//! UNRUN actual Point storage metadata queries. The paid CURRENT fixture
//! deliberately has an unqualified operation name: inspection is not source/render/edit authority.
use super::*;
use photocraft_cms::{Builtin, Profile};
use photocraft_doc::{
    BackedLayer, Document, FilterMask, ImportCompositing, Layer, PixelFormat, SampleType, Size, SourceColorContext, XcfState,
    import_compositing::{GimpComposition, ResolvedGimpComposition},
    item_identity::MaskId,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_doc::{
    ColorMode, LayerContent, SampleEncoding, WorkingProfileInterpretation,
    accounted_document::AccountedDocument,
    import_compositing::{GimpOperator, GimpSpace},
    item_identity::{FilterId, ItemRef},
};
use photocraft_engine::Session;
use photocraft_raster::backing::{
    NativePlaneDescriptor, ReadControl,
    quota::{QuotaKind, QuotaLease},
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackedRaster, BackingError, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits},
    },
};
use serde_json::Value;
use serde_json::json;
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
    let mut doc = Document::new("Persisted document title", Size::new(8, 6), mode, sample);
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
                schema_epoch: "xcf-22-stock-threshold-alpha-precision-v1".into(),
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
                compatibility_epoch: "xcf-22-stock-threshold-alpha-precision-v1".into(),
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

fn point_session(count: usize) -> (Session, BackingQuota, Arc<AtomicU64>) {
    let (mut doc, q, guard, reads, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let extra = q.reserve(QuotaKind::Index, 32768).unwrap();
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = "current:metadata-not-a-renderer".into();
    node.operation.schema_epoch = "current-storage-only".into();
    node.composition.as_mut().unwrap().compatibility_epoch = "current-storage-only".into();
    node.source_display_name = Some("\u{03bb}".repeat(1000));
    node.arguments = (0..count)
        .map(|i| FilterArgument {
            name: format!("field-{i}"),
            value: match i % 3 {
                0 => FilterValue::Number(ExactNumber::Float32Bits(if i == 0 { (-0.0f32).to_bits() } else { 0x7fc12345 })),
                1 => FilterValue::Bool(i % 2 == 0),
                _ => FilterValue::Enum { domain: "declared-metadata-domain".into(), value: i as i32 },
            },
        })
        .collect();
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, extra)), q.clone(), &ctl()).unwrap();
    let mut s = Session::new();
    s.add_accounted_document(owner, None, &ctl()).unwrap();
    (s, q, reads)
}
fn request(s: &Session) -> Value {
    json!({"filterId":s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}

#[test]
fn actual_point_class_headless_routes_paid_filter_and_project_without_job_or_model_mutation() {
    let (s, q, reads) = point_session(10);
    let mut h = Headless::new();
    h.session = s;
    let p = request(&h.session);
    let before = h.session.active().unwrap().doc.clone();
    let journal = h.session.journal.clone();
    let baseline = q.usage().unwrap();
    assert!(h.has_owned_filter_inspection());
    assert!(h.has_owned_inspection_command("xcf.inspect"));
    assert!(h.has_owned_inspection_command("xcf.filter.inspect"));
    for command in ["xcf.inspect", "xcf.filter.inspect"] {
        let params = if command == "xcf.inspect" { Value::Null } else { p.clone() };
        let reply = h.inspect_owned_command_with_read_control(command, &params, false, &ctl()).unwrap();
        assert_eq!(reply.command(), command);
        assert!(reply.quota().shares_ledger(&q));
        drop(reply);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(h.session.journal, journal);
    let wire = h.inspect_owned_filter_cli_bounded(&p, true, crate::budgets::MAX_RESPONSE_BYTES).unwrap();
    let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
    assert_eq!(value["command"], "xcf.filter.inspect");
    assert_eq!(value["result"]["argumentCount"], 10);
    drop(wire);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn point_headless_keeps_authorization_control_and_no_fallback_boundaries() {
    let (s, q, _) = point_session(10);
    let mut h = Headless::new();
    h.session = s;
    let p = request(&h.session);
    let before = h.session.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny point metadata".into())));
    for command in ["xcf.inspect", "xcf.filter.inspect"] {
        let params = if command == "xcf.inspect" { Value::Null } else { p.clone() };
        assert!(h.inspect_owned_command_with_read_control(command, &params, true, &ctl()).is_err());
    }
    h.session.authorize = None;
    let deny = |_| Err(BackingError::Work);
    assert!(h.inspect_owned_command_with_read_control("xcf.filter.inspect", &p, false, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(h.inspect_owned_command_with_read_control("xcf.filter.set", &p, false, &ctl()).is_err());
    assert!(h.session.execute("xcf.filter.inspect", p.clone()).is_err());
    assert!(h.inspect_owned_filter_bounded(&serde_json::json!({"filterId":"999"}), false).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), baseline);
}
