//! Additive checked cooperative F64 one-pixel evaluation. Old evaluation bodies
//! remain literal. Borrowed pipeline storage is not newly allocated or credited.
use super::*;
use crate::control::{Budget, Error, Limits, Result, invalid};
#[derive(Clone, Copy, Debug)]
pub struct F64EvaluationRequirements {
    /// Complete evaluation, including its repeated bounded shape inventory.
    pub work_units: u64,
    /// Work used by this independent requirements phase (not later credit).
    pub planning_work_units: u64,
    /// New fixed stack/scratch envelope only; borrowed pipeline remains owned.
    pub peak_resident_bytes: usize,
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Limit("wide evaluation work"))
}
fn stage_work(stage: &Stage) -> Result<u64> {
    Ok(match stage {
        Stage::Curves(cs) => (cs.len() as u64).checked_mul(128).ok_or(Error::Limit("wide curve work"))?,
        Stage::InvCurves(cs) => cs.iter().try_fold(0u64, |n, c| {
            add(
                n,
                match c {
                    Curve::Table(t) => (t.len() as u64).checked_add(128).ok_or(Error::Limit("wide inverse work"))?,
                    Curve::Table64(t) => (t.len() as u64).checked_add(128).ok_or(Error::Limit("wide inverse work"))?,
                    _ => 128,
                },
            )
        })?,
        Stage::Clut(c) => {
            let visits = c.interpolation_taps()?;
            visits.checked_mul((c.inputs + c.outputs + 1) as u64).ok_or(Error::Limit("wide CLUT work"))?
        }
        Stage::Matrix { rows, cols, .. } => {
            (*rows as u64).checked_mul(*cols as u64).and_then(|n| n.checked_add(128)).ok_or(Error::Limit("wide matrix work"))?
        }
        _ => 128,
    })
}
fn requirements(p: &Pipeline, b: &mut Budget<'_>) -> Result<F64EvaluationRequirements> {
    // O(1) dimensions first, with original cancellation before every traversal.
    b.work(128)?;
    if p.inputs == 0 || p.inputs > MAX || p.outputs == 0 || p.outputs > MAX || p.stages.len() > 4096 {
        return Err(invalid("wide pipeline dimensions/stage count"));
    }
    let planning = add(128, (p.stages.len() as u64).checked_mul(64).ok_or(Error::Limit("wide planning work"))?)?;
    let mut work = 1u64;
    let mut channels = p.inputs;
    for stage in &p.stages {
        b.work(64)?;
        stage.controlled_bytes()?;
        match stage {
            Stage::Curves(cs) | Stage::InvCurves(cs) if cs.len() != channels => return Err(invalid("wide curve dimensions")),
            Stage::Matrix { cols, .. } if *cols != channels => return Err(invalid("wide matrix dimensions")),
            Stage::Clut(c) if c.inputs != channels => return Err(invalid("wide CLUT dimensions")),
            Stage::LabToXyz
            | Stage::XyzToLab
            | Stage::DecodeLabV4
            | Stage::EncodeLabV4
            | Stage::DecodeLabV2
            | Stage::EncodeLabV2
            | Stage::DecodeXyz
            | Stage::EncodeXyz
                if channels != 3 =>
            {
                return Err(invalid("wide PCS dimensions"));
            }
            _ => {}
        }
        channels = stage.outputs(channels);
        if channels == 0 || channels > MAX {
            return Err(invalid("wide stage output dimensions"));
        }
        work = add(work, add(64, stage_work(stage)?)?)?;
    }
    if channels != p.outputs {
        return Err(invalid("wide output dimensions"));
    }
    let total = add(planning, work)?;
    b.admit(4096, work)?;
    Ok(F64EvaluationRequirements { work_units: total, planning_work_units: planning, peak_resident_bytes: 4096 })
}
impl Pipeline {
    /// Prepay this bounded requirements inventory on the caller's original meter.
    pub const fn eval_f64_planning_work_ceiling() -> u64 {
        128 + 4096 * 64
    }
    pub fn eval_f64_requirements_with_control(&self, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<F64EvaluationRequirements> {
        let mut b = Budget::new(limits, cancelled)?;
        requirements(self, &mut b)
    }
    pub fn try_eval_f64_with_control(&self, input: &[f64], out: &mut [f64], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let mut budget = Budget::new(limits, cancelled)?;
        let req = requirements(self, &mut budget)?;
        budget.retained(req.peak_resident_bytes)?;
        let invalid = || crate::control::invalid("invalid wide ICC pipeline dimensions or nonfinite samples");
        if self.inputs == 0
            || self.inputs > MAX
            || self.outputs > MAX
            || input.len() != self.inputs
            || out.len() != self.outputs
            || input.iter().any(|v| !v.is_finite())
        {
            return Err(invalid());
        }
        let mut v = [0.0_f64; MAX];
        v.get_mut(..self.inputs).ok_or_else(invalid)?.copy_from_slice(input);
        let mut n = self.inputs;
        for stage in &self.stages {
            budget.work(64)?;
            match stage {
                Stage::Curves(curves) | Stage::InvCurves(curves) => {
                    if curves.len() != n {
                        return Err(invalid());
                    }
                    for (sample, curve) in v.get_mut(..n).ok_or_else(invalid)?.iter_mut().zip(curves) {
                        *sample = if matches!(stage, Stage::Curves(_)) {
                            budget.work(128)?;
                            curve.eval64(*sample)
                        } else {
                            curve.inverse64_controlled(*sample, &mut budget)?
                        };
                    }
                }
                Stage::Matrix { rows, cols, m, offset } => {
                    budget.work((*rows as u64).checked_mul(*cols as u64).and_then(|v| v.checked_add(128)).ok_or(Error::Limit("wide matrix work"))?)?;
                    if *rows == 0 || *rows > MAX || *cols != n || m.len() != rows.checked_mul(*cols).ok_or_else(invalid)? || offset.len() != *rows {
                        return Err(invalid());
                    }
                    let mut values = [0.0_f64; MAX];
                    for (row, target) in values.get_mut(..*rows).ok_or_else(invalid)?.iter_mut().enumerate() {
                        budget.poll()?;
                        let mut value = *offset.get(row).ok_or_else(invalid)?;
                        for (column, &sample) in v.get(..n).ok_or_else(invalid)?.iter().enumerate() {
                            let i = row.checked_mul(*cols).and_then(|i| i.checked_add(column)).ok_or_else(invalid)?;
                            value += *m.get(i).ok_or_else(invalid)? * sample;
                        }
                        *target = value;
                    }
                    v = values;
                    n = *rows;
                }
                Stage::Clut(clut) => {
                    if n != clut.inputs || clut.outputs > MAX {
                        return Err(invalid());
                    }
                    let mut output = [0.0_f64; MAX];
                    clut.eval_f64_controlled(v.get(..n).ok_or_else(invalid)?, output.get_mut(..clut.outputs).ok_or_else(invalid)?, &mut budget)?;
                    v = output;
                    n = clut.outputs;
                }
                Stage::Clamp01 => {
                    budget.work(128)?;
                    for sample in v.get_mut(..n).ok_or_else(invalid)? {
                        *sample = sample.clamp(0.0, 1.0);
                    }
                }
                _ => {
                    budget.work(128)?;
                    if n != 3 {
                        return Err(invalid());
                    }
                    let first: [f64; 3] = v.get(..3).ok_or_else(invalid)?.try_into().map_err(|_| invalid())?;
                    let [x, y, z] = first;
                    let xyz_scale = 65535.0_f64 / 32768.0;
                    let v2_l = 100.0_f64 * 65535.0 / 65280.0;
                    let v2_ab = 255.0_f64 * 65535.0 / 65280.0;
                    let result = match stage {
                        Stage::LabToXyz => math::lab_to_xyz(first, math::D50),
                        Stage::XyzToLab => math::xyz_to_lab([x.max(0.0), y.max(0.0), z.max(0.0)], math::D50),
                        Stage::DecodeLabV4 => [x * 100.0, y * 255.0 - 128.0, z * 255.0 - 128.0],
                        Stage::EncodeLabV4 => [(x / 100.0).clamp(0.0, 1.0), ((y + 128.0) / 255.0).clamp(0.0, 1.0), ((z + 128.0) / 255.0).clamp(0.0, 1.0)],
                        Stage::DecodeLabV2 => [x * v2_l, y * v2_ab - 128.0, z * v2_ab - 128.0],
                        Stage::EncodeLabV2 => [(x / v2_l).clamp(0.0, 1.0), ((y + 128.0) / v2_ab).clamp(0.0, 1.0), ((z + 128.0) / v2_ab).clamp(0.0, 1.0)],
                        Stage::DecodeXyz => [x * xyz_scale, y * xyz_scale, z * xyz_scale],
                        Stage::EncodeXyz => [(x / xyz_scale).clamp(0.0, 1.0), (y / xyz_scale).clamp(0.0, 1.0), (z / xyz_scale).clamp(0.0, 1.0)],
                        _ => return Err(invalid()),
                    };
                    v.get_mut(..3).ok_or_else(invalid)?.copy_from_slice(&result);
                }
            }
            if v.get(..n).ok_or_else(invalid)?.iter().any(|value| !value.is_finite()) {
                return Err(invalid());
            }
        }
        if n != self.outputs {
            return Err(invalid());
        }
        budget.work(1)?;
        out.copy_from_slice(v.get(..n).ok_or_else(invalid)?);
        Ok(())
    }
}

#[cfg(test)]
#[path="controlled_f64_tests.rs"]
mod tests;
