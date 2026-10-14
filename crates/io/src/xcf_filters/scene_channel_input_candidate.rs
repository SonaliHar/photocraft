//! Bounded decoded-owner/current saved-channel native-word correspondence.
//! This never authenticates the decoded owner against original-file bytes and
//! never establishes a channel's working transfer/profile from parent ICC or
//! Babl names. The caller independently supplies the numerical domain. Source
//! bytes, NaNs, signed zero, defaults and off-plane storage are never modified.
//! One held aggregate lease includes later opt-in point-stage preparation; child
//! reservations cannot detach from this owner or reset its remaining ceilings.
use super::{scene_channel_configured::ConfiguredSceneStacks, scene_channel_preparation_candidate::Limits};
use crate::IoError;
use photocraft_algo::gimp::graph_execute::ExecutionDomain;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_compose::{
    gimp_composite::{CompositionError, Result as ComposeResult},
    gimp_render::PreparationBudget,
};
use photocraft_doc::item_identity::ChannelId;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};
use photocraft_xcf::{Drawable, Plane, Sample};
use std::mem::size_of;

const CHUNK: usize = 4096;
const HEADER: u64 = 4096;
#[derive(Clone, Copy, Debug)]
pub struct ChannelInputWitness {
    pub channel: ChannelId,
    pub physical_ordinal: usize,
    pub bounds: Rect,
    /// Explicit caller numerical interpretation, NOT a result of word equality.
    pub domain: ExecutionDomain,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub held_owner_bytes: u64,
    pub prospective_resident_bytes: u64,
    pub index_bytes: u64,
    pub vector_bytes: u64,
    pub comparison_bytes: u64,
    pub preparation_work_units: u64,
}
/// Fields are private: callers cannot fabricate a successful comparison receipt.
pub struct PreparedChannelInput<'a> {
    source: &'a Surface,
    plane: &'a Plane,
    witness: ChannelInputWitness,
}
impl<'a> PreparedChannelInput<'a> {
    pub fn surface(&self) -> &'a Surface {
        self.source
    }
    pub fn decoded_plane(&self) -> &'a Plane {
        self.plane
    }
    pub fn witness(&self) -> ChannelInputWitness {
        self.witness
    }
}
/// Complete ordinary saved-channel set. Selection and Quick Mask interpretation
/// is deliberately outside this raw channel-input lane. The lease is retained
/// across all receipt-backed stages; the full cap is an explicit conservative
/// reservation policy, not measured RSS or refunded actual allocation.
pub struct PreparedSceneInputs<'a, 'o, 'p, 's, 'd, 'b, 'r, L> {
    original: &'a ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'r>,
    inputs: Vec<PreparedChannelInput<'a>>,
    requirements: Requirements,
    limits: Limits,
    used_resident: u64,
    used_index: u64,
    used_work: u64,
    _lease: L,
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("scene channel input: {s}"))
}
fn poll(c: &Interrupt<'_>) -> Result<(), IoError> {
    c.check().map_err(|_| IoError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size/work overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size/work overflow"))
}
fn size<T>(n: usize) -> Result<u64, IoError> {
    mul(n as u64, size_of::<T>() as u64)
}
fn raster(e: RasterInputError) -> IoError {
    if e == RasterInputError::Cancelled { IoError::Cancelled } else { bad(&e.to_string()) }
}
fn compose(e: CompositionError) -> IoError {
    match e {
        CompositionError::Raster(e) => raster(e),
        _ => bad(&e.to_string()),
    }
}
fn lower(l: Limits) -> Result<(), IoError> {
    let m = Limits::default();
    if l.resident_bytes > m.resident_bytes || l.index_bytes > m.index_bytes || l.work_units > m.work_units {
        return Err(bad("lower-only limits"));
    }
    Ok(())
}
fn native_bounds(d: &Drawable) -> Result<Rect, IoError> {
    if d.width == 0 || d.height == 0 || d.width > 262144 || d.height > 262144 {
        return Err(bad("native extent"));
    }
    let x = d.offset.0.checked_add(i32::try_from(d.width).map_err(|_| bad("width"))?).ok_or_else(|| bad("native x overflow"))?;
    let y = d.offset.1.checked_add(i32::try_from(d.height).map_err(|_| bad("height"))?).ok_or_else(|| bad("native y overflow"))?;
    if [d.offset.0, d.offset.1, x, y].iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(n)) {
        return Err(bad("native offset"));
    }
    Ok(Rect::new(d.offset.0, d.offset.1, x, y))
}
fn plane_format(p: &Plane) -> Result<PixelFormat, IoError> {
    // Wire F16 has already been widened by the existing decoder; this receipt
    // compares decoded F32 words, never claims original wire-half equality.
    let sample = match p.sample {
        Sample::F16 | Sample::F32 => SampleType::F32,
        Sample::F64 => SampleType::F64,
        _ => return Err(bad("direct native Float input only")),
    };
    Ok(PixelFormat::new(ColorMode::Grayscale, sample, false))
}
fn resolve<'a>(
    scene: &'a ConfiguredSceneStacks<'_, '_, '_, '_, '_, '_>,
    w: ChannelInputWitness,
    c: &Interrupt<'_>,
) -> Result<PreparedChannelInput<'a>, IoError> {
    poll(c)?;
    let descriptor = scene.scene().channels().get(w.physical_ordinal).ok_or_else(|| bad("physical channel ordinal"))?;
    if descriptor.source_ordinal != w.physical_ordinal || descriptor.channel != w.channel || descriptor.selection || w.channel.0 == 0 {
        return Err(bad("exact ordinary saved-channel association"));
    }
    let doc = scene.scene().current_document();
    if doc.selection_channel_id == Some(w.channel) || doc.quick_mask.as_ref().is_some_and(|v| v.id == w.channel) {
        return Err(bad("selection/Quick Mask input unsupported"));
    }
    let d = scene.scene().original_channel(w.physical_ordinal).ok_or_else(|| bad("original channel missing"))?;
    if d.selection || d.alpha || d.model != photocraft_xcf::Model::Gray || native_bounds(d)? != w.bounds {
        return Err(bad("exact decoded Gray channel geometry"));
    }
    let plane = d.plane.as_ref().ok_or_else(|| bad("decoded native plane missing"))?;
    let fmt = plane_format(plane)?;
    if plane.width != d.width
        || plane.height != d.height
        || plane.channels != 1
        || w.domain.format != fmt
        || w.domain.profile_fingerprint.is_none_or(|v| v == [0; 32])
    {
        return Err(bad("caller Float domain/native descriptor mismatch"));
    }
    if plane.bytes.len() != checked_region_byte_len(w.bounds, fmt).map_err(raster)? {
        return Err(bad("decoded native byte count"));
    }
    if doc.channels.len() > 10_000 {
        return Err(bad("current channel count"));
    }
    let mut source = None;
    for channel in &doc.channels {
        poll(c)?;
        if channel.id == w.channel && source.replace(&channel.surface).is_some() {
            return Err(bad("duplicate current channel identity"));
        }
    }
    let source = source.ok_or_else(|| bad("current channel missing"))?;
    if source.format() != fmt {
        return Err(bad("current native format changed"));
    }
    Ok(PreparedChannelInput { source, plane, witness: w })
}
pub fn requirements(
    scene: &ConfiguredSceneStacks<'_, '_, '_, '_, '_, '_>,
    witnesses: &Vec<ChannelInputWitness>,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<Requirements, IoError> {
    poll(c)?;
    lower(limits)?;
    if limits.resident_bytes < HEADER + CHUNK as u64 || witnesses.len() != scene.scene().channels().len() || witnesses.len() > 10_000 {
        return Err(bad("complete input set/header envelope"));
    }
    let s = scene.scene().requirements();
    let m = scene.requirements();
    let held = add(add(s.decoder_retained_bytes, s.source_retained_bytes)?, add(s.current_retained_bytes, s.output_bytes)?)?;
    let held = add(held, m.charged_resident_bytes)?;
    let borrowed = size::<ChannelInputWitness>(witnesses.capacity())?;
    let vector = size::<PreparedChannelInput<'_>>(witnesses.len())?;
    let mut resident = add(add(held, borrowed)?, add(vector, HEADER + CHUNK as u64)?)?;
    let index = add(add(s.index_bytes, m.charged_index_bytes)?, add(vector, borrowed)?)?;
    let mut work = add(64, mul(witnesses.len() as u64, add(scene.scene().current_document().channels.len() as u64, 32)?)?)?;
    let mut compared = 0;
    if resident > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(bad("input prospective envelope"));
    }
    for (ordinal, w) in witnesses.iter().enumerate() {
        poll(c)?;
        if w.physical_ordinal != ordinal {
            return Err(bad("complete ordered physical inputs required"));
        }
        let input = resolve(scene, *w, c)?;
        // O(1) sparse-map minimum before exact per-tile polled payload census.
        resident = add(resident, mul(input.source.tile_count() as u64, 256)?)?;
        work = add(work, mul(input.source.tile_count() as u64, 16)?)?;
        if resident > limits.resident_bytes || work > limits.work_units {
            return Err(bad("input native-map envelope"));
        }
        resident = add(resident, input.source.default_bytes().len() as u64)?;
        for (_, tile) in input.source.tiles() {
            poll(c)?;
            resident = add(resident, tile.bytes().len() as u64)?;
            if resident > limits.resident_bytes {
                return Err(bad("input native capacity"));
            }
        }
        input.source.try_validate_tile_origins_with(c).map_err(raster)?;
        compared = add(compared, input.plane.bytes.len() as u64)?;
        // Conservative native byte/read/comparison and bounded sparse lookup
        // schedule, not a measured CPU-time or deep allocator guarantee.
        let readwork = add(mul(input.plane.bytes.len() as u64, 32)?, mul(input.plane.height as u64, 128)?)?;
        work = add(work, readwork)?;
        if work > limits.work_units {
            return Err(bad("comparison work envelope"));
        }
    }
    // Requirements and private preparation both resolve/census, plus comparison.
    work = mul(work, 2)?;
    if resident > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(bad("aggregate input envelope"));
    }
    poll(c)?;
    Ok(Requirements {
        held_owner_bytes: held,
        prospective_resident_bytes: resident,
        index_bytes: index,
        vector_bytes: vector,
        comparison_bytes: compared,
        preparation_work_units: work,
    })
}
fn compare_native(input: &PreparedChannelInput<'_>, c: &Interrupt<'_>) -> Result<(), IoError> {
    let bounds = input.witness.bounds;
    let bpp = input.source.format().bytes_per_pixel();
    if bpp == 0 || bpp > CHUNK {
        return Err(bad("native sample width"));
    }
    let width = usize::try_from(bounds.width()).map_err(|_| bad("native width"))?;
    let row_bytes = width.checked_mul(bpp).ok_or_else(|| bad("row overflow"))?;
    let mut scratch = [0u8; CHUNK];
    for (row, y) in (bounds.y0..bounds.y1).enumerate() {
        poll(c)?;
        let mut column = 0usize;
        while column < width {
            poll(c)?;
            let pixels = (CHUNK / bpp).min(width - column);
            let bytes = pixels.checked_mul(bpp).ok_or_else(|| bad("chunk bytes"))?;
            let x = bounds.x0.checked_add(i32::try_from(column).map_err(|_| bad("chunk coordinate"))?).ok_or_else(|| bad("chunk coordinate"))?;
            let stop = x.checked_add(i32::try_from(pixels).map_err(|_| bad("chunk width"))?).ok_or_else(|| bad("chunk endpoint"))?;
            let target = scratch.get_mut(..bytes).ok_or_else(|| bad("chunk scratch"))?;
            input
                .source
                .try_read_interleaved_into_with(Rect::new(x, y, stop, y.checked_add(1).ok_or_else(|| bad("row endpoint"))?), target, c)
                .map_err(raster)?;
            let offset = row.checked_mul(row_bytes).and_then(|n| column.checked_mul(bpp).and_then(|v| n.checked_add(v))).ok_or_else(|| bad("native offset"))?;
            let end = offset.checked_add(bytes).ok_or_else(|| bad("native endpoint"))?;
            if input.plane.bytes.get(offset..end).ok_or_else(|| bad("truncated decoded plane"))? != target {
                return Err(bad("decoded/current native words differ"));
            }
            column = column.checked_add(pixels).ok_or_else(|| bad("chunk progression"))?;
        }
    }
    poll(c)
}
fn reserve_input_envelope<B: PreparationBudget>(limits: Limits, budget: &mut B, c: &Interrupt<'_>) -> Result<B::Lease, IoError> {
    poll(c)?;
    lower(limits)?;
    if limits.resident_bytes < HEADER + CHUNK as u64 {
        return Err(bad("input preparation header envelope"));
    }
    budget.consume_preparation_work(limits.work_units).map_err(compose)?;
    budget.reserve_prepared_bytes(limits.resident_bytes).map_err(compose)
}
pub fn prepare<'a, 'o, 'p, 's, 'd, 'b, 'r, B: PreparationBudget>(
    scene: &'a ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'r>,
    witnesses: &Vec<ChannelInputWitness>,
    limits: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<PreparedSceneInputs<'a, 'o, 'p, 's, 'd, 'b, 'r, B::Lease>, IoError> {
    poll(c)?;
    lower(limits)?;
    if size_of::<PreparedSceneInputs<'a, 'o, 'p, 's, 'd, 'b, 'r, B::Lease>>() as u64 > HEADER {
        return Err(bad("actual generic input owner/header"));
    }
    if limits.resident_bytes < HEADER + CHUNK as u64 {
        return Err(bad("input preparation header envelope"));
    }
    // Reserve the complete explicitly supplied lower envelope once. All later
    // point-stage quotas come from the private residual ledger of this lease.
    let lease = reserve_input_envelope(limits, budget, c)?;
    let q = requirements(scene, witnesses, limits, c)?;
    let mut inputs = Vec::new();
    inputs.try_reserve_exact(witnesses.len()).map_err(|_| bad("input vector allocation"))?;
    if size::<PreparedChannelInput<'_>>(inputs.capacity())? > q.vector_bytes {
        return Err(bad("actual input vector capacity"));
    }
    for w in witnesses {
        let input = resolve(scene, *w, c)?;
        compare_native(&input, c)?;
        inputs.push(input);
    }
    poll(c)?;
    Ok(PreparedSceneInputs {
        original: scene,
        inputs,
        requirements: q,
        limits,
        used_resident: q.prospective_resident_bytes,
        used_index: q.index_bytes,
        used_work: q.preparation_work_units,
        _lease: lease,
    })
}
pub(super) struct StageBudget {
    bytes: u64,
    work: u64,
}
impl PreparationBudget for StageBudget {
    type Lease = ();
    fn reserve_prepared_bytes(&mut self, n: u64) -> ComposeResult<()> {
        self.bytes = self.bytes.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn consume_preparation_work(&mut self, n: u64) -> ComposeResult<()> {
        self.work = self.work.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
impl<'a, 'o, 'p, 's, 'd, 'b, 'r, L> PreparedSceneInputs<'a, 'o, 'p, 's, 'd, 'b, 'r, L> {
    pub fn original(&self) -> &'a ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'r> {
        self.original
    }
    pub fn inputs(&self) -> &[PreparedChannelInput<'a>] {
        &self.inputs
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
    pub fn remaining_limits(&self) -> Limits {
        Limits {
            resident_bytes: self.limits.resident_bytes.saturating_sub(self.used_resident),
            index_bytes: self.limits.index_bytes.saturating_sub(self.used_index),
            work_units: self.limits.work_units.saturating_sub(self.used_work),
        }
    }
    pub(super) fn claim_stage_envelope(&mut self, limits: Limits, c: &Interrupt<'_>) -> Result<StageBudget, IoError> {
        poll(c)?;
        lower(limits)?;
        let resident = add(self.used_resident, limits.resident_bytes)?;
        let index = add(self.used_index, limits.index_bytes)?;
        let work = add(self.used_work, limits.work_units)?;
        if resident > self.limits.resident_bytes || index > self.limits.index_bytes || work > self.limits.work_units {
            return Err(bad("shared input/stage envelope exhausted"));
        }
        // Failed attempts remain consumed. No new full ceiling or refund.
        self.used_resident = resident;
        self.used_index = index;
        self.used_work = work;
        Ok(StageBudget { bytes: limits.resident_bytes, work: limits.work_units })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input<'a>(s: &'a Surface, p: &'a Plane, bounds: Rect) -> PreparedChannelInput<'a> {
        PreparedChannelInput {
            source: s,
            plane: p,
            witness: ChannelInputWitness {
                channel: ChannelId(1),
                physical_ordinal: 0,
                bounds,
                domain: ExecutionDomain { format: s.format(), encoding: photocraft_doc::SampleEncoding::Linear, profile_fingerprint: Some([1; 32]) },
            },
        }
    }
    #[test]
    fn direct_native_comparison_retains_nan_signed_zero_and_offplane_default() {
        let bounds = Rect::new(-1, 2, 2, 3);
        let words = [0x7ff8000000000042u64, (-0f64).to_bits(), 0.25f64.to_bits()];
        let bytes: Vec<u8> = words.into_iter().flat_map(u64::to_ne_bytes).collect();
        let p = Plane { width: 3, height: 1, channels: 1, sample: Sample::F64, bytes: bytes.clone() };
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let s = Surface::try_from_interleaved_with(format, bounds, &bytes, &Interrupt::NONE).unwrap();
        let default = s.default_bytes().to_vec();
        compare_native(&input(&s, &p, bounds), &Interrupt::NONE).unwrap();
        assert_eq!(p.bytes, bytes);
        assert_eq!(s.default_bytes(), default);
    }
    #[test]
    fn changed_low_bit_and_truncated_final_chunk_refuse_without_mutation() {
        let bounds = Rect::new(0, 0, 1025, 1);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut bytes = vec![0u8; 1025 * 4];
        let s = Surface::try_from_interleaved_with(format, bounds, &bytes, &Interrupt::NONE).unwrap();
        bytes[4100 - 1] = 1;
        let mut p = Plane { width: 1025, height: 1, channels: 1, sample: Sample::F32, bytes };
        assert!(compare_native(&input(&s, &p, bounds), &Interrupt::NONE).is_err());
        p.bytes.pop();
        assert!(compare_native(&input(&s, &p, bounds), &Interrupt::NONE).is_err());
        let mut pixel = [0u8; 4];
        s.try_read_pixel_bytes(1024, 0, &mut pixel).unwrap();
        assert_eq!(pixel, [0; 4]);
    }
    #[test]
    fn cancellation_between_native_chunks_is_typed_and_keeps_source() {
        let bounds = Rect::new(0, 0, 2048, 1);
        let bytes = vec![0u8; 2048 * 8];
        let p = Plane { width: 2048, height: 1, channels: 1, sample: Sample::F64, bytes: bytes.clone() };
        let s = Surface::try_from_interleaved_with(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), bounds, &bytes, &Interrupt::NONE).unwrap();
        let polls = std::cell::Cell::new(0usize);
        let cancel = || {
            polls.set(polls.get() + 1);
            polls.get() > 4
        };
        assert!(matches!(compare_native(&input(&s, &p, bounds), &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(p.bytes, bytes);
    }
    #[test]
    fn caller_work_refusal_precedes_resident_reservation_and_all_input_discovery() {
        struct Budget {
            reserve_calls: usize,
        }
        impl PreparationBudget for Budget {
            type Lease = ();
            fn consume_preparation_work(&mut self, _: u64) -> ComposeResult<()> {
                Err(RasterInputError::LimitExceeded.into())
            }
            fn reserve_prepared_bytes(&mut self, _: u64) -> ComposeResult<()> {
                self.reserve_calls += 1;
                Ok(())
            }
        }
        let mut budget = Budget { reserve_calls: 0 };
        assert!(reserve_input_envelope(Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.reserve_calls, 0);
    }
}
