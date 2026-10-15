//! UNRUN retained Headless/RPC metadata inspection drafts. Synthetic current
//! models establish storage ownership only, never original-XCF eligibility.
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
fn headless(histogram: bool) -> (Headless, BackingQuota, Arc<AtomicU64>) {
    let (mut document, q, guard, reads, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    if histogram {
        let node = &mut document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.name = "gimp:threshold".into();
        node.operation.schema_epoch = "xcf-22-stock-threshold-precision-v1".into();
        node.composition.as_mut().unwrap().compatibility_epoch = node.operation.schema_epoch.clone();
        node.composition.as_mut().unwrap().stored_blend_space = -1_565_818_287;
        node.composition.as_mut().unwrap().stored_composite_space = -1_565_818_287;
        node.composition.as_mut().unwrap().stored_operator = -1_565_818_287;
        node.arguments = vec![
            FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 5 } },
            FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0f32).to_bits())) },
            FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
        ];
    }
    let owner = if histogram {
        AccountedDocument::new_readonly_threshold_storage(document, guard, q.clone(), &ctl()).unwrap()
    } else {
        AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q.clone(), &ctl()).unwrap()
    };
    let mut h = Headless::new();
    h.session.add_accounted_document(owner, None, &ctl()).unwrap();
    (h, q, reads)
}
fn params(h: &Headless) -> Value {
    serde_json::json!({"filterId":h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}
#[test]
fn typed_query_retains_actual_charge_without_native_reads_or_session_mutation() {
    for histogram in [false, true] {
        let (mut h, q, reads) = headless(histogram);
        let p = params(&h);
        let before = h.session.active().unwrap().doc.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        let baseline = q.usage().unwrap();
        let reply = h.inspect_owned_filter_bounded(&p, false).unwrap();
        assert_eq!(reply.value()["filterId"], p["filterId"]);
        assert!(q.usage().unwrap().index > baseline.index);
        assert!(reply.quota().shares_ledger(&q));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        drop(reply);
        assert_eq!(q.usage().unwrap(), baseline);
        let wire = h.inspect_owned_filter_cli_bounded(&p, true, crate::budgets::MAX_RESPONSE_BYTES).unwrap();
        let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
        assert_eq!(value["command"], "xcf.filter.inspect");
        assert_eq!(value["result"]["filterId"], p["filterId"]);
    }
}
#[test]
fn bare_value_and_both_batch_forms_refuse_without_inspection_or_mutation() {
    let (mut h, q, reads) = headless(true);
    let p = params(&h);
    let before = h.session.active().unwrap().doc.clone();
    let baseline = q.usage().unwrap();
    assert!(h.command_run("xcf.filter.inspect", p.clone()).is_err());
    assert!(h.handle("engine.execute", serde_json::json!({"command":"xcf.filter.inspect","params":p})).is_err());
    for step in [
        serde_json::json!({"command":"xcf.filter.inspect","params":p}),
        serde_json::json!({"method":"engine.execute","params":{"command":"xcf.filter.inspect","params":p}}),
    ] {
        let result = h.batch(&serde_json::json!({"steps":[step]})).unwrap();
        assert_eq!(result["failed"], 1);
        assert_eq!(result["completed"], 0);
    }
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(h.session.active().unwrap().doc.shares_state(&before));
}
#[test]
fn retained_rpc_exact_envelope_keeps_fees_through_output_flush_and_raw_respond_refuses() {
    use std::io::Write;
    use std::sync::Mutex;
    let (h, q, reads) = headless(true);
    let p = params(&h);
    let request =
        serde_json::json!({"id":{"nested":[7,"escaped\""]},"method":"engine.execute","params":{"command":"xcf.filter.inspect","params":p,"wait":false}});
    let request = format!("{}\n", request);
    let h = Mutex::new(h);
    let baseline = q.usage().unwrap();
    let denied = crate::rpc::respond(&h, &request);
    assert_eq!(denied["ok"], false);
    assert_eq!(q.usage().unwrap(), baseline);
    struct Output {
        bytes: Vec<u8>,
        q: BackingQuota,
        baseline: photocraft_raster::backing::quota::QuotaUsage,
        flushed: bool,
    }
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            assert!(self.q.usage().unwrap().resident > self.baseline.resident);
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            assert!(self.q.usage().unwrap().resident > self.baseline.resident);
            self.flushed = true;
            Ok(())
        }
    }
    let mut output = Output { bytes: Vec::new(), q: q.clone(), baseline, flushed: false };
    crate::rpc::serve_lines(&h, std::io::Cursor::new(request.as_bytes()), &mut output).unwrap();
    let value: Value = serde_json::from_slice(&output.bytes).unwrap();
    assert_eq!(value["id"], serde_json::json!({"nested":[7,"escaped\""]}));
    assert_eq!(value["ok"], true);
    assert_eq!(value["result"]["filterId"], p["filterId"]);
    assert!(output.flushed);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
#[test]
fn authorization_control_unknown_fields_and_resident_legacy_behavior_are_preserved() {
    let (mut h, q, _) = headless(false);
    let p = params(&h);
    let baseline = q.usage().unwrap();
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny inspection".into())));
    assert!(h.inspect_owned_filter_bounded(&p, true).err().unwrap().to_string().contains("deny inspection"));
    h.session.authorize = None;
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(h.inspect_owned_filter_with_read_control(&p, true, &deny).is_err());
    for p in [serde_json::json!({}), serde_json::json!({"filterId":"0"}), serde_json::json!({"filterId":p["filterId"],"other":true})] {
        assert!(h.inspect_owned_filter_bounded(&p, true).is_err());
    }
    assert_eq!(q.usage().unwrap(), baseline);
    let mut ordinary = Headless::new();
    ordinary.session.add_document(Document::new("resident", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None).unwrap();
    assert!(!ordinary.has_owned_filter_inspection());
    assert!(ordinary.command_run("xcf.filter.inspect", serde_json::json!({"filterId":"17"})).unwrap_err().to_string().contains("filter"));
}

#[test]
fn hostile_rpc_id_is_null_error_without_copy_or_query() {
    use std::sync::Mutex;
    let (h, q, reads) = headless(true);
    let p = params(&h);
    let mut id = Value::Null;
    for _ in 0..9 {
        id = serde_json::json!([id]);
    }
    let request = serde_json::json!({"id":id,"method":"engine.execute","params":{"command":"xcf.filter.inspect","params":p}}).to_string();
    let h = Mutex::new(h);
    let before = q.usage().unwrap();
    let result = crate::rpc::respond(&h, &request);
    assert_eq!(result["id"], Value::Null);
    assert_eq!(result["ok"], false);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn owned_project_discovery_supplies_actual_filter_id_for_inspection_and_registered_edit() {
    for histogram in [false, true] {
        let (mut h, q, reads) = headless(histogram);
        let before = h.session.active().unwrap().doc.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        let baseline = q.usage().unwrap();
        let discovered = h.inspect_owned_command_bounded("xcf.inspect", &Value::Null, false).unwrap();
        assert_eq!(discovered.command(), "xcf.inspect");
        let identity = discovered.value()["filters"]["entries"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(discovered.value()["filters"]["total"], 1);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.journal, journal);
        drop(discovered);
        assert_eq!(q.usage().unwrap(), baseline);
        let inspected = h.inspect_owned_command_bounded("xcf.filter.inspect", &serde_json::json!({"filterId":identity}), true).unwrap();
        assert_eq!(inspected.command(), "xcf.filter.inspect");
        assert_eq!(inspected.value()["filterId"], identity);
        drop(inspected);
        h.command_run("xcf.filter.enable", serde_json::json!({"filterId":identity,"enabled":false})).unwrap();
        assert_eq!(h.session.active().unwrap().revision, revision + 1);
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        assert!(h.session.active().unwrap().history.can_redo());
        let page =
            h.inspect_owned_command_cli_bounded("xcf.inspect", &serde_json::json!({"offset":1,"limit":1}), true, crate::budgets::MAX_RESPONSE_BYTES).unwrap();
        let page: Value = serde_json::from_slice(page.bytes()).unwrap();
        assert_eq!(page["command"], "xcf.inspect");
        assert!(page["result"]["filters"]["entries"].as_array().unwrap().is_empty());
        assert!(h.session.active().unwrap().history.can_redo());
    }
}
#[test]
fn discovery_uses_original_authorization_id_and_only_two_typed_commands() {
    let (mut h, q, _) = headless(true);
    fn deny(command: &str, _: &Value) -> photocraft_engine::Result<()> {
        assert_eq!(command, "xcf.inspect");
        Err(photocraft_engine::EngineError::Other("discovery denied".into()))
    }
    let baseline = q.usage().unwrap();
    h.session.authorize = Some(deny);
    assert!(h.inspect_owned_command_bounded("xcf.inspect", &Value::Null, true).is_err());
    h.session.authorize = None;
    assert!(h.inspect_owned_command_bounded("xcf.assess", &Value::Null, true).is_err());
    for value in [
        serde_json::json!({"limit":0}),
        serde_json::json!({"limit":129}),
        serde_json::json!({"offset":100001}),
        serde_json::json!({"other":1}),
        serde_json::json!({"limit":null}),
    ] {
        assert!(h.inspect_owned_command_bounded("xcf.inspect", &value, true).is_err());
    }
    assert!(h.command_run("xcf.inspect", Value::Null).is_err());
    assert!(h.handle("engine.execute", serde_json::json!({"command":"xcf.inspect"})).is_err());
    assert_eq!(h.batch(&serde_json::json!({"steps":[{"command":"xcf.inspect"}]})).unwrap()["failed"], 1);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn retained_rpc_discovery_accepts_missing_params_and_keeps_legacy_envelope() {
    use std::sync::Mutex;
    let (h, q, reads) = headless(true);
    let baseline = q.usage().unwrap();
    let h = Mutex::new(h);
    let mut bytes = Vec::new();
    crate::rpc::serve_lines(&h, std::io::Cursor::new(b"{\"id\":5,\"method\":\"engine.execute\",\"params\":{\"command\":\"xcf.inspect\"}}\n"), &mut bytes)
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["id"], 5);
    assert_eq!(value["ok"], true);
    assert_eq!(value["result"]["filters"]["total"], 1);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), baseline);
}
