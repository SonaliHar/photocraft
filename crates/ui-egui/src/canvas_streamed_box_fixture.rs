//! Synthetic CURRENT native storage with real prospective model/profile/store
//! fees. No cloned model or GIMP geometry/source claim. Reference/native archive
//! Vecs are external harness captures, separate from production reader custody.
use photocraft_doc::{
    BackedLayer, ColorMode, Document, FilterMask, ImportCompositing, Layer, LayerContent, PixelFormat, Rect, SampleEncoding, SampleType, Size,
    SourceColorContext, WorkingProfileInterpretation, XcfState,
    accounted_document::AccountedDocument,
    import_compositing::*,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::*,
    xcf_state::*,
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use std::sync::Arc;
pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    bytes: Vec<u8>,
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _payload: QuotaLease,
    _header: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        let start = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Length)?;
        let input = self.bytes.get(start..end).ok_or(BackingError::Bounds)?;
        for (src, dst) in input.chunks(4096).zip(out.chunks_mut(4096)) {
            c.check(src.len() as u64)?;
            dst.copy_from_slice(src);
        }
        Ok(())
    }
}
struct Profiles {
    current: Arc<Vec<u8>>,
    original: Option<Arc<Vec<u8>>>,
    _fee: QuotaLease,
}
pub(super) struct Literal {
    pub bounds: Rect,
    pub raw: Vec<u8>,
    pub sample: SampleType,
}
pub(super) fn fixture(sample: SampleType, assigned: bool, variant: usize) -> (AccountedDocument, Literal) {
    build(sample, assigned, variant, false)
}
pub(super) fn malformed_fixture(sample: SampleType, assigned: bool) -> AccountedDocument {
    build(sample, assigned, 1, true).0
}
fn build(sample: SampleType, assigned: bool, variant: usize, malformed: bool) -> (AccountedDocument, Literal) {
    assert!(!assigned || sample == SampleType::F32);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let model = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    // The small fixed original is read only for its exact profile definitions;
    // its transient decoded surfaces are covered before import and then drop.
    let profile_fee = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let profiles = if assigned {
        use sha2::{Digest, Sha256};
        let path =
            std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("box-raster-color-evidence/assigned-unit-f32.source.xcf");
        let bytes = std::fs::read(path).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(hash, "e4f31ba998692bb7f57ebc82433a4c8d90a7aeaeb2d67a9276079bb594d4cec8");
        let old = photocraft_io::import("profile-only.xcf", &bytes).unwrap().document;
        Profiles {
            current: old.icc_profile.as_ref().unwrap().clone(),
            original: old.source_color_context.as_ref().unwrap().original_icc.clone(),
            _fee: profile_fee,
        }
    } else {
        Profiles { current: Arc::new(photocraft_cms::Builtin::Srgb.profile().to_bytes()), original: None, _fee: profile_fee }
    };
    let canvas = if variant == 0 { Size::new(263, 19) } else { Size::new(17, 13) };
    let bounds = if variant == 0 { Rect::new(-7, -3, 271, 23) } else { Rect::new(253, -2, 266, 9) };
    let mut doc = Document::new("Synthetic streamed Box CURRENT", canvas, ColorMode::Rgb, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(profiles.current.clone());
    doc.working_profile_interpretation =
        if assigned { WorkingProfileInterpretation::Icc } else { WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap()) };
    doc.source_color_context =
        Some(SourceColorContext { source_precision: if sample == SampleType::F32 { 650 } else { 750 }, original_icc: profiles.original.clone() });
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let bpp = format.bytes_per_pixel();
    let len = bounds.width() as usize * bounds.height() as usize * bpp;
    let mut payload = q.reserve(QuotaKind::Resident, (len * 2) as u64).unwrap();
    let header = q.reserve(QuotaKind::Index, (std::mem::size_of::<Store>() + 128) as u64).unwrap();
    ctl().check(len as u64).unwrap();
    let mut raw = Vec::new();
    raw.try_reserve_exact(len).unwrap();
    assert!(raw.capacity() <= len * 2);
    raw.resize(len, 0);
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let at = ((y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize) * bpp;
            let n = (x * 3 + y * 5).rem_euclid(17);
            let p = [
                if n == 0 { -0.0 } else { 0.125 + f64::from(n) / 32.0 },
                if n == 1 { -0.03125 } else { 0.375 },
                if n == 2 { 1.125 } else { 0.625 },
                if malformed && x == bounds.x0 && y == bounds.y0 {
                    2.0
                } else if n == 3 {
                    0.0
                } else {
                    0.75
                },
            ];
            for (c, v) in p.into_iter().enumerate() {
                match sample {
                    SampleType::F32 => raw[at + c * 4..at + c * 4 + 4].copy_from_slice(&(v as f32).to_ne_bytes()),
                    SampleType::F64 => raw[at + c * 8..at + c * 8 + 8].copy_from_slice(&v.to_ne_bytes()),
                    _ => panic!("float fixture"),
                }
            }
        }
    }
    payload.reconcile_capacity(raw.capacity() as u64).unwrap();
    let literal = Literal { bounds, raw: raw.clone(), sample }; // external reference capture
    let default = vec![0; bpp];
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let store = Arc::new(Store { bytes: raw, descriptor, quota: q.clone(), _payload: payload, _header: header });
    let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
    let mut layer = Layer::new("CURRENT native drawable", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    layer.visible = !malformed;
    layer.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    let mask = MaskId(101);
    let filter = FilterId(100);
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(layer.id),
        revision: 0,
        nodes: vec![FilterNode {
            id: filter,
            operation: OperationIdentity { name: "gegl:box-blur".into(), stored_version: "0:0".into(), schema_epoch: "xcf22-rgb-box-replace-v1".into() },
            arguments: vec![FilterArgument { name: "radius".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: 4 }) }],
            source_display_name: None,
            source_icon: None,
            enabled: !malformed,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: Some(GimpComposition {
                compatibility_epoch: "xcf22-rgb-box-replace-v1".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1474592175,
                stored_composite_space: -1474592175,
                stored_operator: -1474592175,
                resolved: None,
                pass_through: false,
            }),
        }],
    });
    state.items.push(ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: layer.visible,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(bounds),
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
        name: "CURRENT empty effect mask".into(),
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
            outside_f64_bits: 0.0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
    });
    state.filter_masks.push(FilterMask {
        id: mask,
        owner: filter,
        bounds: doc.bounds(),
        surface: Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false)),
    });
    state.selection.selected = vec![ItemRef::Layer(layer.id)];
    state.selection.active = Some(ItemRef::Layer(layer.id));
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    let census = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(census.retained_bytes() as u64 <= model.bytes());
    let guard = Arc::new((profiles, model));
    let owner = if assigned {
        AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q, &ctl())
    } else {
        AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl())
    }
    .unwrap();
    (owner, literal)
}
// Cursor bytes are external harness capture; the reader below supplies actual
// immutable provider/payload fees on the owner's ledger before typed adoption.
pub(super) fn encoded(owner: &AccountedDocument) -> Vec<u8> {
    photocraft_format::owned_document::save_current_document_archive(
        owner.document(),
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        owner.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner()
}
pub(super) fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let raw = encoded(owner);
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(raw.as_slice()),
        raw.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &ctl(),
    )
    .unwrap()
}
pub(super) fn native(doc: &Document) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("native") };
    let r = layer.operative_bounds();
    let bpp = layer.format().bytes_per_pixel();
    let mut raw = vec![0; r.width() as usize * r.height() as usize * bpp];
    for coord in r.tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        let p = coord.rect().intersect(&r);
        for y in p.y0..p.y1 {
            for x in p.x0..p.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * bpp;
                pin.read_pixel_native_into(x, y, &mut raw[at..at + bpp], &ctl()).unwrap();
            }
        }
    }
    raw
}
pub(super) fn sample(literal: &Literal, x: i32, y: i32) -> [f64; 4] {
    let bpp = if literal.sample == SampleType::F32 { 16 } else { 32 };
    let at = ((y - literal.bounds.y0) as usize * literal.bounds.width() as usize + (x - literal.bounds.x0) as usize) * bpp;
    std::array::from_fn(|c| {
        if literal.sample == SampleType::F32 {
            f64::from(f32::from_ne_bytes(literal.raw[at + c * 4..at + c * 4 + 4].try_into().unwrap()))
        } else {
            f64::from_ne_bytes(literal.raw[at + c * 8..at + c * 8 + 8].try_into().unwrap())
        }
    })
}
pub(super) fn exact(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.xcf_state, b.xcf_state);
    assert_eq!(native(a), native(b));
    let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native") };
    assert_eq!(a.descriptor(), b.descriptor());
    assert_eq!(a.default_bytes(), b.default_bytes());
}
