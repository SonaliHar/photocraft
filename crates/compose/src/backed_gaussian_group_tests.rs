//! UNRUN generated CURRENT small-Auto group. This is an independent old-resident
//! kernel oracle over genuinely paid native inputs, not original GIMP admission.
use super::*;
use photocraft_doc::import_compositing::{GimpComposition, ResolvedGimpComposition};
use photocraft_doc::item_identity::{FilterId, ItemRef, MaskId};
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity};
use photocraft_doc::xcf_state::{ItemState, MaskState};
use photocraft_doc::{
    ColorMode, Document, Group, ImportCompositing, Layer, LayerContent, LayerId, LayerMask, PixelFormat, SampleType, Size, SourceColorContext,
    WorkingProfileInterpretation, XcfState,
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::Arc;
const FILTER_EPOCH: &str = "xcf22-isolated-small-auto-v1";
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Guard {
    _model: QuotaLease,
    _payload: QuotaLease,
    reads: std::sync::atomic::AtomicU64,
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    _guard: Arc<Guard>,
    q: BackingQuota,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(output.len() as u64)?;
        self._guard.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        for (i, byte) in output.iter_mut().enumerate() {
            let at = offset + i as u64;
            let pixel = at / 16;
            let channel = (at / 4 % 4) as usize;
            let tuple = [0.125f32 + (pixel % 5) as f32 * 0.0625, 0.5, 0.75, 0.5 + (pixel % 3) as f32 * 0.125];
            *byte = tuple[channel].to_ne_bytes()[(at % 4) as usize];
        }
        Ok(())
    }
}
fn leaf(name: &str, bounds: Rect, q: &BackingQuota, guard: &Arc<Guard>) -> Layer {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut default = [0; 16];
    default[..4].copy_from_slice(&(-0f32).to_ne_bytes());
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, _guard: guard.clone(), q: q.clone() }), q.clone(), 1, &ctl()).unwrap();
    let mut layer = Layer::new(name, LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    layer.import_compositing =
        Some(photocraft_doc::ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    layer
}
fn row(layer: &Layer, extent: Rect, parent: Option<ItemRef>, group: bool) -> ItemState {
    let multiply = layer.blend == photocraft_doc::BlendMode::Multiply;
    ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(extent),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: if group {
                photocraft_doc::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH
            } else if multiply {
                "xcf-22-authored-blend-auto"
            } else {
                "xcf-22-normal"
            }
            .into(),
            source_mode: if multiply { 30 } else { 28 },
            legacy: false,
            pass_through: false,
            stored_blend_space: if multiply { -1 } else { 0 },
            stored_composite_space: -1,
            stored_operator: if multiply { -2 } else { -1 },
            resolved: Some(ResolvedGimpComposition {
                blend_space: GimpSpace::LinearRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: if multiply { GimpOperator::ClipToBackdrop } else { GimpOperator::Union },
            }),
        }),
    }
}
fn mask_row(id: MaskId, owner: LayerId, extent: Rect, physical: bool) -> ItemState {
    ItemState {
        item: ItemRef::Mask(id),
        name: if physical { "physical group mask" } else { "saved selection" }.into(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(extent),
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(owner),
            bounds: extent,
            offset: if physical { [extent.x0, extent.y0] } else { [0, 0] },
            outside_f64_bits: 0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: physical,
            linked: physical,
        }),
    }
}
fn fixture(q: &BackingQuota, edited: bool, enabled: bool) -> (Document, Arc<Guard>) {
    let guard = Arc::new(Guard {
        _model: q.reserve(QuotaKind::Index, 8 << 20).unwrap(),
        _payload: q.reserve(QuotaKind::Resident, 8 << 20).unwrap(),
        reads: std::sync::atomic::AtomicU64::new(0),
    });
    let mut doc = Document::new("original.pcraft", Size::new(16, 12), ColorMode::Rgb, SampleType::F32);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
    let base = leaf("base", doc.bounds(), q, &guard);
    let normal_bounds = Rect::from_xywh(0, 0, 11, 8);
    let multiply_bounds = if edited { Rect::from_xywh(-1, 2, 10, 9) } else { Rect::from_xywh(2, -1, 10, 9) };
    let extent = normal_bounds.union(&multiply_bounds);
    let mut multiply = leaf("multiply", multiply_bounds, q, &guard);
    multiply.blend = photocraft_doc::BlendMode::Multiply;
    multiply.opacity = if edited { 0.4375 } else { 0.7125 };
    let normal = leaf("normal", normal_bounds, q, &guard);
    let mut group = Layer::new("isolated", LayerContent::Group(Group { children: vec![multiply, normal], expanded: true, artboard: None }));
    group.import_compositing =
        Some(photocraft_doc::ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let mut surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
    surface.try_write_pixel_f64(extent.x0, extent.y0, &[0.25]).unwrap();
    group.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
    let LayerContent::Group(tree) = &group.content else { panic!("fixture") };
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    // Intentionally non-tree row order: associations must follow actual IDs.
    state.items = vec![
        row(&tree.children[1], normal_bounds, Some(ItemRef::Layer(group.id)), false),
        mask_row(MaskId(301), group.id, doc.bounds(), false),
        row(&base, doc.bounds(), None, false),
        row(&group, extent, None, true),
        mask_row(MaskId(300), group.id, extent, true),
        row(&tree.children[0], multiply_bounds, Some(ItemRef::Layer(group.id)), false),
    ];
    let arguments = vec![
        FilterArgument { name: "std-dev-y".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1.3f32.to_bits())) },
        FilterArgument { name: "clip-extent".into(), value: FilterValue::Bool(true) },
        FilterArgument { name: "filter".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 } },
        FilterArgument { name: "std-dev-x".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits())) },
        FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 } },
    ];
    let raw = if enabled { -643996976 } else { -1474592175 };
    let node = FilterNode {
        id: FilterId(400),
        operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: FILTER_EPOCH.into() },
        source_display_name: Some("original filter".into()),
        source_icon: None,
        arguments,
        enabled,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(u32::from(edited)),
        mask: Some(ItemRef::Mask(MaskId(301))),
        auxiliary: Vec::new(),
        composition: Some(GimpComposition {
            compatibility_epoch: FILTER_EPOCH.into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: raw,
            stored_composite_space: raw,
            stored_operator: raw,
            resolved: None,
            pass_through: false,
        }),
    };
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(group.id), revision: 17, nodes: vec![node] });
    let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    // An allocated raw -0 word remains distinct from an omitted +0 tile. The
    // storage gate must not normalize it or claim this is an empty-mask proof.
    mask.try_write_pixel_f64(1, 1, &[-0.0]).unwrap();
    state.filter_masks.push(photocraft_doc::FilterMask { id: MaskId(301), owner: FilterId(400), bounds: doc.bounds(), surface: mask });
    state.selection.active = Some(ItemRef::Layer(group.id));
    state.selection.selected.push(ItemRef::Layer(group.id));
    doc.xcf_state = Some(state);
    doc.layers = vec![base, group];
    (doc, guard)
}

// External oracle clone and resident buffers do not assert a production receipt.
fn resident(owner: &AccountedDocument) -> Document {
    let mut doc = owner.document().clone();
    fn convert(layer: &mut Layer) {
        match &mut layer.content {
            LayerContent::BackedRaster(backed) => {
                let view = backed.checked_view();
                let rect = view.logical_bounds();
                let mut raw = vec![0; rect.width() as usize * rect.height() as usize * 16];
                let mut index = 0;
                for y in rect.y0..rect.y1 {
                    for x in rect.x0..rect.x1 {
                        view.read_pixel_native_into(x, y, &mut raw[index..index + 16], &ctl()).unwrap();
                        index += 16;
                    }
                }
                layer.content = LayerContent::Raster(Surface::try_from_interleaved(view.format(), rect, &raw).unwrap());
            }
            LayerContent::Group(g) => {
                for child in &mut g.children {
                    convert(child)
                }
            }
            _ => panic!("fixture"),
        }
    }
    for layer in &mut doc.layers {
        convert(layer)
    }
    doc
}
fn owner(edited: bool, enabled: bool) -> AccountedDocument {
    let q = quota();
    let (doc, guard) = fixture(&q, edited, enabled);
    AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q, &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: exact two extents/enabled+disabled CURRENT group matches independent existing resident renderer and preserves raw storage"]
fn matches_existing_resident_group_order_and_filter() {
    for edited in [false, true] {
        for enabled in [false, true] {
            let owner = owner(edited, enabled);
            let reference = resident(&owner);
            let before = owner.document();
            let expected = crate::gimp_render::PreparedRenderer::new_with_interrupt(&reference, &Interrupt::NONE)
                .unwrap()
                .render(reference.bounds(), &mut |_| Ok(()))
                .unwrap();
            let context = PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &ctl()).unwrap();
            context.validate_operative(&ctl()).unwrap();
            let band = context.render_band(context.bounds(), &ctl()).unwrap();
            assert_eq!(band.pixels().len(), 192);
            for (a, b) in band.pixels().iter().zip(&expected.px) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
            assert!(std::ptr::eq(before, owner.document()));
            let doc = crate::backed_document::PreparedBackedDocument::new_accounted(&owner, Builtin::Srgb.profile(), Default::default(), &ctl()).unwrap();
            let rendered = doc.render_band(owner.document().bounds(), &ctl()).unwrap();
            assert_eq!(rendered.pixels(), band.pixels());
        }
    }
}
#[test]
#[ignore = "UNRUN: retained derived group/output fees and real caller work/cancellation refund on warm source caches"]
fn retained_output_and_actual_control() {
    let owner = owner(true, true);
    drop(PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &ctl()).unwrap());
    let q = owner.quota();
    let before = q.usage().unwrap();
    {
        let context = PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &ctl()).unwrap();
        let held = q.usage().unwrap();
        assert!(held.resident > before.resident);
        let band = context.render_band(Rect::new(3, 2, 8, 6), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        drop(band);
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), before);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n > 1024 { Err(BackingError::Work) } else { Ok(()) } };
    assert!(matches!(PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), before);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(PreparedBackedGaussianGroupContext::new(&owner, Default::default(), &control), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
#[ignore = "UNRUN: distinct class/profile/current gate refuses assigned/basis mismatch, mode61 and malformed disabled content"]
fn exact_class_and_profile_refusals() {
    let good = owner(false, false);
    assert!(crate::backed_document::PreparedBackedDocument::new_accounted(&good, Builtin::SGray.profile(), Default::default(), &ctl()).is_err());
    for defect in 0..3 {
        let q = quota();
        let (mut doc, guard) = fixture(&q, false, false);
        match defect {
            0 => doc.working_profile_interpretation = WorkingProfileInterpretation::Icc,
            1 => {
                doc.xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|i| matches!(i.item,ItemRef::Layer(id) if id==doc.layers[1].id))
                    .unwrap()
                    .composition
                    .as_mut()
                    .unwrap()
                    .source_mode = 61
            }
            _ => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Bool(false),
        }
        assert!(AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q, &ctl()).is_err());
    }
    let q = quota();
    let (mut doc, guard) = fixture(&q, false, false);
    let layer = doc.layers.first_mut().unwrap();
    let LayerContent::BackedRaster(backed) = &layer.content else { panic!("fixture") };
    let changed = backed
        .try_edited_native(Rect::new(15, 11, 16, 12), &[f32::NAN.to_ne_bytes(), 0f32.to_ne_bytes(), 0f32.to_ne_bytes(), 1f32.to_ne_bytes()].concat(), &ctl())
        .unwrap();
    layer.content = LayerContent::BackedRaster(changed);
    let invalid = AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q, &ctl()).unwrap();
    assert!(PreparedBackedGaussianGroupContext::new(&invalid, Default::default(), &ctl()).is_err());
}

#[test]
#[ignore = "UNRUN: source stage preparation owns same-ledger profiles/working metadata and genuine source token without native reads or output evaluation"]
fn source_preparation_only_retains_actual_stage_owner() {
    struct Budget {
        q: BackingQuota,
        work: u64,
    }
    impl crate::gimp_render::PreparationBudget for Budget {
        type Lease = QuotaLease;
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<QuotaLease, CompositionError> {
            self.q.reserve(QuotaKind::Resident, bytes).map_err(|_| CompositionError::Invalid("fixture quota"))
        }
        fn consume_preparation_work(&mut self, units: u64) -> Result<(), CompositionError> {
            self.work = self.work.checked_add(units).ok_or(CompositionError::Invalid("fixture work"))?;
            Ok(())
        }
    }
    for edited in [false, true] {
        for enabled in [false, true] {
            let q = quota();
            let (doc, guard) = fixture(&q, edited, enabled);
            let owner = AccountedDocument::from_owned_readonly_gaussian_group(doc, guard.clone(), q.clone(), &ctl()).unwrap();
            let before = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: 0 };
            let prepared = LeasedGaussianGroupPipeline::new_budgeted(owner.document(), &Interrupt::NONE, &mut budget, owner.clone()).unwrap();
            assert!(budget.work > 0);
            assert_eq!(guard.reads.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert!(q.usage().unwrap().resident > before.resident);
            drop(prepared);
            assert_eq!(q.usage().unwrap(), before);
            assert_eq!(guard.reads.load(std::sync::atomic::Ordering::Relaxed), 0);
        }
    }
}
