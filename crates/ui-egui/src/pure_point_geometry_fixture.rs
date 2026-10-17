//! Synthetic origin-only pure-point CURRENT storage with prospective model,
//! profile, native provider and physical mask fees. No authored larger source or
//! producer numerical claim. Archive/display/raw captures are external harness.
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
#[derive(Clone, Copy, Debug)]
pub(super) enum Family {
    Semi,
    Hue,
    Vibrance,
}
impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Self::Semi => "gimp:semi-flatten",
            Self::Hue => "gegl:hue-chroma",
            Self::Vibrance => "gegl:vibrance",
        }
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::Semi => "xcf22-gimp-semi-flatten-literal-v1",
            Self::Hue => "xcf22-gegl-hue-chroma-lch-float-v1",
            Self::Vibrance => "xcf22-gegl-vibrance-linear-lch-float-v1",
        }
    }
    pub fn arguments(self) -> Vec<FilterArgument> {
        match self {
            Self::Semi => vec![FilterArgument {
                name: "color".into(),
                value: FilterValue::Color {
                    components: [0.25f32, 0.5, 0.75, 0.375].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            }],
            Self::Hue => ["hue", "chroma", "lightness"]
                .into_iter()
                .zip([12.5f32, -3.25, 1.125])
                .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) })
                .collect(),
            Self::Vibrance => ["vibrance", "saturation"]
                .into_iter()
                .zip([25f32, 1.125])
                .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) })
                .collect(),
        }
    }
    pub fn changed(self, id: &str) -> serde_json::Value {
        let mut arguments = self.arguments();
        match self {
            Self::Semi => {
                let FilterValue::Color { components, .. } = &mut arguments[0].value else { panic!("color") };
                components[0] = ExactNumber::Float32Bits(0.625f32.to_bits());
            }
            Self::Hue | Self::Vibrance => arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())),
        }
        serde_json::json!({"filterId":id,"arguments":arguments,"enabled":false})
    }
}
pub(super) fn fixture(family: Family, sample: SampleType, variant: usize) -> AccountedDocument {
    build(family, sample, variant, false, false)
}
pub(super) fn offset_fixture(family: Family, sample: SampleType) -> AccountedDocument {
    build(family, sample, 0, true, false)
}
pub(super) fn malformed_fixture(family: Family, sample: SampleType) -> AccountedDocument {
    build(family, sample, 0, false, true)
}
fn build(family: Family, sample: SampleType, variant: usize, offset: bool, malformed: bool) -> AccountedDocument {
    assert!(matches!(sample, SampleType::F32 | SampleType::F64));
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let model = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    // Prospectively hold profile construction/storage before encoding builtin bytes.
    let profile_fee = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let profiles = Profiles { current: photocraft_cms::Builtin::Srgb.profile().to_bytes(), original: None, _fee: profile_fee };
    let canvas = if variant == 0 { Size::new(257, 259) } else { Size::new(1, 257) };
    let bounds = Rect::from_xywh(if offset { 1 } else { 0 }, 0, canvas.width, canvas.height);
    let mut doc = Document::new("Synthetic origin pure-point CURRENT", canvas, ColorMode::Rgb, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(profiles.current.clone());
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
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
                if n == 1 { 0.0 } else { 0.375 },
                if n == 2 { 1.0 } else { 0.625 },
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
            operation: OperationIdentity { name: family.name().into(), stored_version: "0:0".into(), schema_epoch: family.epoch().into() },
            arguments: family.arguments(),
            source_display_name: None,
            source_icon: None,
            enabled: !malformed,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: Some(GimpComposition {
                compatibility_epoch: family.epoch().into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: 0x2af58251,
                stored_composite_space: 0x2af58251,
                stored_operator: 0x2af58251,
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
    let mask_fee = q.reserve(QuotaKind::Resident, 4 << 20).unwrap();
    let mut mask_surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
    // Two real physical zero tiles cross the boundary; padding remains stored.
    mask_surface.try_write_pixel_f64(0, 0, &[-0.0]).unwrap();
    mask_surface.try_write_pixel_f64(canvas.width as i32 - 1, canvas.height as i32 - 1, &[-0.0]).unwrap();
    state.filter_masks.push(FilterMask { id: mask, owner: filter, bounds: doc.bounds(), surface: mask_surface });
    state.selection.selected = vec![ItemRef::Layer(layer.id)];
    state.selection.active = Some(ItemRef::Layer(layer.id));
    doc.layers.push(layer);
    doc.xcf_state = Some(state);
    let census = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(census.retained_bytes() as u64 <= model.bytes());
    let guard = Arc::new((profiles, model, mask_fee));
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
pub(super) fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub(super) fn encoded(owner: &AccountedDocument) -> Vec<u8> {
    native::encoded(owner)
}
pub(super) fn raw(doc: &Document) -> Vec<u8> {
    native::native(doc)
}
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod native;
pub(super) fn storage(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(raw(a), raw(b));
    let (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native") };
    assert_eq!(x.default_bytes(), y.default_bytes());
    assert_eq!(x.operative_bounds(), y.operative_bounds());
    assert_eq!(x.format(), y.format());
    let (x, y) = (a.xcf_state.as_ref().unwrap(), b.xcf_state.as_ref().unwrap());
    assert_eq!(x.opaque, y.opaque);
    assert_eq!(x.provenance, y.provenance);
    assert_eq!(x.filter_masks, y.filter_masks);
    assert_eq!(x.filter_stacks[0].owner, y.filter_stacks[0].owner);
    assert_eq!(x.filter_stacks[0].nodes[0].composition, y.filter_stacks[0].nodes[0].composition);
}
