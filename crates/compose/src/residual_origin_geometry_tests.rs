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

fn configure_residual(doc: &mut Document, family: u8, choice: i32) {
    let (name, epoch, raw, arguments) = match family {
        0 => (
            "gimp:desaturate",
            gimp_filters::DESATURATE_PRECISION_EPOCH,
            1789100625,
            vec![FilterArgument { name: "mode".into(), value: FilterValue::Enum { domain: "GimpDesaturateMode".into(), value: choice } }],
        ),
        1..=3 => (
            "gegl:saturation",
            match family {
                1 => gimp_filters::SATURATION_PRECISION_EPOCH,
                2 => gimp_filters::SATURATION_GRAY_PRECISION_EPOCH,
                _ => gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH,
            },
            -1581350319,
            vec![
                FilterArgument { name: "scale".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
                FilterArgument { name: "colorspace".into(), value: FilterValue::Enum { domain: "GeglSaturationType".into(), value: choice } },
            ],
        ),
        4 => ("gegl:value-invert", gimp_filters::VALUE_INVERT_PRECISION_EPOCH, -1395293615, vec![]),
        5 => (
            "gimp:posterize",
            "xcf-22-stock-point-replace",
            -261651887,
            vec![FilterArgument { name: "levels".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(choice) }) }],
        ),
        _ => panic!(),
    };
    let state = doc.xcf_state.as_mut().unwrap();
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: name.into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    node.arguments = arguments;
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = epoch.into();
    policy.stored_blend_space = raw;
    policy.stored_composite_space = raw;
    policy.stored_operator = raw;
}

fn scalar(name: &str, value: f64) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(value.to_bits())) }
}
fn configure(doc: &mut Document, family: u8) {
    if family < 5 {
        configure_residual(doc, family, if family == 2 || family == 3 { 2 } else { 0 });
        return;
    }
    let epoch = if family == 5 { crate::gimp_color_balance::EPOCH } else { crate::gimp_color_balance::DEPTH_EPOCH };
    let raw = if family == 5 { -113999279 } else { -1215462831 };
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: "gimp:color-balance".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    node.arguments = vec![
        FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) },
        FilterArgument {
            name: "config".into(),
            value: FilterValue::Config(
                ["shadows", "midtones", "highlights"]
                    .map(|name| FilterArgument {
                        name: name.into(),
                        value: FilterValue::Config([scalar("cyan-red", 0.125), scalar("magenta-green", -0.25), scalar("yellow-blue", 0.0625)].into()),
                    })
                    .into(),
            ),
        },
    ];
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = epoch.into();
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
        for channel in 0..format.mode.color_channels() {
            photocraft_color::try_write_sample_f64(pixel, format.sample, channel, [0.25, 0.5, 0.75][channel]).unwrap();
        }
        photocraft_color::try_write_sample_f64(
            pixel,
            format.sample,
            format.mode.color_channels(),
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

fn larger(family: u8, size: Size) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile) {
    let model = if family == 2 { ColorMode::Grayscale } else { ColorMode::Rgb };
    let stored = if family == 4 {
        SampleEncoding::Linear
    } else if family == 6 {
        SampleEncoding::Perceptual
    } else {
        SampleEncoding::NonlinearProfile
    };
    let (mut doc, q, original, _, basis, _) = fixture(0, model, SampleType::F32, stored);
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let guard: Arc<dyn std::any::Any + Send + Sync> = Arc::new((original, index, resident));
    configure(&mut doc, family);
    reframe(&mut doc, &q, size, Rect::new(0, 0, size.width as i32, size.height as i32));
    (doc, q, guard, basis)
}
fn mask(doc: &Document, c: &Interrupt<'_>) -> Result<(), CompositionError> {
    let state = doc.xcf_state.as_ref().unwrap();
    crate::gimp_neighborhood::qualify_selection_mask_with_interrupt(state, &state.filter_stacks[0].nodes[0], doc.layers[0].id, doc.bounds(), c)
}
#[test]
#[ignore = "UNRUN seven selected epochs origin/multitile CURRENT vs unchanged OrderedStack"]
fn seven_current_origin_geometries_keep_existing_pixel_schedule() {
    for family in 0..7 {
        for size in [Size::new(1, 257), Size::new(257, 3)] {
            let (doc, q, guard, basis) = larger(family, size);
            let owner = adopt(doc, q, guard);
            let context = PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl()).unwrap();
            context.validate_operative(&ctl()).unwrap();
            let d = owner.document();
            let state = d.xcf_state.as_ref().unwrap();
            let old =
                OrderedStack::new_with_interrupt(&state.filter_stacks[0], basis, d.sample_encoding.unwrap(), 22, d.bounds(), [false; 3], &Interrupt::NONE)
                    .unwrap()
                    .with_current_point_storage(Some(d.depth))
                    .with_filter_output_precision(state.current_filter_output_precision);
            let LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!() };
            for (x, y) in [(0, 0), (size.width as i32 - 1, size.height as i32 - 1)] {
                let actual = context.render_wide_band(Rect::new(x, y, x + 1, y + 1), &ctl()).unwrap();
                let mut raw = [0; 40];
                layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[..layer.format().bytes_per_pixel()], &ctl()).unwrap();
                let read = |c| try_read_sample_f64(&raw, d.depth, c).unwrap();
                let rgb = if d.mode == ColorMode::Grayscale { [read(0); 3] } else { [read(0), read(1), read(2)] };
                let expected = old.point_at([rgb[0], rgb[1], rgb[2], read(d.mode.color_channels())], (x, y), &mut |_| Ok(())).unwrap();
                assert_eq!(actual.pixels()[0].map(f64::to_bits), expected.map(f64::to_bits), "family{family}");
            }
        }
    }
}
#[test]
#[ignore = "UNRUN actual canonical/native/doc owner join and excluded geometry"]
fn plain_document_mismatches_refuse_before_assuming_l1() {
    for family in 0..7 {
        let (mut d, q, _guard, basis) = larger(family, Size::new(257, 3));
        let id = d.layers[0].id;
        d.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(Rect::new(0, 0, 8, 6));
        assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_err());
        let bounds = d.bounds();
        d.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(bounds);
        d.size = Size::new(256, 3);
        assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_err());
    }
    let (mut d, _q, _guard, _basis) = larger(0, Size::new(1, 1));
    d.size = Size::new(65536, 32769);
    let e = d.bounds();
    let id = d.layers[0].id;
    let state = d.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(e);
    assert!(
        crate::gimp_render::qualify_residual_origin_context(
            &d,
            &d.layers[0],
            d.xcf_state.as_ref().unwrap(),
            &d.xcf_state.as_ref().unwrap().filter_stacks[0],
            e
        )
        .is_err()
    );
    assert!(!crate::gimp_render::residual_origin_extent(Rect::new(-1, 0, 256, 3)));
    let (mut d, q, _guard, basis) = larger(0, Size::new(257, 3));
    let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    n.operation.name = "gimp:threshold".into();
    n.operation.schema_epoch = gimp_filters::THRESHOLD_PRECISION_EPOCH.into();
    assert!(!crate::gimp_render::residual_origin_node(n));
    assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN old8x6 accepted padding vs expanded fullphysical BYTEzero/outgrid/cap policy"]
fn expanded_mask_proof_is_fullphysical_but_old_small_behavior_survives() {
    for family in 0..7 {
        let (mut old, _q, _guard, _basis) = larger(family, Size::new(8, 6));
        old.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &(-0f32).to_ne_bytes()).unwrap();
        assert!(mask(&old, &Interrupt::NONE).is_ok());
        let (mut d, _q, _guard, _basis) = larger(family, Size::new(257, 3));
        d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &(-0f32).to_ne_bytes()).unwrap();
        assert!(mask(&d, &Interrupt::NONE).is_err());
        let surface = &mut d.xcf_state.as_mut().unwrap().filter_masks[0].surface;
        *surface = photocraft_raster::Surface::new(surface.format());
        let zero = Arc::new(surface.try_default_tile().unwrap());
        // One out-grid zero tile within the admitted TWO-tile budget is kept.
        surface.put_tiles([(photocraft_geom::TileCoord::new(9, 9), zero.clone())]);
        assert!(mask(&d, &Interrupt::NONE).is_ok());
        let surface = &mut d.xcf_state.as_mut().unwrap().filter_masks[0].surface;
        surface.put_tiles([(photocraft_geom::TileCoord::new(10, 9), zero.clone()), (photocraft_geom::TileCoord::new(11, 9), zero)]);
        // Cap admission is O1 before any physical byte loop; no new tile pruning.
        assert!(
            crate::gimp_neighborhood::qualify_expanded_residual_mask_storage(
                d.xcf_state.as_ref().unwrap(),
                &d.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0],
                d.bounds()
            )
            .is_err()
        );
        assert_eq!(d.xcf_state.as_ref().unwrap().filter_masks[0].surface.tile_count(), 3);
    }
}
#[test]
#[ignore = "UNRUN exact family precision/activity/profile restrictions unaffected by geometry"]
fn no_activity_profile_or_yuv_precision_grant() {
    for family in 0..7 {
        let (mut d, q, _guard, basis) = larger(family, Size::new(257, 3));
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_ok());
        d.layers[0].opacity = 0.;
        assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_err());
        d.layers[0].opacity = 1.;
        d.layers[0].visible = false;
        let id = d.layers[0].id;
        d.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
        assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_err());
    }
    let (mut d, q, _guard, basis) = larger(3, Size::new(257, 3));
    // Wrong marker is rejected independently of active kernel/geometry.
    d.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
    assert!(PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN larger actual sameledger prep/band controls and cleanup"]
fn one_ledger_preparation_and_warmed_band_failures() {
    for family in 0..7 {
        let (d, q, guard, basis) = larger(family, Size::new(257, 3));
        let before = q.usage().unwrap();
        let prepared = PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        drop(prepared);
        assert_eq!(q.usage().unwrap(), before);
        let owner = adopt(d, q.clone(), guard);
        let context = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits { max_rows: 2, max_pixels: 16 }, &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        drop(context.render_wide_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap());
        let baseline = q.usage().unwrap();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(context.render_wide_band(Rect::new(0, 0, 1, 1), &denied), Err(BackedNormalError::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), baseline);
        let stop = || true;
        let interrupted = Interrupt::cancel_only(&stop);
        assert!(context.render_wide_band(Rect::new(0, 0, 1, 1), &ReadControl { interrupt: &interrupted, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        let output = context.render_wide_band(Rect::new(0, 0, 8, 1), &ctl()).unwrap();
        drop(context);
        drop(owner);
        assert_eq!(output.pixels().len(), 8);
    }
}
