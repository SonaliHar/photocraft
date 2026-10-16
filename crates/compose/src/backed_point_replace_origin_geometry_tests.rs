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
fn reframe_coverage(doc: &mut Document, q: &BackingQuota, canvas: Size, bounds: Rect, opaque: bool) {
    let format = doc.pixel_format();
    let length = bounds.width() as usize * bounds.height() as usize * format.bytes_per_pixel();
    assert!(length <= 8 << 20);
    let fee = q.reserve(QuotaKind::Resident, (2 * length + 512) as u64).unwrap();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).unwrap();
    assert!(bytes.capacity() <= 2 * length);
    bytes.resize(length, 0);
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

const MEMBERS: [usize; 9] = [0, 2, 3, 4, 9, 10, 11, 13, 14];
fn larger(member: usize, sample: SampleType, size: Size) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile) {
    let stored = if member == 0 { SampleEncoding::Linear } else { SampleEncoding::NonlinearProfile };
    let (mut doc, q, original, _, basis, _) = fixture(0, ColorMode::Rgb, sample, stored);
    // Prospectively admit copied/changed fixture containers and actual physical
    // zero-mask payload before replacement. Native Vec has a separate 2x fee.
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let retained: Arc<dyn std::any::Any + Send + Sync> = Arc::new((original, index, resident));
    configure(&mut doc, member);
    reframe(&mut doc, &q, size, Rect::new(0, 0, size.width as i32, size.height as i32));
    (doc, q, retained, basis)
}
fn metadata(doc: &Document, basis: &Profile, extent: Rect) -> Result<(), CompositionError> {
    let state = doc.xcf_state.as_ref().unwrap();
    crate::gimp_render::qualify_point_family_context(
        doc,
        &doc.layers[0],
        state,
        &state.filter_stacks[0],
        extent,
        basis,
        doc.sample_encoding.unwrap(),
        &Interrupt::NONE,
    )
    .map(|_| ())
}
#[test]
#[ignore = "UNRUN synthetic nine-domain geometry/reference contract"]
fn nine_domains_multitile_current_matches_existing_ordered_pixel_schedule() {
    for member in MEMBERS {
        for sample in [SampleType::F32, SampleType::F64] {
            for size in [Size::new(1, 257), Size::new(257, 3)] {
                let (doc, q, retained, basis) = larger(member, sample, size);
                let owner = adopt(doc, q, retained);
                let current = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits::default(), &ctl()).unwrap();
                current.validate_operative(&ctl()).unwrap();
                let doc = owner.document();
                let state = doc.xcf_state.as_ref().unwrap();
                let old = OrderedStack::new_with_interrupt(
                    &state.filter_stacks[0],
                    basis,
                    doc.sample_encoding.unwrap(),
                    22,
                    doc.bounds(),
                    [false; 3],
                    &Interrupt::NONE,
                )
                .unwrap()
                .with_current_point_storage(Some(sample))
                .with_filter_output_precision(state.current_filter_output_precision);
                let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
                for bounds in [Rect::new(0, 0, 1, 1), Rect::new(size.width as i32 - 1, size.height as i32 - 1, size.width as i32, size.height as i32)] {
                    let actual = current.render_wide_band(bounds, &ctl()).unwrap();
                    let mut raw = [0; 40];
                    layer.checked_view().read_logical_pixel_native_into(bounds.x0, bounds.y0, &mut raw[..layer.format().bytes_per_pixel()], &ctl()).unwrap();
                    let input = std::array::from_fn(|c| try_read_sample_f64(&raw, sample, c).unwrap());
                    let expected = old.point_at(input, (bounds.x0, bounds.y0), &mut |_| Ok(())).unwrap();
                    assert_eq!(actual.pixels()[0].map(f64::to_bits), expected.map(f64::to_bits), "{member}/{sample:?}/{size:?}");
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN plain Document canonical/native defense"]
fn canonical_native_doc_and_stack_join_refuse_without_borrowing_l1() {
    for member in MEMBERS {
        let (mut doc, q, _retained, basis) = larger(member, SampleType::F32, Size::new(257, 3));
        assert!(metadata(&doc, basis, doc.bounds()).is_ok());
        let extent = doc.bounds();
        let id = doc.layers[0].id;
        let row = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap();
        row.raster_bounds = Some(Rect::new(0, 0, 8, 6));
        assert!(PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).is_err());
        assert!(metadata(&doc, basis, extent).is_err());
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(extent);
        doc.size = Size::new(256, 3);
        assert!(metadata(&doc, basis, extent).is_err());
        doc.size = Size::new(257, 3);
        assert!(metadata(&doc, basis, Rect::new(-1, 0, 256, 3)).is_err());
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].owner = ItemRef::Mask(photocraft_doc::item_identity::MaskId(999999));
        assert!(metadata(&doc, basis, extent).is_err());
    }
    // Direct metadata defense, no giant native allocation or source claim.
    let (mut huge, _q, _retained, basis) = larger(9, SampleType::F32, Size::new(1, 1));
    huge.size = Size::new(65536, 32769);
    let extent = huge.bounds();
    let id = huge.layers[0].id;
    huge.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(extent);
    assert!(metadata(&huge, basis, extent).is_err());
    // The separately witnessed ComponentGeometry remains offset-specific.
    let (mut doc, q, _retained, basis) = larger(9, SampleType::F32, Size::new(257, 3));
    configure(&mut doc, 12);
    assert!(metadata(&doc, basis, doc.bounds()).is_err());
    reframe(&mut doc, &q, Size::new(37, 29), Rect::new(-7, 5, 12, 18));
    assert!(metadata(&doc, basis, Rect::new(-7, 5, 12, 18)).is_ok());
}
#[test]
#[ignore = "UNRUN full physical byte-zero mask contract"]
fn physical_padding_default_signed_zero_and_disabled_mask_are_not_shortcuts() {
    use photocraft_raster::Surface;
    for fault in 0..3 {
        let (mut doc, q, _retained, basis) = larger(2, SampleType::F32, Size::new(257, 3));
        let state = doc.xcf_state.as_mut().unwrap();
        let format = state.filter_masks[0].surface.format();
        let mut surface = Surface::with_default(format, &[if fault == 0 { -0f32 } else { 0f32 }]);
        if fault > 0 {
            // Beyond operative y3, still inside a retained physical tile.
            let word = if fault == 1 { -0f32 } else { f32::NAN };
            surface.try_write_interleaved(Rect::new(0, 200, 1, 201), &word.to_ne_bytes()).unwrap();
        }
        state.filter_masks[0].surface = surface;
        state.filter_stacks[0].nodes[0].enabled = false;
        doc.layers[0].visible = false;
        let id = doc.layers[0].id;
        state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        assert!(PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN family gates remain separate from geometry"]
fn inactive_proof_and_existing_encoding_precision_rules_remain() {
    for member in MEMBERS {
        let (mut doc, q, _retained, basis) = larger(member, SampleType::F64, Size::new(1, 257));
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        assert!(PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).is_ok());
        doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSGray { icc_blake3: "0".repeat(64) };
        assert!(PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).is_err());
    }
    let (mut absolute, _q, _retained, basis) = larger(0, SampleType::F32, Size::new(257, 1));
    absolute.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    assert!(metadata(&absolute, basis, absolute.bounds()).is_err());
    let (mut alpha, _q, _retained, basis) = larger(14, SampleType::F32, Size::new(257, 1));
    alpha.xcf_state.as_mut().unwrap().current_filter_output_precision = photocraft_doc::FilterOutputPrecision::Half;
    assert!(metadata(&alpha, basis, alpha.bounds()).is_err());
}

#[test]
#[ignore = "UNRUN same-ledger preparation and warmed-band work/cancel refunds"]
fn larger_preparation_and_band_failures_keep_real_custody() {
    for member in MEMBERS {
        let (doc, q, retained, basis) = larger(member, SampleType::F64, Size::new(257, 3));
        let before = q.usage().unwrap();
        let scalar = PreparedStockPointPipeline::prepare(&doc, basis, &q, &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        drop(scalar);
        assert_eq!(q.usage().unwrap(), before);
        let owner = adopt(doc, q.clone(), retained);
        let context = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits { max_rows: 2, max_pixels: 16 }, &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        drop(context.render_wide_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap());
        let baseline = q.usage().unwrap();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(context.render_wide_band(Rect::new(0, 0, 1, 1), &denied), Err(BackedNormalError::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), baseline);
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(context.render_wide_band(Rect::new(0, 0, 1, 1), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        let output = context.render_wide_band(Rect::new(0, 0, 8, 1), &ctl()).unwrap();
        drop(context);
        drop(owner);
        assert_eq!(output.pixels().len(), 8);
        drop(output);
    }
}
