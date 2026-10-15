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
pub(super) fn ctl() -> ReadControl<'static> {
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
pub(super) fn fixture(
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
    point_session_with_colors(count, false)
}
fn point_session_with_colors(count: usize, colors: bool) -> (Session, BackingQuota, Arc<AtomicU64>) {
    point_session_with_values(count, colors, None)
}
fn point_session_with_values(count: usize, colors: bool, integer: Option<i32>) -> (Session, BackingQuota, Arc<AtomicU64>) {
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
                0 if colors => FilterValue::Color {
                    components: [(-0.0f32).to_bits(), 0x7fc12345, 0.5f32.to_bits(), 1f32.to_bits()].map(ExactNumber::Float32Bits).to_vec(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
                0 => FilterValue::Number(match integer {
                    Some(value) => ExactNumber::Signed { width: 32, value: i64::from(value) },
                    None => ExactNumber::Float32Bits(if i == 0 { (-0.0f32).to_bits() } else { 0x7fc12345 }),
                }),
                1 => FilterValue::Bool(i % 2 == 0),
                _ => FilterValue::Enum { domain: "declared-metadata-domain".into(), value: i as i32 },
            },
        })
        .collect();
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, extra)), q.clone(), &ctl()).unwrap();
    let mut s = Session::new();
    // Deliberately install metadata-only storage with test-private primitives.
    // Public generic Point adoption correctly refuses this unqualified recipe.
    let state = crate::DocState::new_accounted(owner, None, 16, &ctl()).unwrap();
    s.publish_document_state(state, &ctl()).unwrap();
    (s, q, reads)
}
fn request(s: &Session) -> Value {
    json!({"filterId":s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}

#[test]
fn point_signed32_metadata_keeps_exact_bounds_and_refuses_other_widths_without_pixel_reads() {
    for integer in [i32::MIN, i32::MAX] {
        let (s, q, reads) = point_session_with_values(1, false, Some(integer));
        let baseline = q.usage().unwrap();
        let reply = prepare(&s, &request(&s), &ctl()).unwrap();
        assert_eq!(reply.value()["arguments"][0]["state"]["value"]["Number"]["Signed"], json!({"width":32,"value":integer}));
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(q.usage().unwrap().index > baseline.index);
        let mut node = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
        assert!(metadata(&node, &ctl()).is_err());
        for value in [
            ExactNumber::Signed { width: 64, value: 0 },
            ExactNumber::Signed { width: 32, value: i64::from(i32::MAX) + 1 },
            ExactNumber::Unsigned { width: 32, value: 0 },
        ] {
            node.arguments[0].value = FilterValue::Number(value);
            assert!(point_metadata(&node, &ctl()).is_err());
        }
        drop(reply);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[test]
fn point_literal_color_metadata_keeps_raw_words_in_a_paid_reply_without_reading_pixels() {
    let (s, q, reads) = point_session_with_colors(10, true);
    let before = s.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    let reply = prepare(&s, &request(&s), &ctl()).unwrap();
    let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
    let node = &stack.nodes[0];
    let ordinary = crate::xcf_filter_cmds::build_inspect_reply(node.id, stack, node).unwrap();
    assert!(ordinary["arguments"][0]["state"]["unresolvedStructuredValue"].is_string());
    assert_eq!(reply.value()["filterId"], node.id.0.to_string());
    assert_eq!(reply.value()["argumentCount"], 10);
    assert_eq!(reply.value()["operation"], ordinary["operation"]);
    assert_eq!(reply.value()["arguments"][0]["state"]["value"]["Color"]["components"][0]["Float32Bits"], (-0.0f32).to_bits());
    assert_eq!(reply.value()["arguments"][0]["state"]["value"]["Color"]["components"][1]["Float32Bits"], 0x7fc12345u32);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(q.usage().unwrap().index > baseline.index);
    assert!(s.active().unwrap().doc.shares_state(&before));
    drop(reply);
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
fn point_literal_color_inspection_refuses_other_shapes_without_widening_threshold_metadata() {
    let (s, _, _) = point_session_with_colors(1, true);
    let mut node = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
    assert!(point_metadata(&node, &ctl()).is_ok());
    assert!(metadata(&node, &ctl()).is_err());
    let original = node.arguments[0].value.clone();
    for kind in 0..4 {
        node.arguments[0].value = original.clone();
        let FilterValue::Color { components, encoding, profile } = &mut node.arguments[0].value else { panic!("color fixture") };
        match kind {
            0 => {
                components.pop();
            }
            1 => components[0] = ExactNumber::Float64Bits(0),
            2 => *encoding = SampleEncoding::Linear,
            _ => *profile = Some(photocraft_doc::item_identity::ResourceId(1)),
        }
        assert!(point_metadata(&node, &ctl()).is_err());
    }
    let deny = |_| Err(BackingError::Work);
    assert!(point_metadata(&node, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
}
#[test]
fn point_filter_complete_flat_arguments_and_project_discovery_are_paid_immutable_metadata() {
    let (s, q, reads) = point_session(10);
    let p = request(&s);
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let history = st.history.clone();
    let journal = s.journal.clone();
    let baseline = q.usage().unwrap();
    let reply = prepare(&s, &p, &ctl()).unwrap();
    assert_eq!(reply.command(), "xcf.filter.inspect");
    assert_eq!(reply.value()["argumentCount"], 10);
    assert_eq!(reply.value()["argumentsTruncated"], false);
    let stack = &st.doc.xcf_state.as_ref().unwrap().filter_stacks[0];
    let node = &stack.nodes[0];
    let _oracle = q.reserve(QuotaKind::Index, point_envelope(point_metadata(node, &ctl()).unwrap()).unwrap()).unwrap();
    let expected = crate::xcf_filter_cmds::build_inspect_reply(node.id, stack, node).unwrap();
    assert_eq!(reply.value(), &expected);
    assert_eq!(reply.value()["arguments"][0]["state"]["value"]["Number"]["Float32Bits"], (-0.0f32).to_bits());
    let project = crate::accounted_xcf_inspect::prepare(&s, &Value::Null, &ctl()).unwrap();
    assert_eq!(project.command(), "xcf.inspect");
    assert_eq!(project.value()["filters"]["entries"][0]["id"], p["filterId"]);
    assert!(q.usage().unwrap().index > baseline.index);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(st.doc.shares_state(&before));
    assert!(st.history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
    drop(project);
    drop(expected);
    drop(_oracle);
    drop(reply);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn point_only_bound_does_not_enlarge_threshold_arguments_or_recursive_value_admission() {
    let (s, q, _) = point_session(10);
    let node = &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert!(point_metadata(node, &ctl()).is_ok());
    assert!(metadata(node, &ctl()).is_err());
    assert_eq!((FILTER_BOUNDS.nodes, FILTER_BOUNDS.objects, FILTER_BOUNDS.members, FILTER_BOUNDS.depth, FILTER_BOUNDS.max_string), (256, 32, 256, 8, 4096));
    let _fee = q.reserve(QuotaKind::Index, 32768).unwrap();
    let mut invalid = node.clone();
    invalid.arguments.push(invalid.arguments[0].clone());
    assert!(point_metadata(&invalid, &ctl()).is_err());
    invalid.arguments.pop();
    invalid.arguments[0].value = FilterValue::Config(Vec::new());
    assert!(point_metadata(&invalid, &ctl()).is_err());
    invalid.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0));
    assert!(point_metadata(&invalid, &ctl()).is_err());
}
#[test]
fn point_query_control_quota_canonical_id_and_filter_edits_remain_closed() {
    let (mut s, q, _) = point_session(10);
    let p = request(&s);
    let before = s.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    for value in
        [json!({}), json!({"filterId":"01"}), json!({"filterId":"0"}), json!({"filterId":"18446744073709551616"}), json!({"filterId":"100","extra":true})]
    {
        assert!(prepare(&s, &value, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let deny = |_| Err(BackingError::Work);
    assert!(prepare(&s, &p, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(prepare(&s, &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(prepare(&s, &p, &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(s.execute("xcf.filter.enable", json!({"filterId":p["filterId"],"enabled":false})).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
