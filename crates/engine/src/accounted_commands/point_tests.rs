//! UNRUN synthetic paid CURRENT Point transaction drafts. No original-source qualification.
use super::*;
use crate::Session;
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
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
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

fn session() -> (Session, BackingQuota) {
    session_for(false)
}
fn session_for(alien: bool) -> (Session, BackingQuota) {
    session_locked(alien, 0)
}
fn session_locked(alien: bool, lock: usize) -> (Session, BackingQuota) {
    session_member(alien, lock, false)
}
fn session_member(alien: bool, lock: usize, color: bool) -> (Session, BackingQuota) {
    let (mut doc, q, guard, _, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    let extra = q.reserve(QuotaKind::Index, 32768).unwrap();
    let layer = doc.layers[0].id;
    match lock {
        1 => doc.layers[0].locks.all = true,
        2 => doc.layers[0].locks.pixels = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true,
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.visibility = true,
        _ => {}
    }

    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity {
        name: "gegl:exposure".into(),
        stored_version: "1:0".into(),
        schema_epoch: photocraft_compose::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into(),
    };
    node.arguments = vec![
        FilterArgument { name: "exposure".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
        FilterArgument { name: "black-level".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.03125f32.to_bits())) },
    ];
    if alien {
        node.operation = OperationIdentity {
            name: "gegl:alien-map".into(),
            stored_version: "0:0".into(),
            schema_epoch: photocraft_compose::gimp_filters::ALIEN_MAP_PRECISION_EPOCH.into(),
        };
        node.arguments =
            vec![FilterArgument { name: "color-model".into(), value: FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 0 } }];
        for name in ["cpn-1-frequency", "cpn-2-frequency", "cpn-3-frequency"] {
            node.arguments.push(FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(1.25f32.to_bits())) });
        }
        for name in ["cpn-1-phaseshift", "cpn-2-phaseshift", "cpn-3-phaseshift"] {
            node.arguments.push(FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(25f32.to_bits())) });
        }
        for name in ["cpn-1-keep", "cpn-2-keep", "cpn-3-keep"] {
            node.arguments.push(FilterArgument { name: name.into(), value: FilterValue::Bool(false) });
        }
    }
    if color {
        node.operation = OperationIdentity {
            name: "gegl:color-overlay".into(),
            stored_version: "0:0".into(),
            schema_epoch: photocraft_compose::gimp_filters::COLOR_OVERLAY_PRECISION_EPOCH.into(),
        };
        node.arguments = vec![
            FilterArgument {
                name: "value".into(),
                value: FilterValue::Color {
                    components: [0.25f32, 0.5, 0.75, 0.5].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            },
            FilterArgument { name: "srgb".into(), value: FilterValue::Bool(true) },
        ];
        let policy = node.composition.as_mut().unwrap();
        policy.stored_blend_space = 1435337297;
        policy.stored_composite_space = 1435337297;
        policy.stored_operator = 1435337297;
    }
    node.composition.as_mut().unwrap().compatibility_epoch = photocraft_compose::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, extra)), q.clone(), &ctl()).unwrap();
    validate_recipe(&owner, &ctl()).unwrap();
    let mut s = Session::new();
    // Engine-internal CURRENT fixture: full operative validation above is
    // mandatory. Generic public structural adoption intentionally refuses Point.
    let history_states = s.prefs().performance.history_states.max(1) as usize;
    let state = crate::DocState::new_accounted(owner, None, history_states, &ctl()).unwrap();
    s.publish_document_state(state, &ctl()).unwrap();
    (s, q)
}
fn patch(bits: u32) -> Value {
    json!({"filterId":"100","arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":bits}}}]})
}
#[test]
fn bounded_flat_wire_preserves_bits_types_domains_and_exact_journal_depth() {
    let args:Vec<_>=(0..10).map(|i|json!({"name":format!("arg-{i}"),"value":if i%3==0 {json!({"Number":{"Float32Bits":(-0.0f32).to_bits()}})} else if i%3==1 {json!({"Bool":false})} else {json!({"Enum":{"domain":"existing-domain","value":-3}})}})).collect();
    let p = json!({"filterId":"18446744073709551615","arguments":args});
    let fields = wire("xcf.filter.set", &p).unwrap();
    assert_eq!(fields.count, 10);
    assert!(matches!(fields.arguments[0].unwrap().value,PointArgumentValue::Float32Bits(v) if v==(-0.0f32).to_bits()));
    assert_eq!(fields.arguments[2].unwrap().domain, Some("existing-domain"));
    assert_eq!(journal_depth("xcf.filter.set", &p), 5);
    for p in [
        json!({"filterId":"01","enabled":true}),
        json!({"filterId":"100","enabled":true,"coalesce":null}),
        json!({"filterId":"100","arguments":[{"name":"x","value":{"Config":[]}}]}),
        json!({"filterId":"100","opacityF64Bits":"0x3fe0000000000000"}),
        json!({"filterId":"100","arguments":[{"name":"x","value":{"Bool":true}},{"name":"x","value":{"Bool":false}}]}),
    ] {
        assert!(wire("xcf.filter.set", &p).is_none());
        assert_eq!(journal_depth("xcf.filter.set", &p), 4);
    }
    assert!(wire("xcf.filter.enable", &json!({"filterId":"100","enabled":true,"arguments":[]})).is_none());
}
#[test]
fn registered_point_patch_is_one_undo_and_exact_noop_keeps_redo_source_and_mask() {
    let (mut s, _q) = session();
    let before = s.active().unwrap().doc.clone();
    let reply = s.execute("xcf.filter.set", patch(0.75f32.to_bits())).unwrap();
    assert_eq!(reply, json!({"filterId":"100","updated":true}));
    let after = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(after.source_color_context, before.source_color_context);
    assert_eq!(after.icc_profile, before.icc_profile);
    assert_eq!(after.xcf_state.as_ref().unwrap().filter_stacks[0].revision, 1);
    assert_eq!(after.xcf_state.as_ref().unwrap().filter_masks[0].surface.format(), before.xcf_state.as_ref().unwrap().filter_masks[0].surface.format());
    assert!(s.undo_with_control(&ctl()).unwrap());
    let history = s.active().unwrap().history.clone();
    assert_eq!(s.execute("xcf.filter.set", patch(0.5f32.to_bits())).unwrap()["updated"], false);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert!(s.redo_with_control(&ctl()).unwrap());
    assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":"100","enabled":false})).unwrap()["updated"], true);
}
#[test]
fn current_and_staged_semantics_domain_and_type_refuse_without_publication() {
    let (mut s, _q) = session();
    let before = s.active().unwrap().doc.clone();
    for p in [
        patch(f32::NAN.to_bits()),
        json!({"filterId":"100","arguments":[{"name":"exposure","value":{"Bool":true}}]}),
        json!({"filterId":"100","arguments":[{"name":"missing","value":{"Number":{"Float32Bits":0}}}]}),
    ] {
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
}
#[test]
fn prepared_point_stale_view_history_cancel_and_work_preserve_state() {
    let (mut s, q) = session();
    let before = s.active().unwrap().doc.clone();
    let p = patch(0.75f32.to_bits());
    let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
    s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
    let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
    s.active_mut().unwrap().history.max_states += 1;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let stop = Interrupt::cancel_only(&|| true);
    assert!(Request::parse(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
    assert!(Request::parse(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }).is_err());
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).is_err());
    drop(blocker);
    assert!(s.active().unwrap().doc.shares_state(&before));
}

#[test]
fn existing_bool_enum_domain_and_semantic_recipe_are_independent_gates() {
    let (mut s, _q) = session_for(true);
    let before = s.active().unwrap().doc.clone();
    let bad = json!({"filterId":"100","arguments":[{"name":"color-model","value":{"Enum":{"domain":"forged-domain","value":0}}}]});
    assert!(s.execute("xcf.filter.set", bad).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let patch = json!({"filterId":"100","arguments":[{"name":"cpn-1-keep","value":{"Bool":true}},{"name":"color-model","value":{"Enum":{"domain":"gegl:alien-map/color-model","value":0}}}]});
    assert_eq!(s.execute("xcf.filter.set", patch).unwrap()["updated"], true);
    assert_eq!(s.active().unwrap().history.past_len(), 1);
}

#[test]
fn actual_current_locks_reject_content_or_changed_visibility_only() {
    for lock in 1..=4 {
        let (mut s, _q) = session_locked(false, lock);
        let before = s.active().unwrap().doc.clone();
        if lock < 4 {
            assert!(s.execute("xcf.filter.set", patch(0.75f32.to_bits())).is_err());
        } else {
            assert_eq!(s.execute("xcf.filter.set", patch(0.75f32.to_bits())).unwrap()["updated"], true);
            let changed = s.active().unwrap().doc.clone();
            assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":"100","enabled":true})).unwrap()["updated"], false);
            assert!(s.execute("xcf.filter.enable", json!({"filterId":"100","enabled":false})).is_err());
            assert!(s.active().unwrap().doc.shares_state(&changed));
            continue;
        }
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
}

fn color_patch(words: [u32; 4]) -> Value {
    json!({"filterId":"100","arguments":[{"name":"value","value":{"Color":{"components":words.map(|v|json!({"Float32Bits":v})),"encoding":"nonlinear_profile","profile":null}}}]})
}
#[test]
fn exact_literal_color_has_depth_seven_only_for_its_closed_command_entry() {
    let p = color_patch([(-0.0f32).to_bits(), 0.5f32.to_bits(), 0.75f32.to_bits(), 1f32.to_bits()]);
    assert_eq!(journal_depth("xcf.filter.set", &p), 7);
    assert_eq!(journal_depth("layer.setProps", &p), 4);
    let f = wire("xcf.filter.set", &p).unwrap();
    assert!(matches!(f.arguments[0].unwrap().value,PointArgumentValue::Color4F32(v) if v[0]==(-0.0f32).to_bits()));
    let mut c = super::super::Census::default();
    c.command_value("xcf.filter.set", &p, &ctl()).unwrap();
    let mut c = super::super::Census::default();
    c.retained_command_value("xcf.filter.set", &p, &ctl()).unwrap();
    assert!(super::super::Census::default().command_value("layer.setProps", &p, &ctl()).is_err());
    for mode in 0..6 {
        let mut bad = p.clone();
        let color = &mut bad["arguments"][0]["value"]["Color"];
        match mode {
            0 => color["components"][0] = json!({"Float32Bits":f32::NAN.to_bits()}),
            1 => color["components"][0] = json!({"Float32Bits":1.5f32.to_bits()}),
            2 => color["components"][0] = json!({"Float64Bits":"0x0000000000000000"}),
            3 => color["encoding"] = json!("linear"),
            4 => color["profile"] = json!(1),
            _ => color["components"] = json!([]),
        }
        assert!(wire("xcf.filter.set", &bad).is_none());
        assert_eq!(journal_depth("xcf.filter.set", &bad), 4);
    }
}
#[test]
fn registered_literal_color_patch_retains_typed_storage_one_history_and_noop_redo() {
    let (mut s, _q) = session_member(false, 0, true);
    let old = s.active().unwrap().doc.clone();
    let words = [(-0.0f32).to_bits(), 0.375f32.to_bits(), 0.875f32.to_bits(), 0.625f32.to_bits()];
    assert_eq!(s.execute("xcf.filter.set", color_patch(words)).unwrap()["updated"], true);
    let next = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    let node = &next.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let FilterValue::Color { components, encoding, profile } = &node.arguments[0].value else { panic!() };
    assert_eq!(*encoding, SampleEncoding::NonlinearProfile);
    assert_eq!(*profile, None);
    assert_eq!(components, &words.map(ExactNumber::Float32Bits));
    assert_eq!(next.layers[0].content, old.layers[0].content);
    assert_eq!(next.source_color_context, old.source_color_context);
    assert_eq!(
        next.xcf_state.as_ref().unwrap().filter_masks[0].surface.default_bytes(),
        old.xcf_state.as_ref().unwrap().filter_masks[0].surface.default_bytes()
    );
    assert!(s.undo_with_control(&ctl()).unwrap());
    let history = s.active().unwrap().history.clone();
    let original = [0.25f32, 0.5, 0.75, 0.5].map(f32::to_bits);
    assert_eq!(s.execute("xcf.filter.set", color_patch(original)).unwrap()["updated"], false);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert!(s.redo_with_control(&ctl()).unwrap());
    let before = s.active().unwrap().doc.clone();
    let mut malformed = color_patch(words);
    malformed["arguments"][0]["value"]["Color"]["profile"] = json!(5);
    assert!(s.execute("xcf.filter.set", malformed).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
