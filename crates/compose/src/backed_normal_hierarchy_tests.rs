//! Deferred synthetic actual-owner tree contracts; no producer projection oracle.
//! Native Source import/format fixtures belong to IO/hosts. Reference buffers
//! and expected clones are external harness allocations, not quota receipts.
use super::*;
use crate::backed_stock_point_context::tests::{ctl, fixture as base};
use photocraft_doc::{
    BackedLayer, Layer, Size,
    import_compositing::{GimpComposition, ImportCompositing, ResolvedGimpComposition},
    item_identity::{FilterId, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
    xcf_state::ItemState,
};
use photocraft_raster::{
    Surface,
    backing::{BackedRaster, NativePlaneStore},
};
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    q: BackingQuota,
    fail: Arc<AtomicBool>,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
struct Fixture {
    doc: Document,
    template: ItemState,
    // Diagnostic handles are external harness. Each actual native Store owns
    // its independent real fee; model/template payload drops before retention.
    rasters: Vec<BackedRaster>,
    fails: Vec<Arc<AtomicBool>>,
    retention: Arc<dyn Any + Send + Sync>,
    q: BackingQuota,
}
fn input(which: usize, x: u32, y: u32, sample: SampleType) -> [f64; 4] {
    let p = [
        0.125 + (which as f64) / 16. + f64::from((x + y) % 3) / 32.,
        0.5 + 2f64.powi(-45),
        0.875 - (which as f64) / 16.,
        match (x + y) % 5 {
            0 => -0.,
            1 => 0.,
            2 => 0.25,
            3 => 0.75,
            _ => f64::from_bits(1f64.to_bits() - 1),
        },
    ];
    if sample == SampleType::F32 { p.map(|v| f64::from(v as f32)) } else { p }
}
fn physical() -> ImportCompositing {
    ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false }
}
fn composition(version: u32) -> GimpComposition {
    GimpComposition {
        compatibility_epoch: format!("xcf-{version}-normal"),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: -1,
        stored_operator: -1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    }
}
fn number(n: &str, v: f32) -> FilterArgument {
    FilterArgument { name: n.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn configure(n: &mut photocraft_doc::xcf_filters::FilterNode, kind: u8) {
    let (name, version, epoch, args) = match kind {
        0 => ("gegl:invert-gamma", "0:0", crate::INVERT_GAMMA_MIXED_EPOCH, vec![]),
        1 => ("gegl:exposure", "1:0", crate::EXPOSURE_MIXED_EPOCH, vec![number("exposure", 0.5), number("black-level", 0.0625)]),
        _ => (
            "gimp:semi-flatten",
            "0:0",
            crate::gimp_semi_flatten::EPOCH,
            vec![FilterArgument {
                name: "color".into(),
                value: FilterValue::Color {
                    components: [0.25f32, 0.5, 0.75, 0.375].map(|v| ExactNumber::Float32Bits(v.to_bits())).into(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            }],
        ),
    };
    n.operation = OperationIdentity { name: name.into(), stored_version: version.into(), schema_epoch: epoch.into() };
    n.arguments = args;
    n.opacity_f64_bits = 1f64.to_bits();
    n.enabled = true;
    n.source_clipping = Some(0);
    n.source_region = Some(0);
    n.auxiliary.clear();
    let c = n.composition.as_mut().unwrap();
    c.compatibility_epoch = epoch.into();
    c.source_mode = 62;
    c.stored_blend_space = 1654948433;
    c.stored_composite_space = -17;
    c.stored_operator = 31;
    c.resolved = None;
    c.legacy = false;
    c.pass_through = false;
}
fn group(name: &str, children: Vec<Layer>, opacity: f32) -> Layer {
    let mut l = Layer::group(name, children);
    l.opacity = opacity;
    l.import_compositing = Some(physical());
    l
}
fn rows(layers: &[Layer], parent: Option<ItemRef>, template: &ItemState, version: u32, bounds: Rect, out: &mut Vec<ItemState>) {
    for layer in layers {
        let mut row = template.clone();
        row.item = ItemRef::Layer(layer.id);
        row.name = layer.name.clone();
        row.visible = layer.visible;
        row.parent = parent;
        row.raster_bounds = if layer.is_group() { None } else { Some(bounds) };
        row.mask = None;
        row.source_flags = 0;
        row.composition = Some(composition(version));
        out.push(row);
        if let LayerContent::Group(g) = &layer.content {
            rows(&g.children, Some(ItemRef::Layer(layer.id)), template, version, bounds, out);
        }
    }
}
fn reindex(f: &mut Fixture) {
    let version = f.doc.xcf_state.as_ref().unwrap().provenance.format_version;
    let bounds = f.doc.bounds();
    let mut layer_rows = Vec::new();
    rows(&f.doc.layers, None, &f.template, version, bounds, &mut layer_rows);
    let state = f.doc.xcf_state.as_mut().unwrap();
    state.items.retain(|r| matches!(r.item, ItemRef::Mask(_)));
    layer_rows.append(&mut state.items);
    state.items = layer_rows;
    let first = f.doc.layers[0].id;
    state.selection = Default::default();
    state.selection.selected = vec![ItemRef::Layer(first)];
    state.selection.active = Some(ItemRef::Layer(first));
}
fn fixture(sample: SampleType, size: Size, semi: bool) -> Fixture {
    let (mut doc, q, original, _, _, _) = base(0, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    // Prospective closed fixture model/Strings/maps/Group children BEFORE clones.
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let retention: Arc<dyn Any + Send + Sync> = Arc::new((original, index, resident));
    let old = doc.layers[0].clone();
    let state = doc.xcf_state.as_ref().unwrap();
    let template = state.items.iter().find(|r| r.item == ItemRef::Layer(old.id)).unwrap().clone();
    let stack_template = state.filter_stacks[0].clone();
    let mask_template = state.filter_masks[0].clone();
    let mask_row = state.items.iter().find(|r| matches!(r.item, ItemRef::Mask(_))).unwrap().clone();
    doc.size = size;
    let bounds = doc.bounds();
    let format = doc.pixel_format();
    let bpp = format.bytes_per_pixel();
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.clear();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.provenance.format_version = 22;
    state.current_filter_output_precision = FilterOutputPrecision::Canonical;
    let mut leaves = Vec::new();
    let mut rasters = Vec::new();
    let mut fails = Vec::new();
    for which in 0..3 {
        let mut layer = old.clone();
        layer.id = LayerId::fresh();
        layer.name = format!("leaf-{which}");
        layer.opacity = match which {
            0 => 0.5,
            1 => 0.75,
            _ => 1.,
        };
        layer.visible = true;
        layer.import_compositing = Some(physical());
        let len = (size.width as usize).checked_mul(size.height as usize).unwrap().checked_mul(bpp).unwrap();
        let fee = q.reserve(QuotaKind::Resident, (len as u64) * 2 + 512).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).unwrap();
        assert!(bytes.capacity() <= len * 2);
        bytes.resize(len, 0);
        for y in 0..size.height {
            for x in 0..size.width {
                let at = ((y * size.width + x) as usize) * bpp;
                for (ch, v) in input(which, x, y, sample).iter().enumerate() {
                    photocraft_color::try_write_sample_f64(&mut bytes[at..at + bpp], sample, ch, *v).unwrap();
                }
            }
        }
        let descriptor = NativePlaneDescriptor::new(format, bounds, &[0u8; 32][..bpp]).unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let store = Arc::new(Store { descriptor, bytes, q: q.clone(), fail: Arc::clone(&fail), _fee: fee });
        let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
        layer.content = LayerContent::BackedRaster(BackedLayer::new(raster.clone(), descriptor, &ctl()).unwrap());
        if which != 0 {
            let mut stack = stack_template.clone();
            stack.owner = ItemRef::Layer(layer.id);
            let node = &mut stack.nodes[0];
            node.id = FilterId(LayerId::fresh().0);
            let id = MaskId(LayerId::fresh().0);
            node.mask = Some(ItemRef::Mask(id));
            configure(
                node,
                if which == 1 {
                    0
                } else if semi {
                    2
                } else {
                    1
                },
            );
            let mut mask = mask_template.clone();
            mask.id = id;
            mask.owner = node.id;
            mask.bounds = bounds;
            mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            let mut row = mask_row.clone();
            row.item = ItemRef::Mask(id);
            row.name = format!("mask-{which}");
            row.raster_bounds = Some(bounds);
            let m = row.mask.as_mut().unwrap();
            m.owner = ItemRef::Layer(layer.id);
            m.bounds = bounds;
            doc.xcf_state.as_mut().unwrap().filter_stacks.push(stack);
            doc.xcf_state.as_mut().unwrap().filter_masks.push(mask);
            doc.xcf_state.as_mut().unwrap().items.push(row);
        }
        leaves.push(layer);
        rasters.push(raster);
        fails.push(fail);
    }
    let last = leaves.pop().unwrap();
    let middle = leaves.pop().unwrap();
    let bottom = leaves.pop().unwrap();
    let inner = group("inner", vec![group("empty-child", vec![], 0.25), last], 0.5);
    let outer = group("outer", vec![middle, inner], 0.375);
    doc.layers = vec![bottom, outer, group("empty-root", vec![], 0.5)];
    let mut f = Fixture { doc, retention, q, rasters, fails, template };
    reindex(&mut f);
    f
}
fn adopt(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    drop(f.template);
    let owner = AccountedDocument::new_readonly_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).unwrap();
    (owner, f.rasters, f.fails)
}
fn gamma(p: [f64; 4]) -> [f64; 4] {
    [1. - p[0], 1. - p[1], 1. - p[2], p[3]]
}
fn exposure(p: [f64; 4]) -> [f64; 4] {
    let trc = photocraft_cms::srgb_trc();
    let d = (-0.5f64).exp2() - 0.0625;
    [
        trc.eval_inverse64((trc.eval64(p[0]) - 0.0625) / d),
        trc.eval_inverse64((trc.eval64(p[1]) - 0.0625) / d),
        trc.eval_inverse64((trc.eval64(p[2]) - 0.0625) / d),
        p[3],
    ]
}
fn semi(p: [f64; 4]) -> [f64; 4] {
    let trc = photocraft_cms::srgb_trc();
    let input = [trc.eval64(p[0]), trc.eval64(p[1]), trc.eval64(p[2]), p[3]];
    let mut out = [0.; 4];
    photocraft_algo::gimp::point_semi_flatten::SemiFlatten { color: [0.25, 0.5, 0.75, 0.375] }.try_eval_pixel(&input, 3, &mut out).unwrap();
    [trc.eval_inverse64(out[0]), trc.eval_inverse64(out[1]), trc.eval_inverse64(out[2]), out[3]]
}
fn reference(sample: SampleType, x: u32, y: u32, semi_leaf: bool, hidden: bool) -> [f64; 4] {
    let normal =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    let b = normal.normal([0.; 4], input(0, x, y, sample), 0.5, GimpOperator::Union).unwrap();
    let last = if semi_leaf { semi(input(2, x, y, sample)) } else { exposure(input(2, x, y, sample)) };
    let inner = normal.normal([0.; 4], last, 1., GimpOperator::Union).unwrap();
    let first = normal.normal([0.; 4], gamma(input(1, x, y, sample)), 0.75, GimpOperator::Union).unwrap();
    let outer = normal.normal(first, inner, 0.5, GimpOperator::Union).unwrap();
    if hidden { b } else { normal.normal(b, outer, 0.375, GimpOperator::Union).unwrap() }
}
#[test]
#[ignore = "UNRUN actual nested bottomfirst isolated Normal with two old leaf families, alpha-changing Semi, empty groups and full hidden subtree; independent fixed nested reference"]
fn actual_nested_order_and_isolation_are_not_flat_root_folds() {
    for sample in [SampleType::F32, SampleType::F64] {
        for semi_leaf in [false, true] {
            let (owner, _, _) = adopt(fixture(sample, Size::new(8, 6), semi_leaf));
            assert!(owner.is_readonly_normal_hierarchy_storage());
            assert!(!owner.is_readonly_filtered_flat_point_storage());
            assert!(!owner.is_readonly_point_storage());
            let ctx = Context::new(&owner, Default::default(), &ctl()).unwrap();
            ctx.validate_operative(&ctl()).unwrap();
            assert_eq!(ctx.plan.max_depth, 3);
            for x in 0..8 {
                let band = ctx.render_band(Rect::new(x, 0, x + 1, 1), &ctl()).unwrap();
                assert_eq!(band.pixels()[0].map(f64::to_bits), reference(sample, x as u32, 0, semi_leaf, false).map(f64::to_bits));
            }
            let outer =
                crate::backed_document::PreparedBackedDocument::new_accounted(&owner, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl())
                    .unwrap();
            assert_eq!(
                outer.render_band(Rect::new(1, 0, 2, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits),
                reference(sample, 1, 0, semi_leaf, false).map(f64::to_bits)
            );
        }
    }
    let mut f = fixture(SampleType::F64, Size::new(8, 6), true);
    f.doc.layers[1].visible = false;
    reindex(&mut f);
    let (owner, _, _) = adopt(f);
    let ctx = Context::new(&owner, Default::default(), &ctl()).unwrap();
    ctx.validate_operative(&ctl()).unwrap();
    assert_eq!(
        ctx.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits),
        reference(SampleType::F64, 2, 0, true, true).map(f64::to_bits)
    );
}
#[test]
#[ignore = "UNRUN depth32 including empty chains, lower max_pixels1 and true column+row partitions, actualversion12/13 plain hierarchy and no fake22 filtered grant"]
fn generic_depth_and_both_axis_chunks_keep_original_values() {
    let (owner, _, _) = adopt(fixture(SampleType::F64, Size::new(257, 259), false));
    let ctx = Context::new(&owner, NormalBandLimits { max_rows: 2, max_pixels: 14 }, &ctl()).unwrap();
    let r = Rect::new(250, 255, 257, 257);
    let band = ctx.render_band(r, &ctl()).unwrap();
    for y in 255..257 {
        for x in 250..257 {
            let at = ((y - 255) * 7 + x - 250) as usize;
            assert_eq!(band.pixels()[at].map(f64::to_bits), reference(SampleType::F64, x as u32, y as u32, false, false).map(f64::to_bits));
        }
    }
    for depth in [32usize, 33] {
        let mut f = fixture(SampleType::F64, Size::new(8, 6), false);
        let mut leaf = f.doc.layers.remove(0);
        leaf.opacity = 1.;
        let mut chain = leaf;
        for i in 0..depth {
            chain = group(&format!("deep-{i}"), vec![chain], 1.);
        }
        f.doc.layers = vec![chain];
        let s = f.doc.xcf_state.as_mut().unwrap();
        s.filter_stacks.clear();
        s.filter_masks.clear();
        s.items.clear();
        reindex(&mut f);
        let q = f.q.clone();
        if depth == 33 {
            drop(f.template);
            assert!(AccountedDocument::new_readonly_normal_hierarchy_current_storage(f.doc, f.retention, q, &ctl()).is_err());
        } else {
            let (owner, _, _) = adopt(f);
            let ctx = Context::new(&owner, NormalBandLimits { max_rows: 1, max_pixels: 1 }, &ctl()).unwrap();
            assert_eq!(ctx.plan.max_depth, 32);
            ctx.validate_operative(&ctl()).unwrap();
            assert_eq!(
                ctx.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits),
                input(0, 2, 0, SampleType::F64).map(f64::to_bits)
            );
        }
    }
    for version in [12, 13] {
        let mut f = fixture(SampleType::F32, Size::new(8, 6), false);
        let s = f.doc.xcf_state.as_mut().unwrap();
        s.filter_stacks.clear();
        s.filter_masks.clear();
        s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
        s.provenance.format_version = version;
        reindex(&mut f);
        let (owner, _, _) = adopt(f);
        Context::new(&owner, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).unwrap();
        let mut f = fixture(SampleType::F32, Size::new(8, 6), false);
        f.doc.xcf_state.as_mut().unwrap().provenance.format_version = version;
        reindex(&mut f);
        drop(f.template);
        match AccountedDocument::new_readonly_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()) {
            Err(_) => {}
            Ok(owner) => assert!(Context::new(&owner, Default::default(), &ctl()).is_err()),
        }
    }
}
#[test]
#[ignore = "UNRUN canonicalNone group, foreign parents/raw Normal/offset/profile/BYTEzero masks, inactive malformed input/provider proof"]
fn actual_tree_roles_and_inactive_defects_refuse_without_fallback() {
    for fault in 0..9 {
        let mut f = fixture(SampleType::F64, Size::new(8, 6), false);
        let outer_id = f.doc.layers[1].id;
        let canvas = f.doc.bounds();
        match fault {
            0 => f.doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(outer_id)).unwrap().raster_bounds = Some(canvas),
            1 => f.doc.xcf_state.as_mut().unwrap().items[1].parent = Some(ItemRef::Layer(LayerId::fresh())),
            2 => f.doc.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().stored_composite_space = 1,
            3 => f.doc.source_color_context.as_mut().unwrap().original_icc = Some(vec![0; 132]),
            4 => f.doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(255, 255, &[-0.]).unwrap(),
            5 => f.doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "foreign".into(),
            7 => {
                f.doc.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(Rect::new(1, 0, 9, 6));
            }
            8 => {
                f.doc.layers[1].visible = false;
                reindex(&mut f);
                f.fails[2].store(true, Ordering::Relaxed);
            }
            _ => {
                f.doc.layers[1].visible = false;
                reindex(&mut f);
                f.doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(0, 0, &[f64::NAN]).unwrap();
            }
        }
        drop(f.template);
        match AccountedDocument::new_readonly_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()) {
            Err(_) => {}
            Ok(o) => match Context::new(&o, Default::default(), &ctl()) {
                Err(_) => {}
                Ok(ctx) => assert!(ctx.validate_operative(&ctl()).is_err()),
            },
        }
    }
}
#[test]
#[ignore = "UNRUN real tree/profile/Parts/output/frame runtime fees, warm late Work/cancel and actual lastleaf cold Io, outer known-basis atomic refusal"]
fn failed_tree_walks_and_publication_refund_on_same_ledger() {
    let (owner, rasters, fails) = adopt(fixture(SampleType::F64, Size::new(8, 6), true));
    let q = owner.quota().clone();
    let ctx = Context::new(&owner, Default::default(), &ctl()).unwrap();
    ctx.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 2, 1);
    drop(ctx.render_band(r, &ctl()).unwrap());
    let base = q.usage().unwrap();
    let total = AtomicU64::new(0);
    drop(
        ctx.render_band(
            r,
            &ReadControl {
                interrupt: &Interrupt::NONE,
                work: &|n| {
                    total.fetch_add(n, Ordering::Relaxed);
                    Ok(())
                },
            },
        )
        .unwrap(),
    );
    let used = AtomicU64::new(0);
    let c = ReadControl {
        interrupt: &Interrupt::NONE,
        work: &|n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total.load(Ordering::Relaxed) / 2 { Err(BackingError::Work) } else { Ok(()) },
    };
    assert!(ctx.render_band(r, &c).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let stop = || true;
    let i = Interrupt::cancel_only(&stop);
    assert!(ctx.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), base);
    rasters[2].clear_cache().unwrap();
    let base = q.usage().unwrap();
    fails[2].store(true, Ordering::Relaxed);
    assert!(ctx.render_band(r, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), base);
    fails[2].store(false, Ordering::Relaxed);
    drop(ctx);
    let base = q.usage().unwrap();
    for basis in [photocraft_cms::Builtin::SGray.profile(), photocraft_cms::Builtin::LinearSrgb.profile()] {
        assert!(crate::backed_document::PreparedBackedDocument::new_accounted(&owner, basis, Default::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), base);
    }
}

#[path = "backed_normal_hierarchy_mask_tests.rs"]
mod masked;

#[path = "backed_normal_hierarchy_integer_tests.rs"]
mod integer;
