//! UNRUN explicit native current Threshold Alpha reopen/publication drafts.
use super::*;
use photocraft_cms::{Builtin, Profile};
use photocraft_doc::{
    BackedLayer, ColorMode, Document, FilterMask, ImportCompositing, Layer, LayerContent, PixelFormat, SampleEncoding, SampleType, Size, SourceColorContext,
    Surface, WorkingProfileInterpretation, XcfState,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{load_document_from_owned_archive, save_current_document_archive},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
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
type NativeFixture = (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, &'static Profile, Vec<u8>);
fn fixture(mode: ColorMode, sample: SampleType, stored: SampleEncoding, invalid_native: bool) -> NativeFixture {
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
    if invalid_native {
        raw.get_mut(..8).unwrap().copy_from_slice(&f64::NAN.to_ne_bytes());
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

#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn packet(bad: u8) -> (Arc<Archive>, BackingQuota) {
    let (mut doc, q, guard, _, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, bad == 4);
    match bad {
        1 => doc.layers[0].fill_opacity = 0.5,
        2 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.5]).unwrap(),
        3 => doc.working_profile_interpretation = WorkingProfileInterpretation::Icc,
        _ => {}
    }
    let bytes = save_current_document_archive(&doc, std::io::Cursor::new(Vec::new()), Default::default(), &q, &ctl()).unwrap().into_inner();
    let fee = q.reserve(QuotaKind::Resident, bytes.capacity() as u64 + 128).unwrap();
    drop(doc);
    drop(guard);
    (Arc::new(Archive { bytes, quota: q.clone(), _lease: fee }), q)
}
fn load(source: Arc<Archive>, q: &BackingQuota) -> OwnedDocumentArchive {
    load_document_from_owned_archive(source, &LoadOptions::default(), Default::default(), q.clone(), &ctl()).unwrap()
}
#[test]
fn explicit_reopen_validates_current_recipe_and_keeps_native_identity_title_class() {
    let (source, q) = packet(0);
    let weak = Arc::downgrade(&source);
    let archive = load(source.clone(), &q);
    let mut session = Session::new();
    session.add_owned_native_threshold_alpha_archive(archive, "current.pcraft", Some("authorized/current.pcraft"), &ctl()).unwrap();
    let state = session.active().unwrap();
    let owner = state.doc.accounted().unwrap();
    owner.validate_readonly_threshold_alpha_storage(&ctl()).unwrap();
    assert!(owner.quota().shares_ledger(&q));
    assert_eq!(state.path.as_deref(), Some("authorized/current.pcraft"));
    assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
    assert!(!state.source_read_only);
    assert!(!state.history.can_undo());
    let renamed = owner.try_with_document_name("saved-as.pcraft", &ctl()).unwrap();
    renamed.validate_readonly_threshold_alpha_storage(&ctl()).unwrap();
    assert_eq!(renamed.document().xcf_state, owner.document().xcf_state);
    assert_eq!(renamed.document().source_color_context, owner.document().source_color_context);
    drop(renamed);
    drop(source);
    assert!(weak.upgrade().is_some());
    drop(session);
    assert!(weak.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn stored_filter_shape_never_bypasses_current_profile_fill_opacity_or_empty_mask_qualification() {
    for bad in [1, 2, 3, 4] {
        let (source, q) = packet(bad);
        let baseline = q.usage().unwrap();
        let mut session = Session::new();
        assert!(session.add_owned_native_threshold_alpha_archive(load(source.clone(), &q), "current.pcraft", None, &ctl()).is_err());
        assert!(session.docs.is_empty());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[test]
fn disabled_native_reopen_save_as_and_canvas_preserve_fractional_f64_alpha_and_raw_words() {
    fn native_words(owner: &photocraft_doc::accounted_document::AccountedDocument) -> Vec<u8> {
        let LayerContent::BackedRaster(layer) = &owner.document().layers[0].content else { panic!() };
        let mut output = vec![0; 48 * 32];
        for y in 0..6 {
            for x in 0..8 {
                let at = (y * 8 + x) * 32;
                layer.checked_view().read_logical_pixel_native_into(x as i32, y as i32, &mut output[at..at + 32], &ctl()).unwrap();
            }
        }
        output
    }
    fn display(owner: &photocraft_doc::accounted_document::AccountedDocument) -> Vec<[u64; 4]> {
        let prepared =
            photocraft_compose::backed_document::PreparedBackedDocument::new_accounted(owner, Builtin::Srgb.profile(), Default::default(), &ctl()).unwrap();
        prepared.render_band(owner.document().bounds(), &ctl()).unwrap().pixels().iter().map(|p| p.map(f64::to_bits)).collect()
    }
    let (mut doc, q, guard, _, _, raw) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, false);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let bytes = save_current_document_archive(&doc, std::io::Cursor::new(Vec::new()), Default::default(), &q, &ctl()).unwrap().into_inner();
    let fee = q.reserve(QuotaKind::Resident, bytes.capacity() as u64 + 128).unwrap();
    let source = Arc::new(Archive { bytes, quota: q.clone(), _lease: fee });
    drop(doc);
    drop(guard);
    let mut session = Session::new();
    session.add_owned_native_threshold_alpha_archive(load(source, &q), "disabled.pcraft", Some("authorized/disabled.pcraft"), &ctl()).unwrap();
    let owner = session.active().unwrap().doc.accounted().unwrap();
    assert!(owner.is_readonly_threshold_alpha_storage());
    assert!(!owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    assert_eq!(native_words(owner), raw);
    let expected = raw
        .as_chunks::<32>()
        .0
        .iter()
        .map(|p| std::array::from_fn(|c| f64::from_ne_bytes(p[c * 8..(c + 1) * 8].try_into().unwrap()).to_bits()))
        .collect::<Vec<[u64; 4]>>();
    let output = display(owner);
    assert_eq!(output, expected, "disabled threshold is raw identity before existing Normal/transparent-backdrop composition");
    for (i, pixel) in output.iter().enumerate() {
        assert_eq!(f64::from_bits(pixel[3]), if i % 2 == 0 { 0.25 } else { 0.75 });
    }
    let source_identity = session.active().unwrap().source.clone();
    let prepared = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&session, "disabled-copy.pcraft", &ctl()).unwrap();
    let saved = prepared
        .write_and_publish(&mut session, &ctl(), |snapshot, _| {
            save_current_document_archive(snapshot, std::io::Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                .map(|writer| writer.into_inner())
                .map_err(|e| EngineError::Other(e.to_string()))
        })
        .unwrap();
    assert_eq!(session.active().unwrap().source, source_identity);
    assert_eq!(session.active().unwrap().path.as_deref(), Some("disabled-copy.pcraft"));
    assert!(session.active().unwrap().doc.accounted().unwrap().is_readonly_threshold_alpha_storage());
    assert!(!session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    let fee = q.reserve(QuotaKind::Resident, saved.capacity() as u64 + 128).unwrap();
    let source = Arc::new(Archive { bytes: saved, quota: q.clone(), _lease: fee });
    let mut reopened = Session::new();
    reopened.add_owned_native_threshold_alpha_archive(load(source, &q), "disabled-copy.pcraft", Some("authorized/disabled-copy.pcraft"), &ctl()).unwrap();
    let state = reopened.active().unwrap();
    assert!(state.doc.quota().unwrap().shares_ledger(&q));
    assert!(!state.source_read_only);
    let owner = state.doc.accounted().unwrap();
    assert!(!owner.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    assert_eq!(native_words(owner), raw);
    assert_eq!(display(owner), expected);
}
#[test]
fn duplicate_cancel_and_quota_failures_never_publish_partial_tab_or_release_other_owner() {
    let (source, q) = packet(0);
    let mut session = Session::new();
    session.add_owned_native_threshold_alpha_archive(load(source.clone(), &q), "first.pcraft", None, &ctl()).unwrap();
    let original = session.active().unwrap().doc.clone();
    assert!(session.add_owned_native_threshold_alpha_archive(load(source.clone(), &q), "duplicate.pcraft", None, &ctl()).is_err());
    assert_eq!(session.docs.len(), 1);
    assert!(session.active().unwrap().doc.shares_state(&original));
    for limit in [0, 1, 32, 512, 4096] {
        let archive = load(source.clone(), &q);
        let mut target = Session::new();
        let work = AtomicU64::new(0);
        let charge = |n| {
            if work.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
        assert!(target.add_owned_native_threshold_alpha_archive(archive, "cancel.pcraft", None, &c).is_err());
        assert!(target.docs.is_empty());
    }
    let archive = load(source.clone(), &q);
    let used = q.usage().unwrap().resident;
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - used).unwrap();
    let mut target = Session::new();
    assert!(target.add_owned_native_threshold_alpha_archive(archive, "quota.pcraft", None, &ctl()).is_err());
    assert!(target.docs.is_empty());
    drop(blocker);
    assert!(session.active().unwrap().doc.shares_state(&original));
}
