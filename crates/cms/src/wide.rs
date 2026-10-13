//! Bounded, atomic native-endian conversion of authoritative wide samples.
use crate::{CmsError, Transform, xcf_color::NonFinitePolicy};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WideSampleKind {
    U32,
    F64,
}

/// Caller-owned accounting and cancellation checkpoints. Zero sample units polls at entry.
/// Reservations are released on every path after successful reservation admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WideWork {
    ReserveBytes(u64),
    ReleaseBytes(u64),
    SampleUnits(u64),
}

impl WideSampleKind {
    const fn bytes(self) -> usize {
        match self {
            Self::U32 => 4,
            Self::F64 => 8,
        }
    }
    fn read(self, bytes: &[u8]) -> Result<f64, CmsError> {
        match self {
            Self::U32 => Ok(f64::from(u32::from_ne_bytes(bytes.try_into().map_err(|_| invalid())?)) / f64::from(u32::MAX)),
            Self::F64 => Ok(f64::from_ne_bytes(bytes.try_into().map_err(|_| invalid())?)),
        }
    }
    fn write(self, value: f64, bytes: &mut [u8]) -> Result<(), CmsError> {
        if !value.is_finite() {
            return Err(CmsError::Invalid("nonfinite wide ICC output".into()));
        }
        match self {
            Self::U32 => {
                if !(0.0..=1.0).contains(&value) {
                    return Err(CmsError::Invalid("wide ICC U32 output needs an explicit gamut/HDR conversion".into()));
                }
                bytes.copy_from_slice(&((value * f64::from(u32::MAX)).round() as u32).to_ne_bytes());
            }
            Self::F64 => bytes.copy_from_slice(&value.to_ne_bytes()),
        }
        Ok(())
    }
}

fn invalid() -> CmsError {
    CmsError::Invalid("invalid wide ICC byte buffer, stride or work dimensions".into())
}

impl Transform {
    /// Converts one bounded work slice, with atomic destination update.
    ///
    /// Both buffers contain whole native-endian pixels; strides count samples. A slice is
    /// at most 64 KiB of source and destination. Callers subdivide larger buffers at pixel
    /// boundaries and retain transaction staging above CMS. Each pixel polls/accounting
    /// before evaluation, with at most 4096 sample-kernel units between polls.
    /// Color samples must be finite; optional extra channels are copied as original bytes.
    /// PreserveBits permits nonfinite copied extras, never nonfinite color evaluation.
    #[allow(clippy::too_many_arguments)]
    pub fn try_convert_bytes_wide(
        &self,
        sample: WideSampleKind,
        src: &[u8],
        src_stride: usize,
        dst: &mut [u8],
        dst_stride: usize,
        copy_extra: bool,
        nonfinite: NonFinitePolicy,
        poll: &mut impl FnMut(WideWork) -> Result<(), CmsError>,
    ) -> Result<(), CmsError> {
        poll(WideWork::SampleUnits(0))?;
        if src.len() > 65536
            || dst.len() > 65536
            || src_stride < self.inputs()
            || dst_stride < self.outputs()
            || self.inputs() == 0
            || self.inputs() > 16
            || self.outputs() == 0
            || self.outputs() > 16
        {
            return Err(invalid());
        }
        let size = sample.bytes();
        let source_pixel = src_stride.checked_mul(size).ok_or_else(invalid)?;
        let target_pixel = dst_stride.checked_mul(size).ok_or_else(invalid)?;
        if source_pixel == 0
            || target_pixel == 0
            || !src.len().is_multiple_of(source_pixel)
            || !dst.len().is_multiple_of(target_pixel)
            || src.len() / source_pixel != dst.len() / target_pixel
        {
            return Err(invalid());
        }
        let mut channels = self.inputs();
        let mut units = channels as u64;
        for stage in &self.pipeline().stages {
            let cost = match stage {
                crate::pipeline::Stage::Clut(clut) => {
                    clut.interpolation_taps()?.checked_mul(clut.outputs as u64).and_then(|cost| cost.checked_add(channels as u64)).ok_or_else(invalid)?
                }
                _ => channels as u64,
            };
            units = units.checked_add(cost).ok_or_else(invalid)?;
            channels = stage.outputs(channels);
            if channels > 16 || units > 4096 {
                return Err(invalid());
            }
        }
        let reserved = u64::try_from(dst.len()).map_err(|_| invalid())?;
        poll(WideWork::ReserveBytes(reserved))?;
        let result = (|| {
            let mut staged = Vec::new();
            staged.try_reserve_exact(dst.len()).map_err(|_| CmsError::Invalid("wide ICC staging allocation denied".into()))?;
            staged.extend_from_slice(dst);
            let extra = if copy_extra { (src_stride - self.inputs()).min(dst_stride - self.outputs()) } else { 0 };
            for (source, target) in src.chunks_exact(source_pixel).zip(staged.chunks_exact_mut(target_pixel)) {
                poll(WideWork::SampleUnits(units))?;
                let mut input = [0.0_f64; 16];
                let mut output = [0.0_f64; 16];
                for (i, value) in input.get_mut(..self.inputs()).ok_or_else(invalid)?.iter_mut().enumerate() {
                    let offset = i.checked_mul(size).ok_or_else(invalid)?;
                    *value = sample.read(source.get(offset..offset + size).ok_or_else(invalid)?)?;
                }
                self.try_eval_f64(input.get(..self.inputs()).ok_or_else(invalid)?, output.get_mut(..self.outputs()).ok_or_else(invalid)?)?;
                if self.is_identity() {
                    let color_bytes = self.inputs().checked_mul(size).ok_or_else(invalid)?;
                    target.get_mut(..color_bytes).ok_or_else(invalid)?.copy_from_slice(source.get(..color_bytes).ok_or_else(invalid)?);
                }
                for (i, &value) in output.get(..self.outputs()).ok_or_else(invalid)?.iter().enumerate() {
                    if self.is_identity() {
                        continue;
                    }
                    let offset = i.checked_mul(size).ok_or_else(invalid)?;
                    sample.write(value, target.get_mut(offset..offset + size).ok_or_else(invalid)?)?;
                }
                for i in 0..extra {
                    let source_offset = (self.inputs() + i).checked_mul(size).ok_or_else(invalid)?;
                    let target_offset = (self.outputs() + i).checked_mul(size).ok_or_else(invalid)?;
                    let bytes = source.get(source_offset..source_offset + size).ok_or_else(invalid)?;
                    if nonfinite == NonFinitePolicy::Reject && !sample.read(bytes)?.is_finite() {
                        return Err(CmsError::Invalid("nonfinite copied wide ICC extra channel".into()));
                    }
                    target.get_mut(target_offset..target_offset + size).ok_or_else(invalid)?.copy_from_slice(bytes);
                }
            }
            poll(WideWork::SampleUnits(0))?;
            Ok(staged)
        })();
        let released = poll(WideWork::ReleaseBytes(reserved));
        let staged = result?;
        released?;
        dst.copy_from_slice(&staged);
        Ok(())
    }
}
