//! UNRUN synthetic CURRENT tests. Harness buffers are not source evidence or
//! production allocation receipts. Actual native stores/context owners are paid.
use super::*;
use crate::backed_stock_point_context::tests::{adopt, ctl, fixture};
use photocraft_doc::{
    BackedLayer, Size,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore};
use std::sync::Arc;
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    pixel: [u8; 40],
    q: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        if offset.checked_add(out.len() as u64).ok_or(BackingError::Bounds)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        for (i, block) in out.chunks_mut(4096).enumerate() {
            c.check(block.len() as u64)?;
            for (j, v) in block.iter_mut().enumerate() {
                *v = *self.pixel.get((offset as usize + i * 4096 + j) % self.descriptor.format().bytes_per_pixel()).ok_or(BackingError::Bounds)?;
            }
        }
        Ok(())
    }
}
fn arg(name: &str, value: FilterValue) -> FilterArgument {
    FilterArgument { name: name.into(), value }
}
fn f(name: &str, value: f32) -> FilterArgument {
    arg(name, FilterValue::Number(ExactNumber::Float32Bits(value.to_bits())))
}
fn i(name: &str, value: i64) -> FilterArgument {
    arg(name, FilterValue::Number(ExactNumber::Signed { width: 32, value }))
}
fn e(name: &str, domain: &str, value: i32) -> FilterArgument {
    arg(name, FilterValue::Enum { domain: domain.into(), value })
}
fn model(kind: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile, Surface) {
    let finite = kind >= 4;
    let gray = kind == 6;
    let mode = if gray { ColorMode::Grayscale } else { ColorMode::Rgb };
    let sample = if finite { SampleType::F64 } else { SampleType::U8 };
    let (mut doc, q, guard, _, profile, _) = fixture(2, mode, sample, SampleEncoding::NonlinearProfile);
    let bounds = if finite { Rect::new(0, 0, 16, 12) } else { Rect::new(0, 0, 32, 32) };
    doc.size = Size::new(bounds.width(), bounds.height());
    let format = doc.pixel_format();
    let mut pixel = [0; 40];
    for c in 0..mode.color_channels() {
        photocraft_color::try_write_sample_f64(&mut pixel, sample, c, if finite { -0.125 + c as f64 * 0.75 } else { 0.25 + c as f64 * 0.25 }).unwrap();
    }
    photocraft_color::try_write_sample_f64(&mut pixel, sample, mode.color_channels(), if finite { 0.5 } else { 1.0 }).unwrap();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 40][..format.bytes_per_pixel()]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, std::mem::size_of::<Store>() as u64 + 128).unwrap();
    let store = Arc::new(Store { descriptor, pixel, q: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let mut old = Surface::try_with_default_bytes(format, &[0; 40][..format.bytes_per_pixel()]).unwrap();
    let bytes = (0..u64::from(bounds.width()) * u64::from(bounds.height())).flat_map(|_| pixel[..format.bytes_per_pixel()].iter().copied()).collect::<Vec<_>>();
    old.try_write_interleaved(bounds, &bytes).unwrap();
    let layer = doc.layers[0].id;
    let state = doc.xcf_state.as_mut().unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(bounds);
        if let Some(mask) = &mut item.mask {
            mask.bounds = bounds
        }
    }
    state.filter_masks[0].bounds = bounds;
    let (name, epoch, args) = match kind {
        0 | 4 => ("gegl:box-blur", "xcf22-rgb-box-replace-v1", vec![i("radius", 4)]),
        1 | 5 | 6 => (
            "gegl:median-blur",
            if kind == 1 {
                "xcf22-rgb-u8-quantized-median-v1"
            } else if gray {
                "xcf22-gray-alpha-median-v1"
            } else {
                "xcf22-rgb-alpha-median-v1"
            },
            vec![
                i("radius", 3),
                e("neighborhood", "gegl:median-blur::neighborhood", 1),
                f("percentile", 50.),
                f("alpha-percentile", 50.),
                e("abyss-policy", "gegl:median-blur::abyss-policy", 1),
                arg("high-precision", FilterValue::Bool(false)),
            ],
        ),
        2 => ("gegl:motion-blur-linear", "xcf22-rgb-u8-linear-motion-v1", vec![f("length", 10.), f("angle", 0.)]),
        3 => (
            "gegl:pixelize",
            "xcf22-rgb-u8-square-pixelize-v1",
            vec![
                e("norm", "gegl:pixelize::norm", 2),
                i("size-x", 16),
                i("size-y", 16),
                i("offset-x", 0),
                i("offset-y", 0),
                f("ratio-x", 1.),
                f("ratio-y", 1.),
                arg(
                    "background",
                    FilterValue::Color {
                        components: vec![ExactNumber::Float32Bits(1f32.to_bits()); 4],
                        encoding: SampleEncoding::NonlinearProfile,
                        profile: None,
                    },
                ),
            ],
        ),
        _ => (
            "gegl:gaussian-blur",
            "xcf22-rgb-alpha-finite-gaussian-v1",
            vec![
                f("std-dev-x", 2.25),
                f("std-dev-y", 1.3),
                e("filter", "gegl:gaussian-blur::filter", 0),
                e("abyss-policy", "gegl:gaussian-blur::abyss-policy", 1),
                arg("clip-extent", FilterValue::Bool(true)),
            ],
        ),
    };
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation.name = name.into();
    node.operation.stored_version = "0:0".into();
    node.operation.schema_epoch = epoch.into();
    node.arguments = args;
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = epoch.into();
    let raw = if finite { -1474592175 } else { -261651887 };
    policy.stored_blend_space = raw;
    policy.stored_composite_space = raw;
    policy.stored_operator = raw;
    assert_eq!(state.filter_stacks[0].owner, ItemRef::Layer(layer));
    (doc, q, guard, profile, old)
}
#[test]
fn all_eight_paid_neighborhoods_match_old_evaluate_and_preserve_expanded_bounds() {
    for kind in 0..8 {
        let (doc, q, guard, profile, source) = model(kind);
        let owner = adopt(doc, q, guard);
        let current = PreparedBackedNeighborhoodContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
        current.validate_operative(&ctl()).unwrap();
        let state = owner.document().xcf_state.as_ref().unwrap();
        let legacy = OrderedStack::new_with_interrupt(
            &state.filter_stacks[0],
            profile,
            SampleEncoding::NonlinearProfile,
            22,
            owner.document().bounds(),
            [true, false, false],
            &Interrupt::NONE,
        )
        .unwrap();
        let expected = legacy.evaluate(&source, current.bounds(), &mut |_| Ok(()), &Interrupt::NONE).unwrap();
        let output = current.render_wide_band(current.bounds(), &ctl()).unwrap();
        let mut n = 0;
        for y in current.bounds().y0..current.bounds().y1 {
            for x in current.bounds().x0..current.bounds().x1 {
                let mut p = [0.; 4];
                expected.try_read_pixel_f64(x, y, &mut p).unwrap();
                assert_eq!(output.pixels()[n].map(f64::to_bits), p.map(f64::to_bits));
                n += 1;
            }
        }
        if kind == 0 || kind == 2 {
            assert!(current.bounds().x0 < 0);
            assert!(current.bounds().width() > 32);
        }
    }
}
#[test]
fn current_shape_profile_activation_and_actual_work_refusals_are_private() {
    let (doc, q, guard, profile, _) = model(0);
    let owner = adopt(doc, q.clone(), guard);
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), before);
    for case in 0..3 {
        let (mut doc, q, _guard, profile, _) = model(0);
        match case {
            0 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "99:0".into(),
            1 => doc.source_color_context.as_mut().unwrap().source_precision = 250,
            _ => doc.sample_encoding = Some(SampleEncoding::Linear),
        };
        let before = q.usage().unwrap();
        assert!(prepare(&doc, Some(profile), 512, &Interrupt::NONE, &mut NativeBudget { q: &q, c: &ctl() }).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn owned_retained_outputs_hold_real_quota_until_drop() {
    let (doc, q, guard, profile, _) = model(2);
    let owner = adopt(doc, q.clone(), guard);
    // Genuine source pins may populate the retained native cache. Warm using
    // the actual recipe and let its temporary owners drop before baseline.
    {
        let warm = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        warm.validate_operative(&ctl()).unwrap();
    }
    let before = q.usage().unwrap();
    {
        let context = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        let held = q.usage().unwrap();
        assert!(held.resident > before.resident);
        let output = context.render_wide_band(context.bounds(), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        drop(output);
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), before);
}

#[path = "backed_neighborhood_disabled_tests.rs"]
mod disabled;

#[path = "backed_original_gaussian_tests.rs"]
mod original_gaussian;

#[path = "backed_neighborhood_assigned_tests.rs"]
mod assigned;
