//! Ignored UNRUN actual paid multi-root contracts. Generated native stores and
//! independent public Parts/math references are not producer projection oracles.
use super::*;
use crate::{
    backed_filtered_flat_point_document::PreparedBackedFilteredFlatPointDocument,
    backed_stock_point_context::tests::{ctl, fixture as base},
    gimp_composite::WorkingSpace,
};
use photocraft_algo::gimp::hue_chroma::{self, Parameters};
use photocraft_doc::{
    BackedLayer, Size,
    item_identity::{FilterId, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
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
    fail: Arc<AtomicBool>,
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
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        let a = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn input(root: usize, x: u32, y: u32, sample: SampleType) -> [f64; 4] {
    let p = [
        0.125 + f64::from((x + y * 3) % 5) / 16. + root as f64 / 32.,
        0.5 + 2f64.powi(-45),
        0.875 - root as f64 / 32.,
        match (x + y + root as u32) % 5 {
            0 => -0.,
            1 => 0.,
            2 => 0.125,
            3 => 0.75,
            _ => f64::from_bits(1f64.to_bits() - 1),
        },
    ];
    if sample == SampleType::F32 { p.map(|v| f64::from(v as f32)) } else { p }
}
fn number(name: &str, value: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) }
}
fn configure(node: &mut photocraft_doc::xcf_filters::FilterNode, kind: u8) {
    let (name, epoch, args) = match kind {
        0 => (
            "gimp:semi-flatten",
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
        1 => ("gegl:hue-chroma", crate::gimp_hue_chroma::EPOCH, vec![number("hue", 31.25), number("chroma", 22.5), number("lightness", -12.5)]),
        _ => ("gegl:vibrance", crate::gimp_vibrance::EPOCH, vec![number("vibrance", 25.), number("saturation", 0.75)]),
    };
    node.operation = OperationIdentity { name: name.into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    node.arguments = args;
    node.enabled = true;
    node.opacity_f64_bits = 1f64.to_bits();
    node.source_clipping = Some(0);
    node.source_region = Some(0);
    node.auxiliary.clear();
    let p = node.composition.as_mut().unwrap();
    p.compatibility_epoch = epoch.into();
    p.source_mode = 62;
    p.stored_blend_space = 1654948433;
    p.stored_composite_space = -17;
    p.stored_operator = 31;
    p.resolved = None;
    p.legacy = false;
    p.pass_through = false;
}
struct Fixture {
    document: Document,
    retention: Arc<dyn Any + Send + Sync>,
    quota: BackingQuota,
    rasters: Vec<BackedRaster>,
    fails: Vec<Arc<AtomicBool>>,
    reads: Arc<AtomicU64>,
}
fn fixture(sample: SampleType, size: Size, kinds: &[Option<u8>], poison: Option<usize>) -> Fixture {
    assert!((2..=4).contains(&kinds.len()));
    let (mut d, q, original, _, _, _) = base(0, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    // Fixed closed fixture model/map/Arc capacity is prospectively paid before
    // cloning templates. Native byte Vecs have separate actual2x fees below.
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let retention: Arc<dyn Any + Send + Sync> = Arc::new((original, index, resident));
    d.size = size;
    let bounds = d.bounds();
    let format = d.pixel_format();
    let bpp = format.bytes_per_pixel();
    let layer_template = d.layers[0].clone();
    let state = d.xcf_state.as_ref().unwrap();
    let layer_item = state.items.iter().find(|v| v.item == ItemRef::Layer(layer_template.id)).unwrap().clone();
    let stack_template = state.filter_stacks[0].clone();
    let mask_template = state.filter_masks[0].clone();
    let mask_item = state.items.iter().find(|v| matches!(v.item, ItemRef::Mask(_))).unwrap().clone();
    d.layers.clear();
    let state = d.xcf_state.as_mut().unwrap();
    state.items.clear();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.selection = Default::default();
    let mut rasters = Vec::new();
    let mut fails = Vec::new();
    let reads = Arc::new(AtomicU64::new(0));
    for (root, kind) in kinds.iter().enumerate() {
        let mut layer = layer_template.clone();
        layer.id = LayerId::fresh();
        layer.name = format!("root{root}");
        layer.visible = true;
        layer.opacity = if root == 1 {
            0.375
        } else if root == 2 {
            0.5
        } else {
            1.
        };
        let len = (size.area() as usize).checked_mul(bpp).unwrap();
        let fee = q.reserve(QuotaKind::Resident, (len.checked_mul(2).unwrap() + 1024) as u64).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).unwrap();
        assert!(bytes.capacity() <= 2 * len);
        bytes.resize(len, 0);
        for y in 0..size.height {
            for x in 0..size.width {
                let start = (y as usize * size.width as usize + x as usize) * bpp;
                let mut p = input(root, x, y, sample);
                if poison == Some(root) && x == size.width - 1 && y == size.height - 1 {
                    p[0] = f64::NAN;
                }
                for (c, v) in p.into_iter().enumerate() {
                    photocraft_color::try_write_sample_f64(&mut bytes[start..start + bpp], sample, c, v).unwrap();
                }
            }
        }
        let descriptor = NativePlaneDescriptor::new(format, bounds, &[0u8; 32][..bpp]).unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let store = Arc::new(Store { descriptor, bytes, fail: fail.clone(), reads: reads.clone(), quota: q.clone(), _fee: fee });
        let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
        layer.content = LayerContent::BackedRaster(BackedLayer::new(raster.clone(), descriptor, &ctl()).unwrap());
        let mut item = layer_item.clone();
        item.item = ItemRef::Layer(layer.id);
        item.name = layer.name.clone();
        item.visible = layer.visible;
        item.raster_bounds = Some(bounds);
        let c = item.composition.as_mut().unwrap();
        c.compatibility_epoch = "xcf-22-normal".into();
        let state = d.xcf_state.as_mut().unwrap();
        state.items.push(item);
        if let Some(kind) = kind {
            let mut stack = stack_template.clone();
            stack.owner = ItemRef::Layer(layer.id);
            let node = &mut stack.nodes[0];
            node.id = FilterId(LayerId::fresh().0);
            let mask_id = MaskId(LayerId::fresh().0);
            node.mask = Some(ItemRef::Mask(mask_id));
            configure(node, *kind);
            let mut mask = mask_template.clone();
            mask.id = mask_id;
            mask.owner = node.id;
            mask.bounds = bounds;
            mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            let mut row = mask_item.clone();
            row.item = ItemRef::Mask(mask_id);
            row.name = format!("mask{root}");
            row.raster_bounds = Some(bounds);
            let m = row.mask.as_mut().unwrap();
            m.owner = ItemRef::Layer(layer.id);
            m.bounds = bounds;
            state.items.push(row);
            state.filter_masks.push(mask);
            state.filter_stacks.push(stack);
        }
        d.layers.push(layer);
        rasters.push(raster);
        fails.push(fail);
    }
    let first = d.layers[0].id;
    let state = d.xcf_state.as_mut().unwrap();
    state.selection.selected = vec![ItemRef::Layer(first)];
    state.selection.active = Some(ItemRef::Layer(first));
    assert!(d.layers.capacity() <= 16);
    assert!(state.items.capacity() <= 32);
    assert!(state.filter_stacks.capacity() <= 16);
    assert!(state.filter_masks.capacity() <= 16);
    for l in &d.layers {
        assert!(l.name.capacity() <= 4096)
    }
    Fixture { document: d, retention, quota: q, rasters, fails, reads }
}
fn adopt(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>, Arc<AtomicU64>) {
    let Fixture { document, retention, quota, rasters, fails, reads } = f;
    let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(document, retention, quota, &ctl()).unwrap();
    (owner, rasters, fails, reads)
}
// Public CMS Parts and reference buffers are external harness, not q receipts.
fn physical(parts: &photocraft_cms::owned_lab::Parts, p: [f64; 4], h: f64, c: f64, l: f64) -> [f64; 4] {
    let mut lab = [0.; 3];
    parts.physical_forward.try_eval_f64(&p[..3], &mut lab).unwrap();
    let chroma = lab[1].hypot(lab[2]);
    let hue = if chroma == 0. { 0. } else { lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.) };
    let changed =
        hue_chroma::evaluate_lch_float([lab[0] as f32, chroma as f32, hue as f32, p[3] as f32], Parameters { hue: h, chroma: c, lightness: l }).unwrap();
    let angle = f64::from(changed[2]).to_radians();
    let lab = [f64::from(changed[0]), f64::from(changed[1]) * angle.cos(), f64::from(changed[1]) * angle.sin()];
    let mut rgb = [0.; 3];
    parts.physical_reverse.try_eval_f64(&lab, &mut rgb).unwrap();
    [f64::from(rgb[0] as f32), f64::from(rgb[1] as f32), f64::from(rgb[2] as f32), f64::from(changed[3])]
}
fn leaf(kind: Option<u8>, p: [f64; 4], nl: &photocraft_cms::owned_lab::Parts, linear: &photocraft_cms::owned_lab::Parts) -> [f64; 4] {
    let curve = photocraft_cms::curve::srgb_trc();
    match kind {
        None => p,
        Some(0) => {
            let work = [curve.eval64(p[0]) as f32, curve.eval64(p[1]) as f32, curve.eval64(p[2]) as f32, p[3] as f32].map(f64::from);
            let mut out = [0.; 4];
            photocraft_algo::gimp::point_semi_flatten::SemiFlatten { color: [0.25, 0.5, 0.75, 0.375] }.try_eval_pixel(&work, 3, &mut out).unwrap();
            [curve.eval_inverse64(out[0]), curve.eval_inverse64(out[1]), curve.eval_inverse64(out[2]), out[3]]
        }
        Some(1) => physical(nl, p, 31.25, 22.5, -12.5),
        _ => {
            let staged = [curve.eval64(p[0]) as f32, curve.eval64(p[1]) as f32, curve.eval64(p[2]) as f32, p[3] as f32];
            let saturated = photocraft_algo::gimp::point_vibrance::saturation_linear_float(staged, 0.75).unwrap();
            let out = physical(linear, saturated.map(f64::from), 0., 25., 0.);
            [
                f64::from(curve.eval_inverse64(out[0]) as f32),
                f64::from(curve.eval_inverse64(out[1]) as f32),
                f64::from(curve.eval_inverse64(out[2]) as f32),
                out[3],
            ]
        }
    }
}
#[test]
#[ignore = "UNRUN mixed4native roots independent declaredleafmath+Normal; noGIMP projectionoracle"]
fn mixed_roots_use_raw_wide_leaf_then_bottom_to_top_normal() {
    let nl = photocraft_cms::owned_lab::construct_with_control(
        photocraft_cms::Builtin::Srgb.profile(),
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        Default::default(),
        &|| false,
    )
    .unwrap();
    let linear = photocraft_cms::owned_linear_srgb_lab::construct_with_control(Default::default(), &|| false).unwrap();
    // Independent two-root order control: Document index0 is the bottom.
    // Disable its Hue stage so the two explicit colored native operands are
    // composed directly; no reference traversal can mirror the renderer order.
    let mut ordered = fixture(SampleType::F64, Size::new(8, 6), &[Some(1), None], None);
    ordered.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let (ordered, _, _, _) = adopt(ordered);
    let ordered_context = PreparedBackedFilteredFlatPointDocument::new(&ordered, Default::default(), &ctl()).unwrap();
    let bottom = [0.25, 0.5 + 2f64.powi(-45), 0.875, 0.125];
    let top = [0.28125, 0.5 + 2f64.powi(-45), 0.84375, 0.75];
    assert_eq!(input(0, 2, 0, SampleType::F64), bottom);
    assert_eq!(input(1, 2, 0, SampleType::F64), top);
    let order_space =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    let bottom_over_zero = order_space.normal([0.; 4], bottom, 1., GimpOperator::Union).unwrap();
    let top_over_bottom = order_space.normal(bottom_over_zero, top, 0.375, GimpOperator::Union).unwrap();
    let top_over_zero = order_space.normal([0.; 4], top, 0.375, GimpOperator::Union).unwrap();
    let reversed = order_space.normal(top_over_zero, bottom, 1., GimpOperator::Union).unwrap();
    assert_ne!(top_over_bottom.map(f64::to_bits), reversed.map(f64::to_bits));
    let actual = ordered_context.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap();
    assert_eq!(actual.pixels()[0].map(f64::to_bits), top_over_bottom.map(f64::to_bits));

    let kinds = [Some(0), Some(1), Some(2), None];
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for size in [Size::new(8, 6), Size::new(257, 3)] {
            let (owner, _, _, _) = adopt(fixture(sample, size, &kinds, None));
            let context = PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).unwrap();
            context.validate_operative(&ctl()).unwrap();
            for (x, y) in [(0, 0), (size.width - 1, size.height - 1)] {
                let r = Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1);
                let band = context.render_band(r, &ctl()).unwrap();
                let mut expected = [0.; 4];
                for root in 0..4 {
                    let p = leaf(kinds[root], input(root, x, y, sample), &nl, &linear);
                    expected = working.normal(expected, p, f64::from(owner.document().layers[root].opacity), GimpOperator::Union).unwrap();
                }
                assert_eq!(band.pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
}
#[test]
#[ignore = "UNRUN hidden/opacityzero/disabled fullnative/mask/profile/epoch proof andglobalidentity"]
fn inactive_and_foreign_relations_do_not_waive_proof() {
    for activity in 0..3 {
        let mut f = fixture(SampleType::F64, Size::new(257, 3), &[Some(1), None], Some(0));
        if activity == 0 {
            f.document.layers[0].visible = false;
            let id = f.document.layers[0].id;
            f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
        } else if activity == 1 {
            f.document.layers[0].opacity = 0.;
        } else {
            f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        }
        let (owner, _, _, _) = adopt(f);
        let p = PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).unwrap();
        assert!(p.validate_operative(&ctl()).is_err());
    }
    for fault in 0..4 {
        let mut f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), Some(2)], None);
        match fault {
            0 => {
                let id = f.document.layers[0].id;
                f.document
                    .xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|i| i.item == ItemRef::Layer(id))
                    .unwrap()
                    .composition
                    .as_mut()
                    .unwrap()
                    .compatibility_epoch = "foreign".into();
            }
            1 => {
                let duplicate = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
                f.document.xcf_state.as_mut().unwrap().filter_stacks[1].nodes[0].id = duplicate;
            }
            2 => {
                f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
            }
            _ => {
                f.document.xcf_state.as_mut().unwrap().filter_masks[0]
                    .surface
                    .try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes())
                    .unwrap();
            }
        }
        if let Ok(owner) = AccountedDocument::new_readonly_filtered_flat_point_storage(f.document, f.retention, f.quota, &ctl()) {
            assert!(PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN outergenericdispatcher basis/header integration and actualsameq threeband failurecleanup"]
fn outer_dispatcher_and_later_root_failure_publish_nothing() {
    let (owner, rasters, fails, reads) = adopt(fixture(SampleType::F64, Size::new(257, 3), &[Some(0), Some(1), Some(2), None], None));
    let q = owner.quota().clone();
    for basis in [photocraft_cms::Builtin::SGray.profile(), photocraft_cms::Builtin::LinearSrgb.profile()] {
        let before = q.usage().unwrap();
        let n = reads.load(Ordering::Relaxed);
        assert!(crate::backed_document::PreparedBackedDocument::new_accounted(&owner, basis, Default::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(reads.load(Ordering::Relaxed), n);
    }
    let outer =
        crate::backed_document::PreparedBackedDocument::new_accounted(&owner, photocraft_cms::Builtin::Srgb.profile(), Default::default(), &ctl()).unwrap();
    assert_eq!(outer.render_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap().rect(), Rect::new(0, 0, 1, 1));
    drop(outer);
    let prepared = PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).unwrap();
    prepared.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 1, 1);
    drop(prepared.render_band(r, &ctl()).unwrap());
    rasters[3].clear_cache().unwrap();
    let baseline = q.usage().unwrap();
    fails[3].store(true, Ordering::Relaxed);
    assert!(prepared.render_band(r, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    fails[3].store(false, Ordering::Relaxed);
    drop(prepared.render_band(r, &ctl()).unwrap());
    let baseline = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(prepared.render_band(r, &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(prepared.render_band(r, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let out = prepared.render_band(r, &ctl()).unwrap();
    drop(prepared);
    drop(owner);
    assert_eq!(out.pixels().len(), 1);
    drop(out);
}
#[test]
#[ignore = "UNRUN radiusfree firstvisible zero/backdrop ordering and strictreadonlyclass boundaries"]
fn source_activity_and_old_singleton_classes_are_not_aliases() {
    let mut f = fixture(SampleType::F64, Size::new(8, 6), &[Some(1), None], None);
    f.document.layers[1].visible = false;
    let id = f.document.layers[1].id;
    f.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
    f.document.layers[0].opacity = 0.375;
    f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let (owner, _, _, _) = adopt(f);
    assert!(!owner.is_readonly_point_storage());
    assert!(!owner.is_readonly_flat_composite_storage());
    let c = PreparedBackedFilteredFlatPointDocument::new(&owner, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    let band = c.render_band(Rect::new(2, 0, 3, 1), &ctl()).unwrap();
    let expected =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap()
            .normal([0.; 4], input(0, 2, 0, SampleType::F64), 0.375, GimpOperator::Union)
            .unwrap();
    assert_eq!(band.pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
    assert_eq!(c.current_profile().encoded_bytes(), owner.document().icc_profile.as_ref().map(|v| v.as_slice()));
    assert_eq!(c.sample_encoding(), SampleEncoding::NonlinearProfile);
}

#[path = "backed_filtered_flat_point_mutation_tests.rs"]
mod mutation;

#[path = "backed_filtered_flat_point_source_activity_tests.rs"]
mod source_activity;
