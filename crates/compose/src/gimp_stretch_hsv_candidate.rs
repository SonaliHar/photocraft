//! Opt-in retained zero-argument HSV stretch consumer. Complete immutable source
//! extrema are prepared once. Explicit nonlinear/profile/policy witnesses are
//! caller declarations, not source, ICC, mask, outer or default-renderer grants.
//! The lease holds known source/kernel/header backing; the enclosing caller must
//! also retain/admit all original decoder/AST/profile/pruned graph owners.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
pub use math::NumericPolicy;
use photocraft_algo::gimp::{global_stretch_hsv_candidate as math, wide_point::WorkingContext};
use photocraft_doc::{SampleEncoding, xcf_filters::FilterNode};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R, Surface};
use std::sync::atomic::{AtomicU64, Ordering};
pub const SCHEMA: &str = "862de99c547cf3bbb25f77204bb93dcd1becf318ad65cce1d645bdafd7bd1a85";
const HEADER: u64 = 4096;
const BIND_WORK: u64 = 128;
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub context: WorkingContext,
    pub policy: NumericPolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub retained_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub preparation_work_units: u64,
    pub sample_work_units: u64,
    pub remaining_work_units: u64,
}
pub struct Prepared<'a, L> {
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    kernel: math::PreparedStretchHsv<'a>,
    requirements: Requirements,
    limits: Limits,
    work: AtomicU64,
    _lease: L,
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| R::Cancelled.into())
}
fn limit() -> E {
    R::LimitExceeded.into()
}
fn bad(s: &'static str) -> E {
    E::Unsupported(s)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn lower(e: math::Error) -> E {
    match e {
        math::Error::Raster(r) => r.into(),
        math::Error::NonFinite | math::Error::OutsideUnit => R::InvalidSample.into(),
        _ => bad("declared HSV stretch context/format"),
    }
}
fn bind(node: &FilterNode, b: SourceBinding<'_>, c: &Interrupt<'_>) -> Result<()> {
    poll(c)?;
    if b.xcf_version != 22
        || b.schema_sha256 != SCHEMA
        || b.expected_epoch.is_empty()
        || b.expected_epoch.len() > 128
        || b.expected_epoch.contains('\0')
        || node.id.0 == 0
        || node.operation.name != "gegl:stretch-contrast-hsv"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != b.expected_epoch
        || !node.arguments.is_empty()
        || !node.auxiliary.is_empty()
        || b.context.encoding != SampleEncoding::NonlinearProfile
        || b.context.profile_identity == [0; 32]
    {
        return Err(bad("exact zero-argument HSV stretch source/schema/context"));
    }
    Ok(())
}
/// O(1), allocation-free diagnostic census. Preparation below debits this same
/// aggregate before the lower full-source reduction or retained construction.
pub fn requirements(node: &FilterNode, b: SourceBinding<'_>, source: &Surface, extent: Rect, l: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    let m = Limits::default();
    if l.resident_bytes > m.resident_bytes || l.work_units > m.work_units {
        return Err(limit());
    }
    bind(node, b, c)?;
    let q = math::preparation_requirements(source, extent, b.context, b.policy).map_err(lower)?;
    let retained = add(HEADER, q.retained_bytes)?;
    let peak = add(HEADER, q.peak_resident_bytes)?;
    let work = add(BIND_WORK, q.work_units)?;
    if peak > l.resident_bytes || work > l.work_units {
        return Err(limit());
    }
    Ok(Requirements {
        retained_bytes: retained,
        peak_resident_bytes: peak,
        index_bytes: q.index_bytes,
        preparation_work_units: work,
        sample_work_units: math::SAMPLE_WORK,
        remaining_work_units: l.work_units - work,
    })
}
pub fn prepare<'a, B: PreparationBudget>(
    node: &'a FilterNode,
    b: SourceBinding<'a>,
    source: &'a Surface,
    extent: Rect,
    l: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<Prepared<'a, B::Lease>> {
    poll(c)?;
    if std::mem::size_of::<Prepared<'a, B::Lease>>() as u64 > HEADER {
        return Err(limit());
    }
    let q = requirements(node, b, source, extent, l, c)?;
    budget.consume_preparation_work(q.preparation_work_units)?;
    let lease = budget.reserve_prepared_bytes(q.peak_resident_bytes)?;
    let lower_q = math::preparation_requirements(source, extent, b.context, b.policy).map_err(lower)?;
    let kernel = math::PreparedStretchHsv::prepare(
        source,
        extent,
        b.context,
        b.policy,
        math::Limits { resident_bytes: lower_q.peak_resident_bytes, work_units: lower_q.work_units },
        c,
    )
    .map_err(lower)?;
    poll(c)?;
    Ok(Prepared { node, binding: b, kernel, requirements: q, limits: l, work: AtomicU64::new(q.preparation_work_units), _lease: lease })
}
impl<'a, L> Prepared<'a, L> {
    pub fn original_node(&self) -> &'a FilterNode {
        self.node
    }
    pub fn source(&self) -> &'a Surface {
        self.kernel.source()
    }
    pub fn full_extent(&self) -> Rect {
        self.kernel.extent()
    }
    pub fn context(&self) -> WorkingContext {
        self.kernel.context()
    }
    pub fn policy(&self) -> NumericPolicy {
        self.kernel.policy()
    }
    pub fn format(&self) -> photocraft_color::PixelFormat {
        self.source().format()
    }
    pub fn binding(&self) -> SourceBinding<'a> {
        self.binding
    }
    pub fn extrema(&self) -> ([f64; 2], [f64; 2]) {
        self.kernel.extrema()
    }
    pub fn requirements(&self) -> Requirements {
        let mut q = self.requirements;
        q.remaining_work_units = self.limits.work_units.saturating_sub(self.work.load(Ordering::Relaxed));
        q
    }
    /// Actual tuple calls (preflight and render alike) consume the immutable quota.
    /// Concurrent prospective checks are not reservations; exhaustion fails privately.
    pub fn evaluate_rgb_with_interrupt(&self, input: [f32; 3], actual: WorkingContext, c: &Interrupt<'_>) -> Result<[f32; 3]> {
        poll(c)?;
        if actual != self.context() {
            return Err(bad("HSV stretch context changed"));
        }
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(self.kernel.sample_work()).filter(|n| *n <= self.limits.work_units))
            .map_err(|_| limit())?;
        let out = self.kernel.try_eval_float_color(input).map_err(lower)?;
        poll(c)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{item_identity::FilterId, xcf_filters::OperationIdentity};
    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
        deny: bool,
    }
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<()> {
            self.bytes = n;
            if self.deny {
                return Err(limit());
            }
            Ok(())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            self.work = n;
            Ok(())
        }
    }
    pub(super) fn node() -> FilterNode {
        FilterNode {
            id: FilterId(7),
            operation: OperationIdentity { name: "gegl:stretch-contrast-hsv".into(), stored_version: "0:0".into(), schema_epoch: "explicit-hsv".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    fn binding() -> SourceBinding<'static> {
        SourceBinding {
            xcf_version: 22,
            expected_epoch: "explicit-hsv",
            schema_sha256: SCHEMA,
            context: WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [9; 32] },
            policy: NumericPolicy::DeclaredBinary64HsvFinalFloat,
        }
    }
    #[test]
    fn full_extent_statistics_once_and_each_tuple_debits_shared_quota() {
        let n = node();
        let before = n.clone();
        let mut input = Surface::new(PixelFormat::RGBA32F);
        let raw: Vec<u8> = [0.25f32.to_bits(), 0.25f32.to_bits(), 0.25f32.to_bits(), 0x7fc00042, 0.75f32.to_bits(), 0.375f32.to_bits(), 0.375f32.to_bits(), 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let extent = Rect::new(-1, 0, 1, 1);
        input.try_write_interleaved(extent, &raw).unwrap();
        let mut budget = Budget::default();
        let prepared = prepare(&n, binding(), &input, extent, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        assert!(std::ptr::eq(prepared.source(), &input));
        assert_eq!(prepared.extrema(), ([0., 0.25], [0.5, 0.5]));
        let left = prepared.requirements().remaining_work_units;
        assert_eq!(prepared.evaluate_rgb_with_interrupt([0.25; 3], binding().context, &Interrupt::NONE).unwrap(), [0.; 3]);
        prepared.evaluate_rgb_with_interrupt([0.75, 0.375, 0.375], binding().context, &Interrupt::NONE).unwrap();
        assert_eq!(prepared.requirements().remaining_work_units, left - 512);
        assert_eq!(n, before);
        let mut after = [0; 32];
        input.try_read_interleaved_into_with(extent, &mut after, &Interrupt::NONE).unwrap();
        assert_eq!(after.as_slice(), raw.as_slice());
    }
    #[test]
    fn denied_shared_budget_precedes_full_source_scan() {
        let n = node();
        let input = Surface::new(PixelFormat::RGBA32F);
        let mut budget = Budget { deny: true, ..Budget::default() };
        assert!(prepare(&n, binding(), &input, Rect::new(0, 0, 1, 1), Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert!(budget.work > 0 && budget.bytes > 0);
        let stop = || true;
        let mut budget = Budget::default();
        assert!(matches!(
            prepare(&n, binding(), &input, Rect::new(0, 0, 1, 1), Limits::default(), &mut budget, &Interrupt::cancel_only(&stop)),
            Err(E::Raster(R::Cancelled))
        ));
        assert_eq!(budget.work, 0);
    }
    #[test]
    fn schema_argument_and_native_wide_refusals_do_not_reserve() {
        let mut n = node();
        n.arguments.push(photocraft_doc::xcf_filters::FilterArgument { name: "invented".into(), value: photocraft_doc::xcf_filters::FilterValue::Bool(false) });
        let input = Surface::new(PixelFormat::RGBA32F);
        let mut budget = Budget::default();
        assert!(prepare(&n, binding(), &input, Rect::new(0, 0, 1, 1), Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.work, 0);
        let n = node();
        let wide = Surface::new(photocraft_color::PixelFormat { sample: photocraft_color::SampleType::F64, ..PixelFormat::RGBA32F });
        assert!(prepare(&n, binding(), &wide, Rect::new(0, 0, 1, 1), Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
    }
}
