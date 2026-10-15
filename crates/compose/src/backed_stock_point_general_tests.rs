//! UNRUN CURRENT-only regression drafts. These independently paid synthetic
//! models establish no original file qualification or GIMP numerical parity.
use super::tests::{adopt, ctl, fixture};
use super::*;
use crate::gimp_neighborhood::OrderedStack;
use photocraft_doc::{
    BackedLayer, LayerContent, Size,
    xcf_filters::{ExactNumber, FilterArgument, OperationIdentity},
};
use photocraft_raster::backing::{BackedRaster, NativePlaneStore};
use std::sync::Arc;

fn f(name: &str, value: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())) }
}
fn b(name: &str, value: bool) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Bool(value) }
}
fn e(name: &str, domain: &str, value: i32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Enum { domain: domain.into(), value } }
}
fn color(name: &str) -> FilterArgument {
    FilterArgument {
        name: name.into(),
        value: FilterValue::Color {
            components: [0.25f32, 0.5, 0.75, 1.].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
            encoding: SampleEncoding::NonlinearProfile,
            profile: None,
        },
    }
}
fn configure(doc: &mut Document, member: usize) {
    let (name, epoch, raw, arguments) = match member {
        0 => ("gegl:absolute", gimp_filters::ABSOLUTE_LINEAR_PRECISION_EPOCH, 1574142545, vec![]),
        1 => (
            "gegl:color-exchange",
            gimp_filters::COLOR_EXCHANGE_PRECISION_EPOCH,
            -466517423,
            vec![color("from-color"), color("to-color"), f("red-threshold", 0.125), f("green-threshold", 0.125), f("blue-threshold", 0.125)],
        ),
        2 => ("gegl:color-overlay", gimp_filters::COLOR_OVERLAY_PRECISION_EPOCH, 1435337297, vec![color("value"), b("srgb", true)]),
        3 => {
            let mut args = vec![];
            for (i, name) in ["rr-gain", "rg-gain", "rb-gain", "gr-gain", "gg-gain", "gb-gain", "br-gain", "bg-gain", "bb-gain"].into_iter().enumerate() {
                args.push(f(name, if i % 4 == 0 { 1.25 } else { 0.0 }));
            }
            args.push(b("preserve-luminosity", false));
            ("gegl:channel-mixer", gimp_filters::CHANNEL_MIXER_PRECISION_EPOCH, -1150320047, args)
        }
        4 => (
            "gegl:mono-mixer",
            gimp_filters::MONO_MIXER_SIGNED_PRECISION_EPOCH,
            1799848529,
            vec![f("red", -0.5), f("green", 0.25), f("blue", 0.125), b("preserve-luminosity", false)],
        ),
        5 => ("gegl:red-eye-removal", gimp_filters::RED_EYE_PRECISION_EPOCH, 127500881, vec![f("threshold", 0.4)]),
        6 => (
            "gegl:color-rotate",
            crate::gimp_color_rotate::COLOR_ROTATE_PERCEPTUAL_EPOCH,
            -1567849903,
            vec![
                b("src-clockwise", false),
                f("src-from", 15.),
                f("src-to", 120.),
                b("dest-clockwise", false),
                f("dest-from", 160.),
                f("dest-to", 280.),
                f("threshold", 0.5),
                e("gray-mode", "gegl:color-rotate/gray-mode", 0),
                f("hue", 45.),
                f("saturation", 0.75),
            ],
        ),
        7 => (
            "gegl:video-degradation",
            crate::gimp_video_degradation::VIDEO_DEGRADATION_EPOCH,
            -1877441967,
            vec![e("pattern", "gegl:video-degradation/pattern", 5), b("additive", false), b("rotated", false)],
        ),
        8 => (
            "gegl:color-to-alpha",
            gimp_filters::COLOR_TO_ALPHA_PRECISION_EPOCH,
            -466517423,
            vec![color("color"), f("transparency-threshold", 0.), f("opacity-threshold", 1.)],
        ),
        9 => (
            "gegl:component-extract",
            gimp_filters::COMPONENT_EXTRACT_PRECISION_EPOCH,
            -466517423,
            vec![e("component", "gegl:component-extract/component", 0), b("invert", false), b("linear", false)],
        ),
        10 => (
            "gegl:component-extract",
            gimp_filters::COMPONENT_EXTRACT_CMYK_PRECISION_EPOCH,
            -305954223,
            vec![e("component", "gegl:component-extract/component", 8), b("invert", true), b("linear", false)],
        ),
        11 => (
            "gegl:component-extract",
            gimp_filters::COMPONENT_EXTRACT_YCBCR_PRECISION_EPOCH,
            -305954223,
            vec![e("component", "gegl:component-extract/component", 12), b("invert", false), b("linear", true)],
        ),
        12 => (
            "gegl:component-extract",
            gimp_filters::COMPONENT_EXTRACT_GEOMETRY_EPOCH,
            87327313,
            vec![e("component", "gegl:component-extract/component", 8), b("invert", false), b("linear", false)],
        ),
        13 => (
            "gegl:alpha-clip",
            gimp_filters::ALPHA_CLIP_PRECISION_EPOCH,
            -1710980527,
            vec![b("clip-low", true), f("low-limit", 0.25), b("clip-high", true), f("high-limit", 0.75)],
        ),
        14 => (
            "gegl:alpha-clip",
            gimp_filters::ALPHA_CLIP_FINITE_PRECISION_EPOCH,
            -305954223,
            vec![b("clip-low", true), f("low-limit", 0.25), b("clip-high", true), f("high-limit", 0.75)],
        ),
        _ => panic!("fixed fixture case"),
    };
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: name.into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    node.arguments = arguments;
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
    policy.stored_blend_space = raw;
    policy.stored_composite_space = raw;
    policy.stored_operator = raw;
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
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
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn reframe(doc: &mut Document, q: &BackingQuota, canvas: Size, bounds: Rect) {
    reframe_coverage(doc, q, canvas, bounds, false);
}
pub(super) fn reframe_coverage(doc: &mut Document, q: &BackingQuota, canvas: Size, bounds: Rect, opaque: bool) {
    let format = doc.pixel_format();
    let length = bounds.width() as usize * bounds.height() as usize * format.bytes_per_pixel();
    assert!(length <= 65536);
    let fee = q.reserve(QuotaKind::Resident, (2 * length + 512) as u64).unwrap();
    let mut bytes = vec![0; length];
    assert!(bytes.capacity() <= 2 * length);
    for (i, pixel) in bytes.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
        for channel in 0..3 {
            photocraft_color::try_write_sample_f64(pixel, format.sample, channel, [0.25, 0.5, 0.75][channel]).unwrap();
        }
        photocraft_color::try_write_sample_f64(
            pixel,
            format.sample,
            3,
            if opaque {
                1.0
            } else if i % 2 == 0 {
                0.125
            } else {
                0.75
            },
        )
        .unwrap();
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, &vec![0; format.bytes_per_pixel()]).unwrap();
    let store = Arc::new(Store { descriptor, bytes, quota: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    doc.size = canvas;
    let id = doc.layers[0].id;
    let canvas = doc.bounds();
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_masks[0].bounds = canvas;
    for item in &mut state.items {
        if item.item == ItemRef::Layer(id) {
            item.raster_bounds = Some(bounds);
        } else {
            item.raster_bounds = Some(canvas);
            item.mask.as_mut().unwrap().bounds = canvas;
        }
    }
}

#[test]
fn remaining_fifteen_exact_current_families_match_upstream_ordered_storage_and_coordinates() {
    for member in 0..15 {
        let sample = if member == 0 || member == 14 { SampleType::F64 } else { SampleType::F32 };
        let stored = if member == 0 {
            SampleEncoding::Linear
        } else if member == 6 {
            SampleEncoding::Perceptual
        } else {
            SampleEncoding::NonlinearProfile
        };
        let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, sample, stored);
        configure(&mut doc, member);
        if member == 6 {
            reframe(&mut doc, &q, Size::new(32, 16), Rect::new(0, 0, 32, 16));
        }
        if member == 7 {
            reframe(&mut doc, &q, Size::new(16, 16), Rect::new(0, 0, 16, 16));
        }
        if member == 12 {
            reframe(&mut doc, &q, Size::new(37, 29), Rect::new(-7, 5, 12, 18));
        }
        let owner = adopt(doc, q, guard);
        let doc = owner.document();
        let state = doc.xcf_state.as_ref().unwrap();
        let layer = match &doc.layers[0].content {
            LayerContent::BackedRaster(v) => v,
            _ => panic!(),
        };
        let current = PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
        current.validate_operative(&ctl()).unwrap();
        let actual = current.render_wide_band(layer.operative_bounds(), &ctl()).unwrap();
        let old = OrderedStack::new_with_interrupt(&state.filter_stacks[0], profile, stored, 22, layer.operative_bounds(), [false; 3], &Interrupt::NONE)
            .unwrap()
            .with_current_point_storage(Some(sample))
            .with_filter_output_precision(state.current_filter_output_precision);
        let mut raw = [0; 40];
        let mut i = 0;
        for y in layer.operative_bounds().y0..layer.operative_bounds().y1 {
            for x in layer.operative_bounds().x0..layer.operative_bounds().x1 {
                layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[..layer.format().bytes_per_pixel()], &ctl()).unwrap();
                let input = std::array::from_fn(|c| try_read_sample_f64(&raw, sample, c).unwrap());
                let expected = old.point_at(input, (x, y), &mut |_| Ok(())).unwrap();
                assert_eq!(actual.pixels()[i].map(f64::to_bits), expected.map(f64::to_bits), "member{member} at{x},{y}");
                i += 1;
            }
        }
    }
}

#[test]
fn generated_alpha_half_and_signed_native_boundary_are_the_existing_ordered_schedule() {
    for member in [3, 4, 8, 9, 13] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let (mut doc, q, _guard, _, profile, raw) = fixture(0, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
            configure(&mut doc, member);
            if sample == SampleType::F32 {
                doc.source_color_context.as_mut().unwrap().source_precision = 550;
                doc.xcf_state.as_mut().unwrap().current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Half;
            }
            let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
            let state = doc.xcf_state.as_ref().unwrap();
            let old = OrderedStack::new_with_interrupt(
                &state.filter_stacks[0],
                profile,
                doc.sample_encoding.unwrap(),
                22,
                doc.bounds(),
                [false; 3],
                &Interrupt::NONE,
            )
            .unwrap()
            .with_current_point_storage(Some(sample))
            .with_filter_output_precision(state.current_filter_output_precision);
            for pixel in raw.chunks_exact(doc.pixel_format().bytes_per_pixel()) {
                let input = std::array::from_fn(|c| try_read_sample_f64(pixel, sample, c).unwrap());
                let expected = old.point_at(input, (0, 0), &mut |_| Ok(())).unwrap();
                let actual = pipeline.evaluate_native_at(pixel, doc.pixel_format(), (0, 0), &ctl()).unwrap();
                assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
}

#[test]
fn disabled_requires_working_input_validation_but_never_invokes_color_or_alpha_equation() {
    for member in [0, 1, 13] {
        let stored = if member == 0 { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
        let (mut doc, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F64, stored);
        configure(&mut doc, member);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
        let mut raw = [0; 32];
        for (i, v) in [-1.0f64, -0.0, 0.75, 0.125].into_iter().enumerate() {
            photocraft_color::try_write_sample_f64(&mut raw, SampleType::F64, i, v).unwrap();
        }
        if member == 0 {
            assert_eq!(
                pipeline.evaluate_native_at(&raw, doc.pixel_format(), (0, 0), &ctl()).unwrap().map(f64::to_bits),
                [-1.0f64, -0.0, 0.75, 0.125].map(f64::to_bits)
            );
        } else {
            assert!(pipeline.evaluate_native_at(&raw, doc.pixel_format(), (0, 0), &ctl()).is_err());
        }
    }
}

#[test]
fn video_absolute_world_phase_is_stable_across_sub_bands_and_bad_special_tuple_stays_closed() {
    let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F32, SampleEncoding::NonlinearProfile);
    configure(&mut doc, 7);
    reframe(&mut doc, &q, Size::new(16, 16), Rect::new(0, 0, 16, 16));
    let owner = adopt(doc, q, guard);
    let context = PreparedBackedStockPointContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
    let whole = context.render_wide_band(Rect::new(0, 0, 16, 16), &ctl()).unwrap();
    let part = context.render_wide_band(Rect::new(5, 3, 12, 9), &ctl()).unwrap();
    for y in 3..9usize {
        for x in 5..12usize {
            assert_eq!(part.pixels()[(y - 3) * 7 + x - 5].map(f64::to_bits), whole.pixels()[y * 16 + x].map(f64::to_bits));
        }
    }
    let (mut bad, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F64, SampleEncoding::Linear);
    configure(&mut bad, 0);
    bad.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0;
    assert!(PreparedStockPointPipeline::prepare(&bad, profile, &q, &ctl()).is_err());
}
