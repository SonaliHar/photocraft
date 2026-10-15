//! UNRUN structural custody only. Synthetic assigned ICC bytes are deliberately
//! not a CMS/source/render witness. Fixture reservations precede allocations;
//! expected model clones below are explicitly external test-oracle buffers.
use super::*;
use crate::import_compositing::{GimpComposition, ResolvedGimpComposition};
use crate::item_identity::MaskId;
use crate::xcf_state::{ItemState, MaskState};
use crate::{BackedLayer, Group, Layer, LayerMask, Size, SourceColorContext, XcfState};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
fn leaf(name: &str, r: Rect, f: PixelFormat, q: &BackingQuota) -> Layer {
    let d = NativePlaneDescriptor::new(f, r, &vec![0; f.bytes_per_pixel()]).unwrap();
    let b = BackedRaster::new(Arc::new(Store(d)), q.clone(), 1, &ctl()).unwrap();
    Layer::new(name, LayerContent::BackedRaster(BackedLayer::new(b, d, &ctl()).unwrap()))
}
fn item(l: &Layer, r: Rect, parent: Option<ItemRef>) -> ItemState {
    ItemState {
        item: ItemRef::Layer(l.id),
        name: l.name.clone(),
        custom_name: false,
        visible: l.visible,
        locks: Default::default(),
        parent,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(r),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: EPOCH.into(),
            source_mode: 28,
            legacy: false,
            stored_blend_space: 0,
            stored_composite_space: 1,
            stored_operator: 1,
            resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
            pass_through: false,
        }),
    }
}
fn fixture(mode: ColorMode, sample: SampleType, masked: bool, visible: bool, enabled: bool) -> AccountedDocument {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 });
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let payload = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut d = Document::new("group.pcraft", Size::new(17, 13), mode, sample);
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let icc = Arc::new(vec![0x91; 132]);
    d.icc_profile = Some(icc.clone());
    d.source_color_context = Some(SourceColorContext {
        source_precision: match sample {
            SampleType::U16 => 250,
            SampleType::F32 => 650,
            _ => 750,
        },
        original_icc: Some(icc),
    });
    let f = PixelFormat::new(mode, sample, true);
    let base = leaf("base", d.bounds(), f, &q);
    let lo = Rect::new(-3, -2, 8, 7);
    let hi = Rect::new(4, 3, 13, 10);
    let extent = lo.union(&hi);
    let lower = leaf("lower", lo, f, &q);
    let mut upper = leaf("upper", hi, f, &q);
    upper.opacity = 0.3125;
    let mut top = Layer::new("isolated", LayerContent::Group(Group { children: vec![lower, upper], expanded: true, artboard: None }));
    top.opacity = 0.625;
    top.visible = visible;
    let LayerContent::Group(g) = &top.content else { panic!("fixture group") };
    let mut s = XcfState::default();
    s.provenance.format = "xcf".into();
    s.provenance.format_version = if masked { 13 } else { 12 };
    // Intentionally not tree order: relationships come from exact IDs/parents.
    s.items.push(item(&g.children[1], hi, Some(ItemRef::Layer(top.id))));
    s.items.push(item(&base, d.bounds(), None));
    s.items.push(item(&g.children[0], lo, Some(ItemRef::Layer(top.id))));
    s.items.push(item(&top, extent, None));
    if masked {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        surface.try_write_pixel_f64(-3, -2, &[0.25]).unwrap();
        top.mask = Some(LayerMask { surface: Arc::new(surface), enabled, linked: true, density: 1., feather: 0. });
        s.items.push(ItemState {
            item: ItemRef::Mask(MaskId(901)),
            name: "group mask".into(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(extent),
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(top.id),
                bounds: extent,
                offset: [extent.x0, extent.y0],
                outside_f64_bits: 0f64.to_bits(),
                enabled,
                shown: false,
                editing: false,
                linked: true,
            }),
        });
    }
    s.selection.active = Some(ItemRef::Layer(top.id));
    s.selection.selected.push(ItemRef::Layer(top.id));
    d.layers = vec![base, top];
    d.xcf_state = Some(s);
    AccountedDocument::from_owned(d, Arc::new((index, payload)), q, &ctl()).unwrap()
}
#[test]
fn title_copy_preserves_three_native_leaves_tree_mask_profiles_and_original_retention() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for (masked, enabled) in [(false, true), (true, true), (true, false)] {
                for visible in [true, false] {
                    let source = fixture(mode, sample, masked, visible, enabled);
                    source.validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
                    let copy = source.try_with_document_name("continued.pcraft", &ctl()).unwrap();
                    copy.validate_assigned_isolated_normal_group_storage(&ctl()).unwrap();
                    let mut expected = source.document().clone();
                    expected.name = "continued.pcraft".into();
                    assert_eq!(copy.document(), &expected);
                    assert!(Arc::ptr_eq(&source.owner.retention, &copy.owner.retention));
                    assert!(source.quota().shares_ledger(copy.quota()));
                    assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), copy.document().icc_profile.as_ref().unwrap()));
                    assert!(Arc::ptr_eq(
                        source.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
                        copy.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
                    ));
                    if masked {
                        assert!(Arc::ptr_eq(
                            &source.document().layers[1].mask.as_ref().unwrap().surface,
                            &copy.document().layers[1].mask.as_ref().unwrap().surface
                        ));
                    }
                    let LayerContent::Group(g) = &copy.document().layers[1].content else { panic!("group must not flatten") };
                    assert_eq!(g.children.len(), 2);
                    assert!(g.children.iter().all(|l| matches!(l.content, LayerContent::BackedRaster(_))));
                }
            }
        }
    }
}
#[test]
fn shape_parent_profile_epoch_and_disabled_mask_contradictions_refuse_without_copy() {
    for fault in 0..7 {
        let source = fixture(ColorMode::Rgb, SampleType::F32, true, false, false);
        let q = source.quota().clone();
        let i = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let p = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
        let mut d = source.document().clone();
        match fault {
            0 => {
                let LayerContent::Group(g) = &mut d.layers[1].content else { panic!() };
                g.children.push(g.children[0].clone());
            }
            1 => d.xcf_state.as_mut().unwrap().items[0].parent = None,
            2 => d.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().compatibility_epoch = "other epoch".into(),
            3 => d.icc_profile = Some(Arc::new(vec![0x92; 132])),
            4 => d.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|v| v.mask.as_mut()).unwrap().offset[0] += 1,
            5 => d.layers[1].mask.as_mut().unwrap().enabled = true,
            _ => d.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().resolved.as_mut().unwrap().operator = GimpOperator::Intersection,
        }
        let bad = AccountedDocument::from_owned(d, Arc::new((source, i, p)), q.clone(), &ctl()).unwrap();
        let before = q.usage().unwrap();
        assert!(bad.validate_assigned_isolated_normal_group_storage(&ctl()).is_err());
        assert!(bad.try_with_document_name("refused", &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn generic_edits_and_selection_are_not_granted_by_group_storage() {
    for masked in [false, true] {
        let source = fixture(ColorMode::Grayscale, SampleType::F64, masked, true, false);
        let id = source.document().layers[1].id;
        let before = source.quota().usage().unwrap();
        for e in [
            AccountedEdit::LayerMetadata { layer: id, name: Some("no generic edit"), visible: Some(false), opacity: Some(0.2) },
            AccountedEdit::LayerName { layer: id, name: "closed" },
            AccountedEdit::MaskEnabled { layer: id, enabled: true },
            AccountedEdit::NativePixels { layer: source.document().layers[0].id, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 16] },
        ] {
            assert!(source.try_edit(e, &ctl()).is_err());
        }
        assert!(source.try_replace_selection(None, &ctl()).is_err());
        assert_eq!(source.quota().usage().unwrap(), before);
    }
}
#[test]
fn title_full_quota_noop_late_cancel_and_repeated_copies_keep_flat_guard_and_refund() {
    let source = fixture(ColorMode::Rgb, SampleType::U16, true, false, false);
    let q = source.quota();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    assert!(source.try_with_document_name(&source.document().name, &ctl()).unwrap().shares_model(&source));
    assert!(source.try_with_document_name("changed", &ctl()).is_err());
    drop(blocker);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.try_with_document_name("changed", &denied).is_err());
    let stop = || q.usage().unwrap().index > before.index;
    let token = Interrupt::cancel_only(&stop);
    assert!(source.try_with_document_name("changed", &ReadControl { interrupt: &token, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let a = source.try_with_document_name("a", &ctl()).unwrap();
    let b = a.try_with_document_name("b", &ctl()).unwrap();
    assert!(Arc::ptr_eq(&source.owner.retention, &b.owner.retention));
    drop(a);
    drop(b);
    assert_eq!(q.usage().unwrap(), before);
}
