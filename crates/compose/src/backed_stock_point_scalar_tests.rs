//! UNRUN synthetic CURRENT scalar continuity drafts. Computed native fixtures
//! establish no original file correspondence or reference numerical parity.
use super::tests::{adopt, ctl, fixture};
use super::*;
use photocraft_doc::{
    BackedLayer, Size,
    xcf_filters::{ExactNumber, FilterArgument},
};
use photocraft_raster::backing::{BackedRaster, NativePlaneStore};
use std::sync::Arc;
fn f(name: &str, v: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
fn b(name: &str, v: bool) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Bool(v) }
}
fn configure(doc: &mut Document, kind: u8) {
    let (name, epoch, raw, args) = match kind {
        0 => ("gegl:exposure", gimp_filters::EXPOSURE_GEOMETRY_EPOCH, -901479855, vec![f("exposure", 0.5), f("black-level", 0.125)]),
        1 => (
            "gegl:rgb-clip",
            gimp_filters::RGB_CLIP_HDR_EPOCH,
            1825931857,
            vec![b("clip-low", true), b("clip-high", false), f("low-limit", 0.), f("high-limit", 1.)],
        ),
        2 => ("gegl:unpremultiply", crate::gimp_unpremultiply::UNPREMULTIPLY_EPOCH, -1877441967, vec![]),
        3 | 4 => (
            "gegl:mono-mixer",
            if kind == 3 { "xcf-22-stock-point-replace" } else { gimp_filters::MONO_MIXER_EDITS_EPOCH },
            if kind == 3 { -261651887 } else { 107971153 },
            vec![f("red", 0.5), f("green", 0.25), f("blue", 0.25), b("preserve-luminosity", false)],
        ),
        5 => ("gegl:value-invert", "xcf-22-stock-point-replace", -261651887, vec![]),
        _ => panic!(),
    };
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = name.into();
    node.operation.schema_epoch = epoch.into();
    node.operation.stored_version = if kind == 0 { "1:0" } else { "0:0" }.into();
    node.arguments = args;
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = epoch.into();
    policy.stored_blend_space = raw;
    policy.stored_composite_space = raw;
    policy.stored_operator = raw;
}
#[derive(Debug)]
struct ComputedStore {
    descriptor: NativePlaneDescriptor,
    pixel: [u8; 40],
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for ComputedStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        let end = offset.checked_add(out.len() as u64).ok_or(BackingError::Bounds)?;
        if end > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        let bpp = self.descriptor.format().bytes_per_pixel();
        for (block_index, block) in out.chunks_mut(4096).enumerate() {
            ctl.check(block.len() as u64)?;
            for (i, byte) in block.iter_mut().enumerate() {
                let at = usize::try_from(offset).map_err(|_| BackingError::Bounds)?.checked_add(block_index * 4096 + i).ok_or(BackingError::Bounds)?;
                *byte = *self.pixel.get(at % bpp).ok_or(BackingError::Length)?;
            }
        }
        Ok(())
    }
}
fn native(doc: &mut Document, q: &BackingQuota, canvas: Size, bounds: Rect, rgba: [f64; 4]) {
    let format = doc.pixel_format();
    let mut pixel = [0; 40];
    for c in 0..format.mode.color_channels() {
        photocraft_color::try_write_sample_f64(&mut pixel, format.sample, c, rgba[c]).unwrap();
    }
    photocraft_color::try_write_sample_f64(&mut pixel, format.sample, format.mode.color_channels(), rgba[3]).unwrap();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 40][..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, (std::mem::size_of::<ComputedStore>() + 128) as u64).unwrap();
    let store = Arc::new(ComputedStore { descriptor, pixel, quota: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    doc.size = canvas;
    let id = doc.layers[0].id;
    let canvas = doc.bounds();
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_masks[0].bounds = canvas;
    for item in &mut state.items {
        if item.item == ItemRef::Layer(id) {
            item.raster_bounds = Some(bounds)
        } else {
            item.raster_bounds = Some(canvas);
            item.mask.as_mut().unwrap().bounds = canvas
        }
    }
}
fn compare(owner: &AccountedDocument, profile: &Profile) {
    let doc = owner.document();
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
    let state = doc.xcf_state.as_ref().unwrap();
    let current = PreparedBackedStockPointContext::new(owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    current.validate_operative(&ctl()).unwrap();
    let bounds = layer.operative_bounds();
    let roi = Rect::new(bounds.x0, bounds.y0, bounds.x0 + 3, bounds.y0 + 2);
    let output = current.render_wide_band(roi, &ctl()).unwrap();
    // Exact legacy routing: Unpremultiply alone uses the generic F32 setter;
    // the other five families retain the nongeneric None schedule.
    let legacy = OrderedStack::new_with_interrupt(&state.filter_stacks[0], profile, doc.sample_encoding.unwrap(), 22, bounds, [false; 3], &Interrupt::NONE)
        .unwrap()
        .with_current_point_storage(if state.filter_stacks[0].nodes[0].operation.name == "gegl:unpremultiply" { Some(doc.depth) } else { None });
    let mut raw = [0; 40];
    let mut i = 0;
    for y in roi.y0..roi.y1 {
        for x in roi.x0..roi.x1 {
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[..layer.format().bytes_per_pixel()], &ctl()).unwrap();
            let read = |c| try_read_sample_f64(&raw, doc.depth, c).unwrap();
            let rgb = if doc.mode == ColorMode::Grayscale { [read(0); 3] } else { [read(0), read(1), read(2)] };
            let expected = legacy.point_at([rgb[0], rgb[1], rgb[2], read(doc.mode.color_channels())], (x, y), &mut |_| Ok(())).unwrap();
            assert_eq!(output.pixels()[i].map(f64::to_bits), expected.map(f64::to_bits));
            i += 1;
        }
    }
}
#[test]
fn six_remaining_current_families_match_old_native_input_and_wide_schedule() {
    for kind in 0..6 {
        for enabled in [false, true] {
            let sample = if kind >= 3 {
                SampleType::U8
            } else if kind == 0 {
                SampleType::U16
            } else {
                SampleType::F32
            };
            let stored = if kind >= 3 { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
            let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, sample, stored);
            configure(&mut doc, kind);
            if kind == 0 {
                native(&mut doc, &q, Size::new(295, 160), Rect::new(-13, 13, 244, 142), [0.25, 0.5, 0.75, 0.5]);
            } else if kind >= 3 {
                native(&mut doc, &q, Size::new(32, 32), Rect::new(0, 0, 32, 32), [0.25, 0.5, 0.75, 1.]);
            } else {
                native(&mut doc, &q, Size::new(8, 6), Rect::new(0, 0, 8, 6), [-3., 2., 0.75, if kind == 1 { 1. } else { 0.5 }]);
            }
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
            if kind == 3 {
                doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None;
                doc.xcf_state.as_mut().unwrap().filter_masks.clear();
                doc.xcf_state.as_mut().unwrap().items.retain(|item| matches!(item.item, ItemRef::Layer(_)));
            }
            let owner = adopt(doc, q, guard);
            compare(&owner, profile);
        }
    }
}
#[test]
fn unpremultiply_keeps_disabled_division_bypass_but_enabled_float_overflow_refuses() {
    let (mut doc, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F32, SampleEncoding::Linear);
    configure(&mut doc, 2);
    let raw = [f32::MAX, 0.5, 0.75, f32::MIN_POSITIVE].map(f32::to_ne_bytes).concat();
    let enabled = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    assert!(enabled.evaluate_native(&raw, doc.pixel_format(), &ctl()).is_err());
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let disabled = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    assert_eq!(disabled.evaluate_native(&raw, doc.pixel_format(), &ctl()).unwrap()[0], f64::from(f32::MAX));
}
#[test]
fn rgb_clip_retains_hdr_but_exact_opaque_and_precision_limits_are_not_widened() {
    let (mut doc, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F32, SampleEncoding::Linear);
    configure(&mut doc, 1);
    let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    let raw = [-3f32, 2., 0.75, 1.].map(f32::to_ne_bytes).concat();
    assert_eq!(pipeline.evaluate_native(&raw, doc.pixel_format(), &ctl()).unwrap(), [0., 2., 0.75, 1.]);
    let fractional = [-3f32, 2., 0.75, 0.5].map(f32::to_ne_bytes).concat();
    assert!(pipeline.evaluate_native(&fractional, doc.pixel_format(), &ctl()).is_err());
    doc.source_color_context.as_mut().unwrap().source_precision = 500;
    assert!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).is_err());
}
