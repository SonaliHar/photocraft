//! Ignored UNRUN geometry/custody contracts for the three explicitly released
//! pure-point families. Generated native stores are not larger GIMP witnesses.
use super::tests::{adopt, ctl, fixture as base_fixture};
use super::*;
use photocraft_doc::{
    BackedLayer, Size,
    xcf_filters::{ExactNumber, FilterArgument, OperationIdentity},
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
    reads: Arc<AtomicU64>,
    fail: Arc<AtomicBool>,
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
        let at = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = at.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn sample_pixel(x: u32, y: u32) -> [f64; 4] {
    [
        f64::from((x * 17 + y * 3) % 257) / 256.,
        f64::from((x * 7 + y * 19) % 257) / 256.,
        f64::from((x * 11 + y * 5) % 257) / 256.,
        match (x + y) % 5 {
            0 => -0.,
            1 => 0.,
            2 => 0.25,
            3 => 0.75,
            _ => f64::from_bits(1f64.to_bits() - 1),
        },
    ]
}
fn number(name: &str, v: f32) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) }
}
type Fixture = (Document, BackingQuota, Arc<dyn Any + Send + Sync>, Arc<AtomicU64>, Arc<AtomicBool>, &'static Profile, BackedRaster);
fn fixture(kind: usize, sample: SampleType, size: Size) -> Fixture {
    let (mut d, q, guard, _, basis, _) = base_fixture(0, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
    // Actual mask payload/map and changed fixture model admission precedes all
    // replacements. The source Vec has its independent fee below.
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mask_fee = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let guard: Arc<dyn Any + Send + Sync> = Arc::new((guard, index, mask_fee));
    d.size = size;
    let bounds = d.bounds();
    let format = d.pixel_format();
    let bpp = format.bytes_per_pixel();
    let count = size.area() as usize;
    let len = count.checked_mul(bpp).unwrap();
    let fee = q.reserve(QuotaKind::Resident, (len.checked_mul(2).unwrap() + 1024) as u64).unwrap();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).unwrap();
    assert!(bytes.capacity() <= 2 * len);
    bytes.resize(len, 0);
    for y in 0..size.height {
        for x in 0..size.width {
            let start = (y as usize * size.width as usize + x as usize) * bpp;
            for (channel, v) in sample_pixel(x, y).into_iter().enumerate() {
                photocraft_color::try_write_sample_f64(&mut bytes[start..start + bpp], sample, channel, v).unwrap();
            }
        }
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, &vec![0; bpp]).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let store = Arc::new(Store { descriptor, bytes, reads: reads.clone(), fail: fail.clone(), quota: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    d.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster.clone(), descriptor, &ctl()).unwrap());
    let layer = d.layers[0].id;
    let state = d.xcf_state.as_mut().unwrap();
    for item in &mut state.items {
        if item.item == ItemRef::Layer(layer) {
            item.raster_bounds = Some(bounds);
        } else {
            item.raster_bounds = Some(bounds);
            item.mask.as_mut().unwrap().bounds = bounds;
        }
    }
    state.filter_masks[0].bounds = bounds;
    state.filter_masks[0].surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
    let n = &mut state.filter_stacks[0].nodes[0];
    let (name, epoch, args) = match kind {
        0 => (
            "gimp:semi-flatten",
            crate::gimp_filters::SEMI_FLATTEN_EPOCH,
            vec![FilterArgument {
                name: "color".into(),
                value: FilterValue::Color {
                    components: [0.75f32, 0.25, 0.125, 0.375].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            }],
        ),
        1 => ("gegl:hue-chroma", crate::HUE_CHROMA_EPOCH, vec![number("hue", 31.25), number("chroma", 22.5), number("lightness", -12.5)]),
        _ => ("gegl:vibrance", crate::VIBRANCE_EPOCH, vec![number("vibrance", 25.), number("saturation", 0.25)]),
    };
    n.operation = OperationIdentity { name: name.into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    n.arguments = args;
    let p = n.composition.as_mut().unwrap();
    p.compatibility_epoch = epoch.into();
    p.stored_blend_space = i32::MIN;
    p.stored_composite_space = 17;
    p.stored_operator = i32::MAX;
    (d, q, guard, reads, fail, basis, raster)
}
fn native(p: [f64; 4], sample: SampleType) -> Vec<u8> {
    let mut b = vec![0; 4 * sample.bytes()];
    for (i, v) in p.into_iter().enumerate() {
        photocraft_color::try_write_sample_f64(&mut b, sample, i, v).unwrap();
    }
    b
}
fn mask_check(kind: usize, d: &Document, c: &Interrupt<'_>) -> crate::gimp_composite::Result<()> {
    let s = d.xcf_state.as_ref().unwrap();
    let n = &s.filter_stacks[0].nodes[0];
    let id = d.layers[0].id;
    match kind {
        0 => crate::gimp_semi_flatten::qualify_mask(s, n, id, d.depth, c),
        1 => crate::gimp_hue_chroma::qualify_mask(s, n, id, d.depth, c),
        _ => crate::gimp_vibrance::qualify_mask(s, n, id, d.depth, c),
    }
}
#[test]
#[ignore = "UNRUN cross-tile origin geometry/partition/native mapping; declared evaluator not producer oracle"]
fn every_family_and_depth_preserves_point_results_across_origin_dimensions() {
    for kind in 0..3 {
        for sample in [SampleType::F32, SampleType::F64] {
            for size in
                [Size::new(1, 1), Size::new(1, 257), Size::new(257, 1), Size::new(255, 256), Size::new(256, 257), Size::new(257, 256), Size::new(509, 263)]
            {
                let (d, q, guard, _, _, basis, _) = fixture(kind, sample, size);
                let owner = adopt(d, q, guard);
                let context = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits { max_rows: 3, max_pixels: 32 }, &ctl()).unwrap();
                context.validate_operative(&ctl()).unwrap();
                // Numeric equations have separate independent physical/lower reference
                // drafts. This standalone paid pixel path isolates geometry/reader joins.
                let scalar = PreparedStockPointPipeline::prepare(owner.document(), basis, owner.quota(), &ctl()).unwrap();
                for (x, y) in [(0, 0), (size.width - 1, size.height - 1), (size.width.min(257) - 1, size.height.min(257) - 1)] {
                    let bytes = native(sample_pixel(x, y), sample);
                    let expected = scalar.evaluate_native(&bytes, owner.document().pixel_format(), &ctl()).unwrap();
                    let r = Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1);
                    let actual = context.render_wide_band(r, &ctl()).unwrap();
                    assert_eq!(actual.pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
                }
                let width = size.width.min(16) as i32;
                let y = size.height as i32 - 1;
                let whole = context.render_wide_band(Rect::new(0, y, width, y + 1), &ctl()).unwrap();
                for x in (0..width).rev() {
                    let p = context.render_wide_band(Rect::new(x, y, x + 1, y + 1), &ctl()).unwrap();
                    assert_eq!(p.pixels()[0].map(f64::to_bits), whole.pixels()[x as usize].map(f64::to_bits));
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN full retained mask/default/padding/grid/cap proofs independent of zero contents"]
fn physical_multitile_zero_masks_preserve_bits_and_refuse_padding_and_foreign_tiles() {
    for kind in 0..3 {
        for sample in [SampleType::F32, SampleType::F64] {
            let (mut d, q, guard, _, _, basis, _) = fixture(kind, sample, Size::new(509, 263));
            let minus = if sample == SampleType::F32 { (-0f32).to_ne_bytes().to_vec() } else { (-0f64).to_ne_bytes().to_vec() };
            for (x, y) in [(0, 0), (256, 0), (0, 256), (256, 256)] {
                d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(x, y, x + 1, y + 1), &minus).unwrap();
            }
            assert_eq!(d.xcf_state.as_ref().unwrap().filter_masks[0].surface.tile_count(), 4);
            assert!(mask_check(kind, &d, &Interrupt::NONE).is_ok());
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let original = std::mem::replace(&mut d.xcf_state.as_mut().unwrap().filter_masks[0].surface, Surface::with_default(format, &[f32::NAN]));
            assert!(mask_check(kind, &d, &Interrupt::NONE).is_err());
            d.xcf_state.as_mut().unwrap().filter_masks[0].surface = original;
            let polls = AtomicU64::new(0);
            let stop = || polls.fetch_add(1, Ordering::Relaxed) > 8;
            assert!(mask_check(kind, &d, &Interrupt::cancel_only(&stop)).is_err());
            let mask = &mut d.xcf_state.as_mut().unwrap().filter_masks[0].surface;
            let bad = if sample == SampleType::F32 { f32::NAN.to_ne_bytes().to_vec() } else { f64::NAN.to_ne_bytes().to_vec() };
            mask.try_write_interleaved(Rect::new(511, 511, 512, 512), &bad).unwrap(); // retained boundary padding
            assert!(mask_check(kind, &d, &Interrupt::NONE).is_err());
            d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(511, 511, 512, 512), &minus).unwrap();
            d.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(512, 0, 513, 1), &minus).unwrap();
            assert!(mask_check(kind, &d, &Interrupt::NONE).is_err()); // zero tile outside grid
            // Existing actual L1 shape may reject before the renderer; both are honest refusals.
            if let Ok(owner) = AccountedDocument::new_readonly_point_storage(d, guard, q, &ctl()) {
                assert!(PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl()).is_err());
            }
        }
    }
    // Direct mask-component cap test only: no native/context adoption of this
    // much larger logical canvas is claimed. One shared zero tile plus actual
    // 4097 map entries fit under the already held 8 MiB fixture envelope.
    let (mut d, _q, _guard, _, _, _, _) = fixture(0, SampleType::F32, Size::new(1, 1));
    let extent = Rect::new(0, 0, 262144, 1280);
    let id = d.layers[0].id;
    let s = d.xcf_state.as_mut().unwrap();
    for item in &mut s.items {
        item.raster_bounds = Some(extent);
        if item.item != ItemRef::Layer(id) {
            item.mask.as_mut().unwrap().bounds = extent;
        }
    }
    let mask = &mut s.filter_masks[0];
    mask.bounds = extent;
    let tile = Arc::new(mask.surface.try_default_tile().unwrap());
    mask.surface.put_tiles((0..4097).map(|n| (photocraft_geom::TileCoord::new(n % 1024, n / 1024), tile.clone())));
    assert_eq!(mask.surface.tile_count(), 4097);
    assert!(mask_check(0, &d, &Interrupt::NONE).is_err());
    for r in [Rect::EMPTY, Rect::new(-1, 0, 256, 259), Rect::new(0, 0, 262145, 1), Rect::new(0, 0, 262144, 262144)] {
        assert!(crate::gimp_semi_flatten::positive_origin(r).is_err());
    }
    assert!(crate::gimp_semi_flatten::mask_tile(Rect::new(0, 0, 509, 263), photocraft_geom::TileCoord::new(i32::MAX, i32::MAX)).is_err());
}
#[test]
#[ignore = "UNRUN inactive full input proof, larger derived empty CURRENT and old empty/refusal boundaries"]
fn inactive_and_removed_filters_keep_full_proof_and_separate_empty_authority() {
    for kind in 0..3 {
        for sample in [SampleType::F32, SampleType::F64] {
            // Plain Document preparation must establish canonical/native
            // equality itself, without borrowing CURRENT L1 class authority.
            let (mut mismatch, q, _guard, reads, _, basis, _) = fixture(kind, sample, Size::new(257, 259));
            let wrong = Rect::new(0, 0, 1, 1);
            let state = mismatch.xcf_state.as_mut().unwrap();
            for item in &mut state.items {
                item.raster_bounds = Some(wrong);
                if let Some(mask) = &mut item.mask {
                    mask.bounds = wrong;
                }
            }
            state.filter_masks[0].bounds = wrong;
            let baseline = q.usage().unwrap();
            assert!(PreparedStockPointPipeline::prepare(&mismatch, basis, &q, &ctl()).is_err());
            assert_eq!(reads.load(Ordering::Relaxed), 0);
            assert_eq!(q.usage().unwrap(), baseline);
            for defect in 0..3 {
                let (mut d, q, guard, _, _, basis, _) = fixture(kind, sample, Size::new(257, 259));
                d.layers[0].visible = false;
                d.layers[0].opacity = 0.;
                let id = d.layers[0].id;
                let s = d.xcf_state.as_mut().unwrap();
                s.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
                s.filter_stacks[0].nodes[0].enabled = false;
                if defect == 0 {
                    let LayerContent::BackedRaster(b) = &d.layers[0].content else { unreachable!() };
                    d.layers[0].content = LayerContent::BackedRaster(
                        b.try_edited_native(Rect::new(256, 258, 257, 259), &native([f64::NAN, 0.25, 0.5, 0.5], sample), &ctl()).unwrap(),
                    );
                } else if defect == 1 {
                    d.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
                } else {
                    s.filter_masks[0].surface.try_write_region(Rect::new(511, 511, 512, 512), &[1.]).unwrap();
                }
                if let Ok(owner) = AccountedDocument::new_readonly_point_storage(d, guard, q, &ctl())
                    && let Ok(context) = PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl())
                {
                    assert!(context.validate_operative(&ctl()).is_err());
                }
            }
            let (mut d, q, guard, _, _, basis, _) = fixture(kind, sample, Size::new(257, 259));
            let id = d.layers[0].id;
            let s = d.xcf_state.as_mut().unwrap();
            s.filter_stacks[0].nodes.clear();
            s.filter_masks.clear();
            s.items.retain(|i| i.item == ItemRef::Layer(id));
            let owner = adopt(d, q, guard);
            let context = PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl()).unwrap();
            context.validate_operative(&ctl()).unwrap();
            assert!(context.node_id().is_none());
            let raw = context.render_wide_band(Rect::new(255, 258, 257, 259), &ctl()).unwrap();
            for (i, p) in raw.pixels().iter().enumerate() {
                let n = native(sample_pixel(255 + i as u32, 258), sample);
                let expected: [f64; 4] = std::array::from_fn(|c| try_read_sample_f64(&n, sample, c).unwrap());
                assert_eq!(p.map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
    let (mut d, q, guard, _, _, basis, _) = fixture(0, SampleType::U8, Size::new(257, 1));
    let id = d.layers[0].id;
    let s = d.xcf_state.as_mut().unwrap();
    s.filter_stacks[0].nodes.clear();
    s.filter_masks.clear();
    s.items.retain(|i| i.item == ItemRef::Layer(id));
    let owner = adopt(d, q, guard);
    assert!(PreparedBackedStockPointContext::new(&owner, basis, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN real source/preparation/cache/band ownership and cumulative work/cancel/typed IO cleanup"]
fn larger_native_custody_and_repeated_preparation_use_real_one_ledger_fees() {
    for kind in 0..3 {
        let (d, q, guard, reads, fail, basis, raster) = fixture(kind, SampleType::F64, Size::new(509, 263));
        let before = q.usage().unwrap();
        let scalar = PreparedStockPointPipeline::prepare(&d, basis, &q, &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        drop(scalar);
        assert_eq!(q.usage().unwrap(), before);
        let owner = adopt(d, q.clone(), guard);
        let context = PreparedBackedStockPointContext::new(&owner, basis, NormalBandLimits { max_rows: 2, max_pixels: 16 }, &ctl()).unwrap();
        context.validate_operative(&ctl()).unwrap();
        // Warm chosen tile before the refund baseline. Cache remains real and
        // separately paid; dropped pins do not imply cache eviction.
        drop(context.render_wide_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap());
        let baseline = q.usage().unwrap();
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(context.render_wide_band(Rect::new(0, 0, 1, 1), &denied), Err(BackedNormalError::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), baseline);
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(context.render_wide_band(Rect::new(0, 0, 1, 1), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
        raster.clear_cache().unwrap();
        let io_baseline = q.usage().unwrap();
        fail.store(true, Ordering::Relaxed);
        assert!(matches!(
            context.render_wide_band(Rect::new(508, 262, 509, 263), &ctl()),
            Err(BackedNormalError::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))
        ));
        assert_eq!(q.usage().unwrap(), io_baseline);
        fail.store(false, Ordering::Relaxed);
        let output = context.render_wide_band(Rect::new(0, 0, 8, 1), &ctl()).unwrap();
        drop(context);
        drop(owner);
        assert_eq!(output.pixels().len(), 8);
        drop(output);
    }
}
