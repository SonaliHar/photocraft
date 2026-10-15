//! UNRUN real native writer/reader/typed Headless lifecycle drafts.
//! Synthetic current recipes supply no original-XCF correspondence.
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
use std::path::PathBuf;
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
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-native-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn write_current(path: &Path, threshold: bool, enabled: bool) -> Vec<u8> {
    let (mut document, q, guard, _, _, raw) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    if threshold {
        document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
    } else {
        document.xcf_state = None;
        document.source_color_context = None;
        document.working_profile_interpretation = WorkingProfileInterpretation::Icc;
        document.layers[0].import_compositing = None;
    }
    let owner = if threshold {
        AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q, &ctl()).unwrap()
    } else {
        AccountedDocument::from_owned(document, guard, q, &ctl()).unwrap()
    };
    let snapshot = photocraft_engine::DocumentSnapshot::from_accounted(owner).unwrap();
    crate::files::save_accounted_native_selected_path(&snapshot, path.to_str().unwrap(), &Default::default(), &ctl()).unwrap();
    raw
}
#[test]
fn trusted_and_rooted_owned_reopen_preserve_title_current_profile_and_exact_native_words() {
    for trusted in [true, false] {
        let dir = Directory::new();
        let path = dir.0.join("actual-input.pcraft");
        let raw = write_current(&path, false, true);
        let mut h = if trusted { Headless::trusted_local() } else { Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), None).unwrap()) };
        let requested = if trusted { path.as_path() } else { Path::new("actual-input.pcraft") };
        let reply = h.open_owned_native_keep_current(requested, &ctl()).unwrap();
        assert_eq!(reply.value()["name"], "Persisted document title");
        assert_eq!(reply.value()["nativeProfilePolicy"], "keepCurrent");
        assert_eq!(reply.value()["index"], 0);
        assert!(reply.warnings().is_empty());
        let state = h.session.active().unwrap();
        assert!(state.doc.accounted().is_some());
        assert!(!state.source_read_only);
        assert_eq!(state.path.as_deref(), requested.to_str());
        assert_eq!(state.source.as_ref().unwrap().label, "actual-input.pcraft");
        assert_eq!(state.doc.name, "Persisted document title");
        assert_eq!(state.doc.icc_profile.as_ref().unwrap(), &Builtin::Srgb.profile().to_bytes());
        let LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
        let mut pixel = [0; 32];
        layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &ctl()).unwrap();
        assert_eq!(&pixel, raw.get(..32).unwrap());
        let q = state.doc.quota().unwrap().clone();
        let before = q.usage().unwrap();
        drop(reply);
        assert!(q.usage().unwrap().index < before.index);
    }
}
#[test]
fn native_threshold_enabled_and_disabled_choose_strict_current_handoff_not_resident_fallback() {
    for enabled in [true, false] {
        let dir = Directory::new();
        let path = dir.0.join("current.pcraft");
        let raw = write_current(&path, true, enabled);
        let mut h = Headless::trusted_local();
        let reply = h.open_owned_native_keep_current(&path, &ctl()).unwrap();
        let state = h.session.active().unwrap();
        assert!(state.doc.accounted().unwrap().is_readonly_threshold_alpha_storage());
        assert_eq!(state.doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled, enabled);
        let LayerContent::BackedRaster(layer) = &state.doc.layers[0].content else { panic!() };
        let mut word = [0; 32];
        layer.checked_view().read_logical_pixel_native_into(1, 0, &mut word, &ctl()).unwrap();
        assert_eq!(&word, raw.get(32..64).unwrap());
        drop(reply);
        let mut ordinary = Headless::trusted_local();
        assert!(ordinary.open(&path).is_err());
        assert!(ordinary.session.documents().is_empty());
    }
}
#[test]
fn existing_catalog_is_reused_and_duplicate_identity_refuses_without_focus_or_fee_changes() {
    let dir = Directory::new();
    let first = dir.0.join("first.pcraft");
    let second = dir.0.join("second.pcraft");
    write_current(&first, false, true);
    write_current(&second, false, true);
    let mut h = Headless::trusted_local();
    drop(h.open_owned_native_keep_current(&first, &ctl()).unwrap());
    let q = h.session.accounted_open_quota().unwrap().clone();
    drop(h.open_owned_native_keep_current(&second, &ctl()).unwrap());
    assert!(h.session.active().unwrap().doc.quota().unwrap().shares_ledger(&q));
    let before = h.session.active().unwrap().doc.clone();
    let usage = q.usage().unwrap();
    let active = h.session.active_index();
    assert!(h.open_owned_native_keep_current(&first, &ctl()).is_err());
    assert_eq!(h.session.documents().len(), 2);
    assert_eq!(h.session.active_index(), active);
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), usage);
}
#[test]
fn denied_scope_bad_policy_shape_work_and_cancel_leave_prior_state_unchanged() {
    let dir = Directory::new();
    let native = dir.0.join("source.pcraft");
    write_current(&native, false, true);
    let mut denied = Headless::new();
    assert!(denied.open_owned_native_keep_current(&native, &ctl()).is_err());
    assert!(denied.session.documents().is_empty());
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), None).unwrap());
    drop(h.open_owned_native_keep_current(Path::new("source.pcraft"), &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let q = before.quota().unwrap().clone();
    let usage = q.usage().unwrap();
    for path in ["../outside.pcraft", "source.xcf", "bad\0.pcraft"] {
        assert!(h.open_owned_native_keep_current(Path::new(path), &ctl()).is_err());
    }
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let cancel = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(h.open_owned_native_keep_current(Path::new("source.pcraft"), &cancel).is_err());
    let spent = AtomicU64::new(0);
    let fail = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(h.open_owned_native_keep_current(Path::new("source.pcraft"), &ReadControl { interrupt: &Interrupt::NONE, work: &fail }).is_err());
    assert!(spent.load(Ordering::Relaxed) > 0);
    assert_eq!(h.session.documents().len(), 1);
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), usage);
}
#[test]
fn complete_paid_rpc_envelope_precedes_adoption_and_hostile_id_refuses_before_read() {
    let dir = Directory::new();
    let path = dir.0.join("native.pcraft");
    write_current(&path, false, true);
    let mut h = Headless::trusted_local();
    let reply = h.open_owned_native_rpc_bounded(&path, &serde_json::json!({"request":["exact",17]})).unwrap();
    assert_eq!(reply.value()["id"], serde_json::json!({"request":["exact",17]}));
    assert_eq!(reply.value()["ok"], true);
    assert_eq!(reply.value()["result"]["nativeProfilePolicy"], "keepCurrent");
    drop(reply);
    let mut id = Value::Null;
    for _ in 0..10 {
        id = Value::Array(vec![id]);
    }
    let error = h.open_owned_native_rpc_bounded(&dir.0.join("absent.pcraft"), &id).err().unwrap();
    assert!(error.to_string().contains("RPC"));
    assert_eq!(h.session.documents().len(), 1);
}
