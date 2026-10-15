//! UNRUN generated CURRENT group/profile custody; authentic source evidence is separate.
use super::*;
use crate::backed_stock_point_context::tests::{ctl, fixture as point_fixture};
use crate::{backed_assigned_current_profile::PreparedAssignedCurrentProfile, backed_document::PreparedBackedDocument};
use photocraft_color::{ColorMode, PixelFormat, try_write_sample_f64};
use photocraft_doc::{
    LayerMask,
    accounted_document::AccountedDocument,
    import_compositing::{GimpComposition, ResolvedGimpComposition},
    item_identity::MaskId,
    xcf_state::MaskState,
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore},
};
use std::sync::Arc;
#[derive(Debug)]
struct Constant {
    descriptor: NativePlaneDescriptor,
    q: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Constant {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let pixel = self.descriptor.default_bytes();
        for (i, v) in out.iter_mut().enumerate() {
            *v = *pixel.get(((at + i as u64) % pixel.len() as u64) as usize).ok_or(BackingError::Bounds)?;
        }
        Ok(())
    }
}
fn leaf(q: &BackingQuota, mode: ColorMode, sample: SampleType, bounds: Rect, name: &str) -> Layer {
    let fee = q.reserve(QuotaKind::Resident, (std::mem::size_of::<Constant>() + 128) as u64).unwrap();
    let format = PixelFormat::new(mode, sample, true);
    let mut bytes = [0; 32];
    let bytes = &mut bytes[..format.bytes_per_pixel()];
    for channel in 0..format.channels() {
        try_write_sample_f64(bytes, sample, channel, if channel == mode.color_channels() { 0.625 } else { 0.25 }).unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, bytes).unwrap();
    let store = Arc::new(Constant { descriptor, q: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    Layer::new(name, LayerContent::BackedRaster(photocraft_doc::BackedLayer::new(raster, descriptor, &ctl()).unwrap()))
}
fn fixture(mode: ColorMode, sample: SampleType, case: u8, defect: u8) -> AccountedDocument {
    fixture_with_backdrop(mode, sample, case, defect, true, 1.0)
}
fn fixture_with_backdrop(mode: ColorMode, sample: SampleType, case: u8, defect: u8, visible: bool, opacity: f32) -> AccountedDocument {
    let (mut doc, q, guard, _, profile, _) = point_fixture(2, mode, sample, SampleEncoding::NonlinearProfile);
    let fee = q.reserve(QuotaKind::Resident, 4 << 20).unwrap();
    doc.size = photocraft_doc::Size::new(17, 13);
    let mut base = leaf(&q, mode, sample, doc.bounds(), "Backdrop");
    base.visible = visible;
    base.opacity = opacity;
    let lower = leaf(&q, mode, sample, Rect::new(-3, -2, 8, 7), "Lower");
    let mut upper = leaf(&q, mode, sample, Rect::new(4, 3, 13, 10), "Upper");
    upper.opacity = 0.3125;
    let mut group = Layer::group("Group", vec![lower, upper]);
    group.opacity = 0.625;
    group.visible = case != 2;
    let groupbounds = Rect::new(-3, -2, 13, 10);
    if case != 3 {
        group.mask = Some(LayerMask {
            surface: Arc::new(Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[0.5])),
            enabled: case != 1,
            linked: true,
            density: 1.0,
            feather: 0.0,
        });
    }
    doc.icc_profile = Some(profile.to_bytes());
    doc.source_color_context.as_mut().unwrap().original_icc = doc.icc_profile.clone();
    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
    let state = doc.xcf_state.as_mut().unwrap();
    let template = state.items[0].clone();
    state.items.clear();
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.selection = Default::default();
    state.provenance.format_version = if group.mask.is_some() { 13 } else { 12 };
    let policy = GimpComposition {
        compatibility_epoch: EPOCH.into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: 1,
        stored_operator: 1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    };
    let LayerContent::Group(children) = &group.content else { panic!("group fixture") };
    for layer in [&base, &group, &children.children[0], &children.children[1]] {
        let mut item = template.clone();
        item.item = ItemRef::Layer(layer.id);
        item.name = layer.name.clone();
        item.visible = layer.visible;
        item.parent = if layer.id == base.id || layer.id == group.id { None } else { Some(ItemRef::Layer(group.id)) };
        item.raster_bounds = Some(if layer.id == group.id {
            groupbounds
        } else {
            let LayerContent::BackedRaster(v) = &layer.content else { panic!("leaf") };
            v.operative_bounds()
        });
        item.composition = Some(policy.clone());
        item.mask = None;
        state.items.push(item);
    }
    if let Some(mask) = &group.mask {
        let mut item = template;
        item.item = ItemRef::Mask(MaskId(9000));
        item.name = "Group mask".into();
        item.parent = None;
        item.composition = None;
        item.raster_bounds = Some(groupbounds);
        item.mask = Some(MaskState {
            owner: ItemRef::Layer(group.id),
            bounds: groupbounds,
            offset: [-3, -2],
            outside_f64_bits: 0f64.to_bits(),
            enabled: mask.enabled,
            shown: false,
            editing: false,
            linked: true,
        });
        state.items.push(item);
    }
    if defect == 1 {
        let LayerContent::Group(g) = &mut group.content else { panic!("group") };
        let child = &mut g.children[1];
        child.visible = false;
        state.items.iter_mut().find(|i| i.item == ItemRef::Layer(child.id)).unwrap().visible = false;
        let LayerContent::BackedRaster(v) = &child.content else { panic!("leaf") };
        let r = v.operative_bounds();
        let last = Rect::new(r.x1 - 1, r.y1 - 1, r.x1, r.y1);
        let mut bytes = [0; 32];
        let bytes = &mut bytes[..v.format().bytes_per_pixel()];
        v.checked_view().read_logical_pixel_native_into(last.x0, last.y0, bytes, &ctl()).unwrap();
        try_write_sample_f64(bytes, sample, 0, 1.5).unwrap();
        child.content = LayerContent::BackedRaster(v.try_edited_native(last, bytes, &ctl()).unwrap());
    }
    if defect == 2 {
        let mask = group.mask.as_mut().unwrap();
        let mut surface = Surface::new(mask.surface.format());
        surface.try_write_pixel_f64(groupbounds.x1 - 1, groupbounds.y1 - 1, &[1.5]).unwrap();
        mask.surface = Arc::new(surface);
    }
    if defect == 3 {
        let LayerContent::BackedRaster(v) = &base.content else { panic!("backdrop") };
        let r = v.operative_bounds();
        let last = Rect::new(r.x1 - 1, r.y1 - 1, r.x1, r.y1);
        let mut bytes = [0; 32];
        let bytes = &mut bytes[..v.format().bytes_per_pixel()];
        v.checked_view().read_logical_pixel_native_into(last.x0, last.y0, bytes, &ctl()).unwrap();
        try_write_sample_f64(bytes, sample, 0, 1.5).unwrap();
        base.content = LayerContent::BackedRaster(v.try_edited_native(last, bytes, &ctl()).unwrap());
    }
    doc.layers = vec![base, group];
    AccountedDocument::from_owned(doc, Arc::new((guard, fee)), q, &ctl()).unwrap()
}
#[test]
fn typed_group_profile_and_editable_hierarchy_cover_both_models_depths_and_activity() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for case in 0..4 {
                let owner = fixture(mode, sample, case, 0);
                let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).unwrap();
                assert!(binding.matches(&owner, SampleEncoding::NonlinearProfile));
                assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err());
                let typed = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).unwrap();
                let band = typed.render_band(owner.document().bounds(), &ctl()).unwrap();
                let bare = PreparedBackedDocument::new_accounted(&owner, binding.profile(), Default::default(), &ctl()).unwrap();
                assert_eq!(band.pixels(), bare.render_band(owner.document().bounds(), &ctl()).unwrap().pixels());
                let working = WorkingSpace::new(binding.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
                let r = owner.document().bounds();
                for (y, row) in band.pixels().chunks_exact(r.width() as usize).enumerate() {
                    for (x, pixel) in row.iter().enumerate() {
                        let reference = reference_layers(owner.document(), &owner.document().layers, &working, r.x0 + x as i32, r.y0 + y as i32);
                        assert_eq!(*pixel, reference, "independent isolated seed/group-mask Normal nesting");
                    }
                }
                let LayerContent::Group(g) = &owner.document().layers[1].content else { panic!("group flattened") };
                assert_eq!(g.children.len(), 2);
                let other = fixture(mode, sample, case, 0);
                assert!(PreparedBackedDocument::new_accounted_with_assigned_profile(&other, &binding, Default::default(), &ctl()).is_err());
            }
        }
    }
}
#[test]
fn hidden_group_and_disabled_mask_do_not_hide_malformed_operative_words() {
    for (case, defect) in [(2, 1), (1, 2)] {
        let owner = fixture(ColorMode::Rgb, SampleType::F64, case, defect);
        let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).unwrap();
        assert!(PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).is_err());
    }
}
#[test]
fn group_preparation_band_copy_and_three_band_peak_are_same_quota_and_cancellable() {
    let owner = fixture(ColorMode::Rgb, SampleType::F64, 0, 0);
    let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(&owner, Default::default(), &ctl()).unwrap();
    let prepared = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).unwrap();
    let before = owner.quota().usage().unwrap();
    let peak = std::sync::atomic::AtomicU64::new(before.resident);
    let observe = |_| {
        peak.fetch_max(owner.quota().usage().unwrap().resident, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    };
    let observed = ReadControl { interrupt: &Interrupt::NONE, work: &observe };
    let band = prepared.render_band(owner.document().bounds(), &observed).unwrap();
    assert!(owner.quota().usage().unwrap().resident > before.resident);
    assert!(peak.load(std::sync::atomic::Ordering::Relaxed) >= before.resident + 3 * 221 * 32, "parent, child and leaf payload fees coexist");
    drop(band);
    assert_eq!(owner.quota().usage().unwrap(), before);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n == 221 * 32 { Err(BackingError::Work) } else { Ok(()) } };
    assert!(matches!(prepared.render_band(owner.document().bounds(), &denied), Err(Error::Backing(BackingError::Work))));
    assert_eq!(owner.quota().usage().unwrap(), before);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(
        PreparedBackedDocument::new_accounted_with_assigned_profile(
            &owner,
            &binding,
            Default::default(),
            &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }
        ),
        Err(Error::Backing(BackingError::Cancelled))
    ));
    assert_eq!(owner.quota().usage().unwrap(), before);
}

fn reference_layers(doc: &Document, layers: &[Layer], working: &WorkingSpace, x: i32, y: i32) -> [f64; 4] {
    let mut backdrop = [0.; 4];
    for layer in layers {
        if !layer.visible || layer.opacity == 0.0 {
            continue;
        }
        let extent = item_bounds(doc, layer).unwrap();
        if !extent.contains(x, y) {
            continue;
        }
        let source = match &layer.content {
            LayerContent::BackedRaster(v) => {
                let mut raw = [0.; 4];
                v.checked_view().read_logical_pixel_f64_into(x, y, &mut raw[..v.format().channels()], &ctl()).unwrap();
                if doc.mode == ColorMode::Grayscale { [raw[0], raw[0], raw[0], raw[1]] } else { raw }
            }
            LayerContent::Group(g) => reference_layers(doc, &g.children, working, x, y),
            _ => panic!("closed group reference"),
        };
        let mut coverage = f64::from(layer.opacity);
        if let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) {
            let mut value = [0.];
            mask.surface.try_read_pixel_f64(x, y, &mut value).unwrap();
            coverage *= value[0];
        }
        backdrop = working.normal(backdrop, source, coverage, GimpOperator::Union).unwrap();
    }
    backdrop
}

#[path = "backed_group_activity_tests.rs"]
mod activity;
