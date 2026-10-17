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
type Fixture = (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>);
fn fixture(mode: ColorMode, sample: SampleType, stored: SampleEncoding) -> Fixture {
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

// CURRENT metadata only: the actual paid native fixture is not original-XCF proof.
fn plain_current_headless() -> (Headless, BackingQuota, Arc<AtomicU64>) {
    let (mut document, q, guard, reads, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let state = document.xcf_state.as_mut().unwrap();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.items.retain(|item| matches!(item.item, ItemRef::Layer(_)));
    let owner = AccountedDocument::from_owned(document, guard, q.clone(), &ctl()).unwrap();
    let mut h = Headless::new();
    h.session.add_accounted_document(owner, None, &ctl()).unwrap();
    (h, q, reads)
}
#[test]
fn plain_current_project_routes_by_command_without_filter_or_raw_escape() {
    let (mut h, q, reads) = plain_current_headless();
    assert!(h.has_owned_inspection_command("xcf.inspect"));
    assert!(!h.has_owned_filter_inspection());
    assert!(!h.has_owned_inspection_command("xcf.filter.inspect"));
    assert!(!h.has_owned_inspection_command("xcf.assess"));
    let baseline = q.usage().unwrap();
    let snapshot = h.session.active().unwrap().doc.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.clone();
    for wait in [false, true] {
        let reply = h.inspect_owned_command_bounded("xcf.inspect", &Value::Null, wait).unwrap();
        assert_eq!(reply.command(), "xcf.inspect");
        assert_eq!(reply.value()["filters"]["total"], 0);
        assert_eq!(reply.value()["items"]["total"], 1);
        assert!(q.usage().unwrap().index > baseline.index);
        drop(reply);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let wire = h.inspect_owned_command_cli_bounded("xcf.inspect", &Value::Null, true, crate::budgets::MAX_RESPONSE_BYTES).unwrap();
    let response: Value = serde_json::from_slice(wire.bytes()).unwrap();
    assert_eq!(response["command"], "xcf.inspect");
    drop(wire);
    assert!(h.inspect_owned_filter_bounded(&serde_json::json!({"filterId":"100"}), true).is_err());
    assert!(h.command_run("xcf.inspect", Value::Null).is_err());
    assert!(h.handle("engine.execute", serde_json::json!({"command":"xcf.inspect"})).is_err());
    assert_eq!(h.batch(&serde_json::json!({"steps":[{"command":"xcf.inspect"}]})).unwrap()["failed"], 1);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(h.session.active().unwrap().doc.shares_state(&snapshot));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal, journal);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn plain_project_authorization_work_and_threshold_refusal_are_final() {
    let (mut h, q, _) = plain_current_headless();
    fn deny(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        assert_eq!(id, "xcf.inspect");
        Err(photocraft_engine::EngineError::Other("plain project denied".into()))
    }
    let baseline = q.usage().unwrap();
    h.session.authorize = Some(deny);
    assert!(h.inspect_owned_command_bounded("xcf.inspect", &Value::Null, true).is_err());
    h.session.authorize = None;
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &denied).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    for histogram in [false, true] {
        let (mut threshold, q, _) = headless(histogram);
        let baseline = q.usage().unwrap();
        assert!(threshold.has_owned_inspection_command("xcf.inspect"));
        assert!(threshold.has_owned_inspection_command("xcf.filter.inspect"));
        assert!(threshold.inspect_owned_command_bounded("xcf.inspect", &serde_json::json!({"limit":0}), true).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
fn plain_current_retained_rpc_preserves_id_and_project_envelope() {
    use std::sync::Mutex;
    let (h, q, reads) = plain_current_headless();
    let baseline = q.usage().unwrap();
    let h = Mutex::new(h);
    let mut bytes = Vec::new();
    crate::rpc::serve_lines(
        &h,
        std::io::Cursor::new(b"{\"id\":\"plain\",\"method\":\"engine.execute\",\"params\":{\"command\":\"xcf.inspect\",\"wait\":false}}\n"),
        &mut bytes,
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["id"], "plain");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"]["items"]["total"], 1);
    assert_eq!(reply["result"]["filters"]["total"], 0);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(q.usage().unwrap(), baseline);
}

// Actual-original continuation is distinct from the synthetic CURRENT fixtures.
mod combined_properties_continuation {
    use super::*;
    use photocraft_engine::DocumentSnapshot;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!("photocraft-compound-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn hash(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|b| format!("{b:02x}")).collect()
    }
    fn original(histogram: bool) -> Vec<u8> {
        let (packet, pin, name) = if histogram {
            ("threshold-precision-evidence", "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142", "rgb-double_non_linear.channel0.xcf")
        } else {
            ("alpha-threshold-precision-evidence", "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263", "rgb-double_non_linear.half.xcf")
        };
        let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires SHA-pinned precision sources")).join(packet);
        assert!(std::fs::metadata(root.join("SHA256SUMS.json")).unwrap().len() <= 1 << 20);
        let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
        assert_eq!(hash(&bytes), pin);
        let manifest: Value = serde_json::from_slice(&bytes).unwrap();
        let n = std::fs::metadata(root.join(name)).unwrap().len();
        assert!(n > 0 && n <= 16 << 20);
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(hash(&bytes), manifest[name].as_str().unwrap());
        bytes
    }
    type MaskStorage = (photocraft_raster::backing::NativePlaneDescriptor, Option<(photocraft_geom::TileCoord, Vec<u8>)>);
    #[derive(Debug, PartialEq, Eq)]
    struct RawStorage {
        layer: photocraft_raster::backing::NativePlaneDescriptor,
        layer_full_tile: bool,
        layer_bytes: Vec<u8>,
        masks: Vec<MaskStorage>,
    }
    #[derive(Debug)]
    struct NativeSnapshot {
        storage: RawStorage,
        // Copied payload/metadata drop before their genuine same-ledger derivative fee.
        _lease: QuotaLease,
    }
    impl PartialEq for NativeSnapshot {
        fn eq(&self, other: &Self) -> bool {
            self.storage == other.storage
        }
    }
    impl Eq for NativeSnapshot {}
    fn native(snapshot: &DocumentSnapshot) -> NativeSnapshot {
        use photocraft_raster::{backing::NativePlaneDescriptor, checked_view::NativeTileBytes};
        let LayerContent::BackedRaster(layer) = &snapshot.layers[0].content else { panic!("source is not native backed") };
        assert_eq!(snapshot.layers.len(), 1);
        assert_eq!(layer.operative_bounds(), photocraft_geom::Rect::new(0, 0, 8, 6));
        let masks = &snapshot.xcf_state.as_ref().unwrap().filter_masks;
        assert!(masks.len() <= 1);
        let full_layer = 256usize.checked_mul(256).unwrap().checked_mul(layer.format().bytes_per_pixel()).unwrap();
        let mut payload = full_layer;
        for mask in masks {
            assert!(mask.surface.tile_count() <= 1);
            payload = payload.checked_add(256 * 256 * mask.surface.format().bytes_per_pixel()).unwrap();
        }
        // Double actual maximum payload admits allocator excess; 4096 prices the
        // bounded descriptor/tuple/vector headers. No native data is narrowed.
        let admitted = payload.checked_mul(2).unwrap().checked_add(4096).unwrap() as u64;
        let mut fee = layer.quota().reserve(QuotaKind::Resident, admitted).unwrap();
        fn copy(bytes: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.try_reserve_exact(bytes.len()).unwrap();
            assert!(out.capacity() <= bytes.len().checked_mul(2).unwrap());
            for chunk in bytes.chunks(64 * 1024) {
                ctl().check(chunk.len() as u64).unwrap();
                out.extend_from_slice(chunk);
            }
            out
        }
        let pin = layer.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
        let (layer_full_tile, layer_bytes) = match pin.encoded() {
            NativeTileBytes::FullTile(bytes) => {
                assert_eq!(bytes.len(), full_layer);
                (true, copy(bytes))
            }
            NativeTileBytes::DefaultPixel(_) => panic!("backed authoritative tile was not full native storage"),
        };
        drop(pin);
        let mut copied_masks = Vec::new();
        copied_masks.try_reserve_exact(masks.len()).unwrap();
        assert!(copied_masks.capacity() <= 2 * masks.len());
        let mut actual = layer_bytes.capacity() + 4096;
        for mask in masks {
            ctl().check(1).unwrap();
            let descriptor = NativePlaneDescriptor::new(mask.surface.format(), mask.bounds, mask.surface.default_bytes()).unwrap();
            let mut tile = None;
            for (coord, bytes) in mask.surface.tiles() {
                assert_eq!(*coord, photocraft_geom::TileCoord::new(0, 0));
                assert_eq!(bytes.bytes().len(), 256 * 256 * mask.surface.format().bytes_per_pixel());
                let copied = copy(bytes.bytes());
                actual = actual.checked_add(copied.capacity()).unwrap();
                tile = Some((*coord, copied));
            }
            copied_masks.push((descriptor, tile));
        }
        assert!(actual as u64 <= admitted);
        fee.reconcile_capacity(actual as u64).unwrap();
        NativeSnapshot { storage: RawStorage { layer: layer.descriptor(), layer_full_tile, layer_bytes, masks: copied_masks }, _lease: fee }
    }

    fn retained(a: &DocumentSnapshot, b: &DocumentSnapshot) {
        assert_eq!(a.xcf_state.as_ref().unwrap().filter_stacks, b.xcf_state.as_ref().unwrap().filter_stacks);
        assert_eq!(a.icc_profile, b.icc_profile);
        assert_eq!(a.source_color_context, b.source_color_context);
        assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    }
    #[test]
    #[ignore = "UNRUN: SHA-pinned Alpha/Histogram originals, compound properties and actual native host continuation"]
    fn registered_compound_properties_native_reopen_discovery_undo_redo_and_resave_preserve_original_storage() {
        for histogram in [false, true] {
            let dir = Directory::new();
            let original = original(histogram);
            let source = dir.0.join("source.xcf");
            std::fs::write(&source, &original).unwrap();
            let mut h = Headless::trusted_local();
            drop(h.open_owned_xcf_keep_current(&source, &ctl()).unwrap());
            let before = h.session.active().unwrap().doc.clone();
            assert_eq!(before.accounted().unwrap().is_readonly_threshold_storage(), histogram);
            assert_eq!(before.accounted().unwrap().is_readonly_threshold_alpha_storage(), !histogram);
            let raw = native(&before);
            let layer = before.layers[0].id;
            let source_identity = h.session.active().unwrap().source.clone();
            assert_eq!(
                h.command_run("layer.setProps", serde_json::json!({"layer":layer.0,"name":"Current compound name","visible":false,"opacity":0.5})).unwrap(),
                Value::Null
            );
            let edited = h.session.active().unwrap().doc.clone();
            assert_eq!(edited.layers[0].name, "Current compound name");
            assert!(!edited.layers[0].visible);
            assert_eq!(edited.layers[0].opacity.to_bits(), 0.5f32.to_bits());
            assert_eq!(h.session.active().unwrap().history.past_len(), 1);
            retained(&before, &edited);
            assert_eq!(native(&edited), raw);
            assert_eq!(edited.layers[0].content, before.layers[0].content);
            let item = edited.xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == ItemRef::Layer(layer)).unwrap();
            assert_eq!(item.name, "Current compound name");
            assert!(!item.visible);
            assert!(item.custom_name);
            let first = dir.0.join("compound.pcraft");
            h.save_owned_native_active(Some(&first), Some("pcraft"), &ctl()).unwrap();
            assert_eq!(h.session.active().unwrap().source, source_identity);
            assert!(h.session.active().unwrap().source_read_only);
            assert_eq!(std::fs::read(&source).unwrap(), original);
            let archive_bytes = std::fs::read(&first).unwrap();
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current(&first, &ctl()).unwrap());
            let saved = reopened.session.active().unwrap().doc.clone();
            assert_eq!(saved.name, "compound.pcraft");
            assert_eq!(saved.xcf_state, edited.xcf_state);
            retained(&before, &saved);
            assert_eq!(native(&saved), raw);
            assert!(saved.quota().unwrap().shares_ledger(reopened.session.accounted_open_quota().unwrap()));
            let discovery = reopened.inspect_owned_command_bounded("xcf.inspect", &Value::Null, true).unwrap();
            let rows = discovery.value()["items"]["entries"].as_array().unwrap();
            let item = rows.iter().find(|v| v["item"]["kind"] == "layer").unwrap();
            assert_eq!(item["name"], "Current compound name");
            assert_eq!(item["visible"], false);
            assert_eq!(discovery.value()["filters"]["entries"][0]["id"], saved.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string());
            drop(discovery);
            reopened.command_run("layer.setProps", serde_json::json!({"visible":true,"opacity":0.75})).unwrap();
            let changed = reopened.session.active().unwrap().doc.clone();
            assert!(changed.layers[0].visible);
            assert_eq!(changed.layers[0].opacity.to_bits(), 0.75f32.to_bits());
            retained(&before, &changed);
            assert_eq!(native(&changed), raw);
            reopened.command_run("edit.undo", serde_json::json!({})).unwrap();
            let undone = &reopened.session.active().unwrap().doc;
            assert_eq!(undone.name, "compound.pcraft");
            assert!(!undone.layers[0].visible);
            assert_eq!(undone.layers[0].opacity.to_bits(), 0.5f32.to_bits());
            assert_eq!(native(undone), raw);
            reopened.command_run("edit.redo", serde_json::json!({})).unwrap();
            assert!(reopened.session.active().unwrap().doc.layers[0].visible);
            assert_eq!(reopened.session.active().unwrap().doc.layers[0].opacity.to_bits(), 0.75f32.to_bits());
            let last = dir.0.join("continued-compound.pcraft");
            reopened.save_owned_native_active(Some(&last), Some("pcraft"), &ctl()).unwrap();
            assert_eq!(std::fs::read(&first).unwrap(), archive_bytes);
            assert_eq!(std::fs::read(&source).unwrap(), original);
            let alias = dir.0.join("source-alias.pcraft");
            std::fs::hard_link(&source, &alias).unwrap();
            let before_denial = h.session.active().unwrap().doc.clone();
            assert!(h.save_owned_native_active(Some(&alias), Some("pcraft"), &ctl()).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before_denial));
            // Native reopening retains the original inert fingerprint exclusion,
            // without treating the serialized source path as read authority.
            let native_before_denial = reopened.session.active().unwrap().doc.clone();
            assert!(reopened.save_owned_native_active(Some(&alias), Some("pcraft"), &ctl()).is_err());
            assert!(reopened.session.active().unwrap().doc.shares_state(&native_before_denial));
            assert_eq!(std::fs::read(&source).unwrap(), original);
            let mut final_open = Headless::trusted_local();
            drop(final_open.open_owned_native_keep_current(&last, &ctl()).unwrap());
            let final_doc = &final_open.session.active().unwrap().doc;
            assert_eq!(final_doc.name, "continued-compound.pcraft");
            assert_eq!(final_doc.layers[0].name, "Current compound name");
            assert!(final_doc.layers[0].visible);
            assert_eq!(final_doc.layers[0].opacity.to_bits(), 0.75f32.to_bits());
            retained(&before, final_doc);
            assert_eq!(native(final_doc), raw);
        }
    }
}
