//! Borrowed exact-F32 RGB -> Lab evaluation, preserving the legacy stage runner.
use super::{Core, Transform, TransformOptions};
use crate::control::{Budget, Error, Limits, Result, invalid};
pub use crate::pipeline::exact_f32::ExactF32Work;
use crate::pipeline::exact_f32::{self, RgbLabPlan};

#[derive(Clone, Copy, Debug)]
pub struct RgbLabF32Requirements {
    pub retained_bytes: usize,
    pub prepared_bytes: usize,
    pub source_bytes: usize,
    pub output_bytes: usize,
    pub scratch_bytes: usize,
    pub work: ExactF32Work,
    pub work_units: u64,
}
/// The original immutable Transform must outlive every call. This exposes no
/// profile/source grant and owns no new profile/table/native storage.
pub struct PreparedRgbLabF32<'a> {
    transform: &'a Transform,
    plan: RgbLabPlan,
    retained: usize,
}
impl Transform {
    /// Caller prepays this metadata-only ceiling on its cumulative work counter.
    /// New CMS local limits remain lower-only. Provider/profile construction is
    /// separate and is neither repeated nor credited by this preparation.
    pub const RGB_LAB_F32_PREPARATION_WORK: u64 = exact_f32::PREPARATION_WORK;
    pub fn prepare_rgb_lab_f32_with_control(&self, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<PreparedRgbLabF32<'_>> {
        let mut budget = Budget::new(limits, cancelled)?;
        budget.work(Self::RGB_LAB_F32_PREPARATION_WORK)?;
        if self.inputs != 3
            || self.outputs != 3
            || self.opts != TransformOptions::default()
            || !matches!(self.core, Core::Exact)
            || self.post.is_some()
            || !self.grid8.is_empty()
        {
            return Err(invalid("exact F32 default RGB Lab Transform required"));
        }
        let plan = exact_f32::prepare_rgb_lab(&self.pipeline, &budget)?;
        // Closed shape first: these shared helpers now inspect at most four
        // stage descriptors and three curve capacities, never CLUT/table values.
        let retained = self.retained_bytes_with_control(limits, cancelled)?;
        budget.admit(
            retained
                .checked_add(exact_f32::SCRATCH_BYTES)
                .and_then(|n| n.checked_add(std::mem::size_of::<PreparedRgbLabF32<'_>>()))
                .ok_or(Error::Limit("exact F32 retained storage"))?,
            0,
        )?;
        budget.poll()?;
        Ok(PreparedRgbLabF32 { transform: self, plan, retained })
    }
}
impl PreparedRgbLabF32<'_> {
    fn requirements(&self, pixels: usize, alpha: bool, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<RgbLabF32Requirements> {
        let budget = Budget::new(limits, cancelled)?;
        let channels = if alpha { 4usize } else { 3 };
        let bytes = pixels.checked_mul(channels).and_then(|n| n.checked_mul(4)).ok_or(Error::Limit("exact F32 buffer bytes"))?;
        let mut pixel = self.plan.pixel_work();
        if alpha {
            pixel.data_moves = pixel.data_moves.checked_add(2).ok_or(Error::Limit("exact F32 alpha work"))?;
        }
        let work = pixel.checked_mul(u64::try_from(pixels).map_err(|_| Error::Limit("exact F32 pixel count"))?)?;
        let work_units = work.total_units()?;
        let prepared_bytes = std::mem::size_of::<Self>();
        let peak = self
            .retained
            .checked_add(bytes)
            .and_then(|n| n.checked_add(bytes))
            .and_then(|n| n.checked_add(exact_f32::SCRATCH_BYTES))
            .and_then(|n| n.checked_add(prepared_bytes))
            .ok_or(Error::Limit("exact F32 peak storage"))?;
        budget.admit(peak, work_units)?;
        Ok(RgbLabF32Requirements {
            retained_bytes: self.retained,
            prepared_bytes,
            source_bytes: bytes,
            output_bytes: bytes,
            scratch_bytes: exact_f32::SCRATCH_BYTES,
            work,
            work_units,
        })
    }
    /// Sum of logical scalar/data/table primitives plus named provider CALLS,
    /// never cycles/libm-interior work. Caller prepays this exact constructor-
    /// derived schedule once before evaluation and passes its local sublimits.
    /// Borrowed spans are included, but enclosing Vec capacities remain caller-held.
    pub fn requirements_rgb(&self, pixels: usize, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<RgbLabF32Requirements> {
        self.requirements(pixels, false, limits, cancelled)
    }
    pub fn requirements_rgba(&self, pixels: usize, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<RgbLabF32Requirements> {
        self.requirements(pixels, true, limits, cancelled)
    }
    pub fn eval_rgb_with_control(&self, input: [f32; 3], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<[f32; 3]> {
        self.requirements_rgb(1, limits, cancelled)?;
        let mut budget = Budget::new(limits, cancelled)?;
        let mut out = [0.; 3];
        exact_f32::eval_rgb_lab(&self.transform.pipeline, &self.plan, &input, &mut out, &mut budget)?;
        Ok(out)
    }
    /// Output may contain a private completed prefix on cancellation; discard it
    /// on Err. No output allocation, source mutation, or finite-policy change.
    pub fn convert_rgb_with_control(&self, source: &[[f32; 3]], output: &mut [[f32; 3]], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<()> {
        if source.len() != output.len() {
            return Err(invalid("exact F32 RGB buffer lengths"));
        }
        self.requirements_rgb(source.len(), limits, cancelled)?;
        let mut budget = Budget::new(limits, cancelled)?;
        for (input, out) in source.iter().zip(output) {
            exact_f32::eval_rgb_lab(&self.transform.pipeline, &self.plan, input, out, &mut budget)?;
        }
        budget.poll()
    }
    pub fn convert_rgba_with_control(&self, source: &[[f32; 4]], output: &mut [[f32; 4]], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<()> {
        if source.len() != output.len() {
            return Err(invalid("exact F32 RGBA buffer lengths"));
        }
        self.requirements_rgba(source.len(), limits, cancelled)?;
        let mut budget = Budget::new(limits, cancelled)?;
        for (input, out) in source.iter().zip(output) {
            let rgb: &[f32; 3] =
                input.get(..3).ok_or(Error::Limit("exact F32 RGBA input"))?.try_into().map_err(|_| Error::Limit("exact F32 RGB input shape"))?;
            let lab: &mut [f32; 3] =
                out.get_mut(..3).ok_or(Error::Limit("exact F32 RGBA output"))?.try_into().map_err(|_| Error::Limit("exact F32 Lab output shape"))?;
            exact_f32::eval_rgb_lab(&self.transform.pipeline, &self.plan, rgb, lab, &mut budget)?;
            budget.work(2)?; // raw alpha read/write, after the pipeline as before
            *out.get_mut(3).ok_or(Error::Limit("exact F32 alpha output"))? = *input.get(3).ok_or(Error::Limit("exact F32 alpha input"))?;
        }
        budget.poll()
    }
}

#[cfg(test)]
mod tests {
    //! UNRUN drafts. No test execution/compilation was authorized for this slice.
    use super::*;
    use crate::{
        Builtin, Curve,
        pipeline::{Pipeline, Stage},
    };
    use std::cell::Cell;

    fn pipeline(curve: Curve, identity_matrix: bool) -> Transform {
        let matrix = if identity_matrix { [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] } else { [[0.7, 0.2, 0.1], [0.1, 0.8, 0.1], [0.1, 0.1, 0.8]] };
        Transform::from_pipeline(
            Pipeline::new(3, vec![Stage::Curves(vec![curve; 3]), Stage::matrix3(&matrix), Stage::XyzToLab, Stage::EncodeLabV4]),
            TransformOptions::default(),
        )
    }
    fn bits(values: &[[f32; 4]]) -> Vec<[u32; 4]> {
        values.iter().map(|v| v.map(f32::to_bits)).collect()
    }
    #[test]
    fn actual_constructor_and_forward_curve_variants_match_legacy_words_and_alpha() {
        let builtin = Transform::new(Builtin::Srgb.profile(), Builtin::LabD50.profile(), crate::Intent::RelativeColorimetric, true).unwrap();
        let mut transforms = vec![builtin];
        for curve in [
            Curve::Identity,
            Curve::Gamma(2.2),
            Curve::Parametric { kind: 3, p: [2.4, 1. / 1.055, 0.055 / 1.055, 1. / 12.92, 0.04045, 0., 0.] },
            Curve::Table(vec![0., 0.1, 0.6, 1.]),
            Curve::Table64(vec![0., 0.10000000000000002, 0.6, 1.]),
        ] {
            for identity in [false, true] {
                transforms.push(pipeline(curve.clone(), identity));
            }
        }
        let source = [[-0., 0.125, 1., -0.], [0.04, 0.8, 0.5, f32::from_bits(0x7fc01234)], [-1., 2., 0., 0.25], [f32::MIN_POSITIVE, 0.1, 0.9, 1.]];
        for transform in &transforms {
            let plan = transform.prepare_rgb_lab_f32_with_control(Limits::default(), &|| false).unwrap();
            let mut expected = [[0.; 4]; 4];
            let mut actual = expected;
            transform.convert_f32(source.as_flattened(), 4, expected.as_flattened_mut(), 4, true);
            let req = plan.requirements_rgba(source.len(), Limits::default(), &|| false).unwrap();
            plan.convert_rgba_with_control(&source, &mut actual, Limits { work_units: req.work_units, ..Limits::default() }, &|| false).unwrap();
            assert_eq!(bits(&actual), bits(&expected));
            assert_eq!(req.work.total_units().unwrap(), req.work_units);
            assert_eq!(req.work.cbrt_calls, 3 * source.len() as u64);
        }
    }
    #[test]
    fn identity_and_matrix_elision_change_only_constructor_owned_schedule() {
        let omitted = pipeline(Curve::Identity, true);
        let matrix = pipeline(Curve::Identity, false);
        let curves = pipeline(Curve::Gamma(2.), false);
        assert_eq!(omitted.pipeline().stages.len(), 2);
        assert_eq!(matrix.pipeline().stages.len(), 3);
        assert_eq!(curves.pipeline().stages.len(), 4);
        let mut units = Vec::new();
        for transform in [&omitted, &matrix, &curves] {
            let plan = transform.prepare_rgb_lab_f32_with_control(Limits::default(), &|| false).unwrap();
            let req = plan.requirements_rgb(1, Limits::default(), &|| false).unwrap();
            units.push(req.work_units);
            assert_eq!(plan.eval_rgb_with_control([0.125, 0.5, 0.875], Limits::default(), &|| false).unwrap().map(f32::to_bits), {
                let mut old = [0.; 3];
                transform.eval(&[0.125, 0.5, 0.875], &mut old);
                old.map(f32::to_bits)
            });
        }
        assert!(units[0] < units[1] && units[1] < units[2]);
    }
    #[test]
    fn finite_coefficients_with_nonfinite_intermediates_keep_original_runner_policy() {
        // These finite public model coefficients overflow the f32 matrix result;
        // the old subsequent max/clamp behavior must not be replaced by the
        // other controlled evaluator's intermediate finite refusal.
        let mut profile = Builtin::Srgb.profile().clone();
        profile.matrix = Some([[1e300, 0., 0.], [0., 1e300, 0.], [0., 0., 1e300]]);
        let transform = Transform::new(&profile, Builtin::LabD50.profile(), crate::Intent::RelativeColorimetric, true).unwrap();
        let plan = transform.prepare_rgb_lab_f32_with_control(Limits::default(), &|| false).unwrap();
        for input in [[f32::MAX, f32::MAX, f32::MAX], [f32::NAN, -1., f32::INFINITY]] {
            let mut old = [0.; 3];
            transform.eval(&input, &mut old);
            assert_eq!(plan.eval_rgb_with_control(input, Limits::default(), &|| false).unwrap().map(f32::to_bits), old.map(f32::to_bits));
        }
    }
    #[test]
    fn lower_work_shape_overflow_and_cancellation_refuse_private_output() {
        let transform = pipeline(Curve::Gamma(2.2), false);
        assert!(
            transform
                .prepare_rgb_lab_f32_with_control(Limits { work_units: Transform::RGB_LAB_F32_PREPARATION_WORK - 1, ..Limits::default() }, &|| false)
                .is_err()
        );
        let plan = transform.prepare_rgb_lab_f32_with_control(Limits::default(), &|| false).unwrap();
        let source = [[0.1, 0.2, 0.3, 1.]; 2];
        let mut output = [[-9.; 4]; 2];
        let original = output;
        let req = plan.requirements_rgba(2, Limits::default(), &|| false).unwrap();
        assert!(plan.convert_rgba_with_control(&source, &mut output, Limits { work_units: req.work_units - 1, ..Limits::default() }, &|| false).is_err());
        assert_eq!(output, original);
        assert!(plan.requirements_rgba(usize::MAX, Limits::default(), &|| false).is_err());
        assert!(plan.convert_rgba_with_control(&source, &mut output[..1], Limits::default(), &|| false).is_err());
        let calls = Cell::new(0);
        assert!(matches!(
            plan.convert_rgba_with_control(&source, &mut output, Limits::default(), &|| {
                calls.set(calls.get() + 1);
                calls.get() > 7
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(source[0].map(f32::to_bits), [0.1, 0.2, 0.3, 1.].map(f32::to_bits));
    }
}
