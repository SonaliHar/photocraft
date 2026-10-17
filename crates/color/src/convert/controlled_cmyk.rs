//! Controlled ordinary CMYK scope. Legacy compatibility fallback stays explicit;
//! cancellation/resource failures never become None, and no global cache grows.
use super::*;
use photocraft_cms::{
    control::{Error, Limits, Result},
    transform::TransformOptions,
};
use std::hash::{Hash, Hasher};
#[derive(Clone, Copy, Debug)]
pub struct CmykSpaceRequirements {
    pub retained_bytes: usize,
    pub peak_resident_bytes: usize,
    pub work_units: u64,
    /// Cold transforms require decoded recipes. The supplied lower envelope is
    /// reserved upfront; exact phase requirements are checked within it afterwards.
    pub cold: bool,
}
fn poll(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() { Err(Error::Cancelled) } else { Ok(()) }
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or(Error::Limit("CMYK scope bytes"))
}
fn work(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Limit("CMYK scope work"))
}
fn check(limits: Limits, bytes: usize, units: u64, cancelled: &dyn Fn() -> bool) -> Result<()> {
    poll(cancelled)?;
    let maximum = Limits::default();
    if limits.resident_bytes > maximum.resident_bytes || limits.work_units > maximum.work_units {
        return Err(Error::Limit("limits may only be lowered"));
    }
    if bytes > limits.resident_bytes || units > limits.work_units {
        return Err(Error::Limit("CMYK aggregate preparation"));
    }
    Ok(())
}
fn remaining(limits: Limits, outside: usize, used: u64) -> Result<Limits> {
    Ok(Limits {
        resident_bytes: limits.resident_bytes.checked_sub(outside).ok_or(Error::Limit("CMYK retained headroom"))?,
        work_units: limits.work_units.checked_sub(used).ok_or(Error::Limit("CMYK remaining work"))?,
    })
}
fn retained(space: &CmykSpace, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<usize> {
    let to = space.to_srgb.retained_bytes_with_control(limits, cancelled)?;
    let from = space.from_srgb.retained_bytes_with_control(limits, cancelled)?;
    add(add(to, from)?, std::mem::size_of::<u64>() + 2 * std::mem::size_of::<usize>())
}
fn peek(bytes: &Arc<Vec<u8>>, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Option<Option<Arc<CmykSpace>>>> {
    let Some(cache) = CMYK_PROFILE_CACHE.get() else { return Ok(None) };
    let entries = cache.lock().unwrap_or_else(|e| e.into_inner());
    if entries.len() as u64 > limits.work_units {
        return Err(Error::Limit("CMYK cache scan work"));
    }
    for (owner, space) in entries.iter() {
        poll(cancelled)?;
        if owner.as_ptr() == Arc::as_ptr(bytes) && owner.strong_count() > 0 {
            return Ok(Some(space.as_ref().map(Arc::clone)));
        }
    }
    Ok(None)
}
fn header_cmyk(bytes: &[u8]) -> bool {
    bytes.get(16..20) == Some(b"CMYK".as_slice())
}
pub fn requirements(icc: Option<&Arc<Vec<u8>>>, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<CmykSpaceRequirements> {
    check(limits, 0, 0, cancelled)?;
    let Some(bytes) = icc else { return Ok(CmykSpaceRequirements { retained_bytes: 0, peak_resident_bytes: 0, work_units: 0, cold: false }) };
    check(limits, bytes.capacity(), 0, cancelled)?;
    if let Some(hit) = peek(bytes, limits, cancelled)? {
        let size = hit.as_ref().map(|s| retained(s, remaining(limits, bytes.capacity(), 0)?, cancelled)).transpose()?.unwrap_or(0);
        let peak = add(bytes.capacity(), size)?;
        let cache_work = CMYK_PROFILE_CACHE.get().map(|cache| cache.lock().unwrap_or_else(|e| e.into_inner()).len() as u64).unwrap_or(0);
        let warm_work = cache_work.checked_mul(3).and_then(|v| v.checked_add(32768)).ok_or(Error::Limit("CMYK cache/census work"))?;
        check(limits, peak, warm_work, cancelled)?;
        return Ok(CmykSpaceRequirements {
            retained_bytes: peak,
            peak_resident_bytes: peak,
            work_units: cache_work.checked_mul(3).and_then(|v| v.checked_add(32768)).ok_or(Error::Limit("CMYK cache/census work"))?,
            cold: false,
        });
    }
    match Profile::parse_requirements_with_control(
        bytes,
        remaining(limits, bytes.capacity().checked_sub(bytes.len()).ok_or(Error::Limit("CMYK ICC capacity"))?, 0)?,
        cancelled,
    ) {
        Err(Error::Cms(_)) => {
            let units = work(bytes.len() as u64, 64)?;
            check(limits, bytes.capacity(), units, cancelled)?;
            return Ok(CmykSpaceRequirements { retained_bytes: bytes.capacity(), peak_resident_bytes: bytes.capacity(), work_units: units, cold: false });
        }
        Err(e) => return Err(e),
        Ok(_) => (),
    }
    if !header_cmyk(bytes) {
        return Ok(CmykSpaceRequirements {
            retained_bytes: bytes.capacity(),
            peak_resident_bytes: bytes.capacity(),
            work_units: limits.work_units,
            cold: false,
        });
    }
    // No arbitrary ICC size/class cap: decoding uses existing CMS limits. Complete
    // cold preparation reserves the caller envelope, then carries actual retained
    // owners and cumulative parse/hash/constructor work through each exact phase.
    Ok(CmykSpaceRequirements { retained_bytes: bytes.capacity(), peak_resident_bytes: limits.resident_bytes, work_units: limits.work_units, cold: true })
}
fn hash_bytes(bytes: &[u8], remaining_work: u64, cancelled: &dyn Fn() -> bool) -> Result<u64> {
    if (bytes.len() as u64).checked_add(8).is_none_or(|n| n > remaining_work) {
        return Err(Error::Limit("CMYK ICC hash work"));
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut hasher);
    for part in bytes.chunks(4096) {
        poll(cancelled)?;
        hasher.write(part);
    }
    poll(cancelled)?;
    Ok(hasher.finish())
}
fn compatibility<T>(value: Result<T>) -> Result<Option<T>> {
    match value {
        Ok(v) => Ok(Some(v)),
        Err(Error::Cms(_)) => Ok(None),
        Err(e) => Err(e),
    }
}
pub fn prepare(icc: Option<&Arc<Vec<u8>>>, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Option<Arc<CmykSpace>>> {
    let _ = requirements(icc, limits, cancelled)?;
    let Some(bytes) = icc else { return Ok(None) };
    if let Some(hit) = peek(bytes, limits, cancelled)? {
        if let Some(space) = &hit {
            let size = retained(space, remaining(limits, bytes.capacity(), 0)?, cancelled)?;
            check(limits, add(bytes.capacity(), size)?, 0, cancelled)?;
        }
        poll(cancelled)?;
        return Ok(hit);
    }
    let spare = bytes.capacity().checked_sub(bytes.len()).ok_or(Error::Limit("CMYK ICC capacity"))?;
    let parsing_limits = remaining(limits, spare, 0)?;
    let Some(parser) = compatibility(Profile::parse_requirements_with_control(bytes, parsing_limits, cancelled))? else { return Ok(None) };
    if !header_cmyk(bytes) {
        poll(cancelled)?;
        return Ok(None);
    }
    let mut used = parser.work_units.checked_mul(3).ok_or(Error::Limit("CMYK parser work"))?;
    check(limits, add(parser.peak_resident_bytes, spare)?, used, cancelled)?;
    let Some(profile) = compatibility(Profile::parse_with_control(bytes, remaining(parsing_limits, 0, parser.work_units)?, cancelled))? else {
        return Ok(None);
    };
    if profile.color_space != ColorSpace::Cmyk {
        return Ok(None);
    }
    let assigned = profile.clone_requirements_with_control(limits, cancelled)?.retained_bytes;
    check(limits, add(bytes.capacity(), assigned)?, used, cancelled)?;
    // Builtin OnceLock initialization is an explicit outer-polled provider boundary.
    poll(cancelled)?;
    let coated = Builtin::CoatedCmyk.profile();
    poll(cancelled)?;
    let coated_ret = coated.clone_requirements_with_control(limits, cancelled)?.retained_bytes;
    let hash_limits = remaining(limits, add(bytes.capacity(), assigned)?, used)?;
    let encoded = coated.to_bytes_with_control(hash_limits, cancelled)?;
    check(limits, add(add(bytes.capacity(), assigned)?, coated_ret)?, used, cancelled)?;
    let id = hash_bytes(bytes, limits.work_units.checked_sub(used).ok_or(Error::Limit("CMYK hash work"))?, cancelled)?;
    used = work(used, work(bytes.len() as u64, 8)?)?;
    let default_id = hash_bytes(&encoded, limits.work_units.checked_sub(used).ok_or(Error::Limit("CMYK default hash work"))?, cancelled)?;
    used = work(used, work(encoded.len() as u64, 8)?)?;
    if id == default_id {
        return Ok(None);
    }
    poll(cancelled)?;
    let srgb = Builtin::Srgb.profile();
    poll(cancelled)?;
    // The assigned/sRGB models are included by Transform's own retained census.
    // Original input and coated comparison owners remain outside that census.
    let outside = add(bytes.capacity(), coated_ret)?;
    let first_limits = remaining(limits, outside, used)?;
    let options = TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, ..Default::default() };
    let Some(to_req) = compatibility(Transform::requirements_with_control(&profile, srgb, options, first_limits, cancelled))? else { return Ok(None) };
    check(limits, add(outside, to_req.peak_resident_bytes)?, work(used, to_req.work_units)?, cancelled)?;
    let Some(to_srgb) = compatibility(Transform::with_options_with_control(&profile, srgb, options, first_limits, cancelled))? else { return Ok(None) };
    used = work(used, to_req.work_units)?;
    let to_ret = to_srgb.retained_bytes_with_control(first_limits, cancelled)?;
    let second_outside = add(outside, to_ret)?;
    let second_limits = remaining(limits, second_outside, used)?;
    let Some(from_req) = compatibility(Transform::requirements_with_control(srgb, &profile, options, second_limits, cancelled))? else { return Ok(None) };
    check(limits, add(second_outside, from_req.peak_resident_bytes)?, work(used, from_req.work_units)?, cancelled)?;
    let Some(from_srgb) = compatibility(Transform::with_options_with_control(srgb, &profile, options, second_limits, cancelled))? else { return Ok(None) };
    let final_ret =
        add(add(to_ret, from_srgb.retained_bytes_with_control(second_limits, cancelled)?)?, std::mem::size_of::<u64>() + 2 * std::mem::size_of::<usize>())?;
    check(limits, add(add(second_outside, assigned)?, final_ret)?, work(used, from_req.work_units)?, cancelled)?;
    poll(cancelled)?;
    Ok(Some(Arc::new(CmykSpace { to_srgb, from_srgb, id })))
}

/// Cumulative preparation debit for an ordinary assigned/default CMYK scope.
/// This is a conservative charged schedule, not CPU time or allocator/RSS.
#[derive(Clone, Copy, Debug)]
pub struct CmykScopeUsage {
    pub work_units: u64,
    pub retained_bytes: usize,
    pub peak_resident_bytes: usize,
}
struct ScopeLedger<'a> {
    limits: Limits,
    cancelled: &'a dyn Fn() -> bool,
    used: u64,
    peak: usize,
}
impl ScopeLedger<'_> {
    fn charge(&mut self, units: u64) -> Result<()> {
        poll(self.cancelled)?;
        self.used = work(self.used, units)?;
        check(self.limits, self.peak, self.used, self.cancelled)
    }
    fn observe(&mut self, bytes: usize) -> Result<()> {
        check(self.limits, bytes, self.used, self.cancelled)?;
        self.peak = self.peak.max(bytes);
        Ok(())
    }
    fn remaining(&self, outside: usize) -> Result<Limits> {
        remaining(self.limits, outside, self.used)
    }
    fn assigned(&self, outside: usize, units: u64) -> Result<Limits> {
        Ok(Limits { resident_bytes: self.limits.resident_bytes.checked_sub(outside).ok_or(Error::Limit("CMYK scope resident headroom"))?, work_units: units })
    }
    // Opposite CMYK/RGB model equality exits before payload comparison.
    // Inventories/selected-side costs visit <=16 curve headers or15CLUT axes.
    fn metadata(&mut self, profiles: &[&Profile]) -> Result<u64> {
        self.charge(65536)?;
        let mut count = 0u64;
        for profile in profiles {
            for lut in profile.a2b.iter().chain(&profile.b2a).flatten() {
                poll(self.cancelled)?;
                if lut.stages.len() > 4096 {
                    return Err(Error::Limit("profile stage count"));
                }
                count = work(count, lut.stages.len() as u64)?;
            }
        }
        let variable = count.checked_mul(512).ok_or(Error::Limit("CMYK metadata work"))?;
        self.charge(variable)?;
        work(65536, variable)
    }
    fn inventory(&mut self, transform: &Transform, outside: usize) -> Result<usize> {
        self.charge(65536)?;
        let variable = (transform.pipeline().stages.len() as u64).checked_mul(512).ok_or(Error::Limit("CMYK inventory work"))?;
        self.charge(variable)?;
        let bound = work(65536, variable)?;
        transform.retained_bytes_with_control(self.assigned(outside, bound)?, self.cancelled)
    }
    fn profile_retained(&mut self, profile: &Profile, outside: usize) -> Result<usize> {
        self.metadata(&[profile])?;
        // This borrowed census API also admits its unused clone forecast.
        // Its planning admission may inspect future remaining headroom; only
        // the independently prepaid metadata walk is performed here.
        Ok(profile.clone_requirements_with_control(self.remaining(outside)?, self.cancelled)?.retained_bytes)
    }
}
fn scope_transform(source: &Profile, destination: &Profile, outside: usize, ledger: &mut ScopeLedger<'_>) -> Result<(Option<Transform>, usize, usize)> {
    if !matches!((source.color_space, destination.color_space), (ColorSpace::Cmyk, ColorSpace::Rgb) | (ColorSpace::Rgb, ColorSpace::Cmyk)) {
        return Err(Error::Limit("CMYK opposite model pair"));
    }
    ledger.metadata(&[source, destination])?;
    let options = TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, ..Default::default() };
    // Requirements admits its FUTURE constructor envelope, despite only doing
    // metadata now. It therefore receives remaining headroom, not merely Q.
    let Some(required) = compatibility(Transform::requirements_with_control(source, destination, options, ledger.remaining(outside)?, ledger.cancelled))?
    else {
        return Ok((None, 0, 0));
    };
    ledger.observe(add(outside, required.peak_resident_bytes)?)?;
    ledger.metadata(&[source, destination])?; // constructor's repeated requirements
    ledger.charge(required.work_units)?;
    let Some(transform) =
        compatibility(Transform::with_options_with_control(source, destination, options, ledger.assigned(outside, required.work_units)?, ledger.cancelled))?
    else {
        return Ok((None, 0, required.peak_resident_bytes));
    };
    let retained = ledger.inventory(&transform, outside)?;
    ledger.observe(add(outside, retained)?)?;
    Ok((Some(transform), retained, required.peak_resident_bytes))
}
fn scope_custom(icc: Option<&Arc<Vec<u8>>>, ledger: &mut ScopeLedger<'_>) -> Result<(Option<Arc<CmykSpace>>, usize)> {
    let Some(bytes) = icc else { return Ok((None, 0)) };
    ledger.observe(bytes.capacity())?;
    if let Some(cache) = CMYK_PROFILE_CACHE.get() {
        let entries = cache.lock().unwrap_or_else(|error| error.into_inner());
        // Count and scan under this SAME lock; an unlocked recount is not proof.
        ledger.charge((entries.len() as u64).checked_mul(16).and_then(|n| n.checked_add(128)).ok_or(Error::Limit("CMYK cache work"))?)?;
        let mut hit = None;
        for (owner, space) in entries.iter() {
            poll(ledger.cancelled)?;
            if owner.as_ptr() == Arc::as_ptr(bytes) && owner.strong_count() > 0 {
                hit = Some(space.as_ref().map(Arc::clone));
                break;
            }
        }
        drop(entries);
        if let Some(space) = hit {
            let retained = if let Some(space) = &space {
                let first = ledger.inventory(&space.to_srgb, bytes.capacity())?;
                let second = ledger.inventory(&space.from_srgb, add(bytes.capacity(), first)?)?;
                add(add(first, second)?, std::mem::size_of::<CmykSpace>() + 256)?
            } else {
                0
            };
            let held = add(bytes.capacity(), retained)?;
            ledger.observe(held)?;
            return Ok((space, held));
        }
    }
    let p = Profile::parse_planning_work_ceiling(bytes.len())?;
    ledger.charge(p)?;
    let spare = bytes.capacity().checked_sub(bytes.len()).ok_or(Error::Limit("CMYK ICC capacity"))?;
    let Some(required) = compatibility(Profile::parse_requirements_with_control(bytes, ledger.remaining(spare)?, ledger.cancelled))? else {
        return Ok((None, bytes.capacity()));
    };
    if !header_cmyk(bytes) {
        return Ok((None, bytes.capacity()));
    }
    ledger.observe(add(required.peak_resident_bytes, spare)?)?;
    ledger.charge(work(p, required.work_units)?)?; // inner planner + actual parse/copy
    let Some(profile) = compatibility(Profile::parse_with_control(bytes, ledger.assigned(spare, required.work_units)?, ledger.cancelled))? else {
        return Ok((None, bytes.capacity()));
    };
    if profile.color_space != ColorSpace::Cmyk {
        return Ok((None, bytes.capacity()));
    }
    let assigned = ledger.profile_retained(&profile, bytes.capacity())?;
    ledger.observe(add(bytes.capacity(), assigned)?)?;
    poll(ledger.cancelled)?;
    let coated = Builtin::CoatedCmyk.profile(); // explicit outer-polled provider boundary
    poll(ledger.cancelled)?;
    let coated_ret = ledger.profile_retained(coated, add(bytes.capacity(), assigned)?)?;
    ledger.observe(add(add(bytes.capacity(), assigned)?, coated_ret)?)?;
    // Both shipped parse and synth fallback retain encoded bytes. No reencode,
    // model clone, or unchecked to_bytes allocation is hidden in this comparison.
    let encoded = coated.encoded_bytes().ok_or(Error::Limit("builtin CMYK cached encoding"))?;
    let input_hash_work = work(bytes.len() as u64, 8)?;
    ledger.charge(input_hash_work)?;
    let id = hash_bytes(bytes, input_hash_work, ledger.cancelled)?;
    let default_hash_work = work(encoded.len() as u64, 8)?;
    ledger.charge(default_hash_work)?;
    if id == hash_bytes(encoded, default_hash_work, ledger.cancelled)? {
        return Ok((None, bytes.capacity()));
    }
    poll(ledger.cancelled)?;
    let srgb = Builtin::Srgb.profile();
    poll(ledger.cancelled)?;
    let outside = add(bytes.capacity(), coated_ret)?;
    let (Some(to_srgb), to_ret, _) = scope_transform(&profile, srgb, outside, ledger)? else { return Ok((None, bytes.capacity())) };
    let (Some(from_srgb), from_ret, _) = scope_transform(srgb, &profile, add(outside, to_ret)?, ledger)? else { return Ok((None, bytes.capacity())) };
    let held = add(bytes.capacity(), add(add(to_ret, from_ret)?, std::mem::size_of::<CmykSpace>() + 256)?)?;
    ledger.observe(add(add(add(outside, assigned)?, add(to_ret, from_ret)?)?, std::mem::size_of::<CmykSpace>() + 256)?)?;
    ledger.observe(held)?;
    poll(ledger.cancelled)?;
    Ok((Some(Arc::new(CmykSpace { to_srgb, from_srgb, id })), held))
}
fn scope_default_part(
    cache: &OnceLock<Option<Transform>>,
    source: &Profile,
    destination: &Profile,
    outside: usize,
    ledger: &mut ScopeLedger<'_>,
) -> Result<(Option<Option<Transform>>, usize)> {
    if let Some(current) = cache.get() {
        let retained = current.as_ref().map(|t| ledger.inventory(t, outside)).transpose()?.unwrap_or(0);
        ledger.observe(add(outside, retained)?)?;
        return Ok((None, retained));
    }
    // Observe None once. A concurrent None->Some publication cannot make our
    // candidate free: construction is still prepaid and privately retained.
    let (value, _private_retained, constructor_peak) = scope_transform(source, destination, outside, ledger)?;
    // A concurrent legacy initializer can win the OnceLock. Its Vec capacities
    // are not the capacities of our discarded private candidate. Keep the full
    // admitted constructor peak as the escaped cold-direction retention bound,
    // just as the existing default_part does. The second direction therefore
    // includes this first bound in its actual simultaneous headroom.
    ledger.observe(add(outside, constructor_peak)?)?;
    Ok((Some(value), constructor_peak))
}
fn prepare_scope_core(
    icc: Option<&Arc<Vec<u8>>>,
    needs_default: bool,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Option<Arc<CmykSpace>>, CmykScopeUsage)> {
    check(limits, 0, 0, cancelled)?;
    let mut ledger = ScopeLedger { limits, cancelled, used: 0, peak: 0 };
    ledger.charge(1024)?; // fixed scope/Arc/header/final-fence operations
    let (space, mut retained) = scope_custom(icc, &mut ledger)?;
    let mut defaults = None;
    if space.is_none() && needs_default {
        poll(cancelled)?;
        let coated = Builtin::CoatedCmyk.profile();
        poll(cancelled)?;
        let srgb = Builtin::Srgb.profile();
        poll(cancelled)?;
        let outside = icc.map_or(0, |bytes| bytes.capacity());
        let (to, to_ret) = scope_default_part(&DEFAULT_CMYK_TO_SRGB, coated, srgb, outside, &mut ledger)?;
        let (from, from_ret) = scope_default_part(&DEFAULT_SRGB_TO_CMYK, srgb, coated, add(outside, to_ret)?, &mut ledger)?;
        retained = add(outside, add(to_ret, from_ret)?)?;
        ledger.observe(retained)?;
        defaults = Some((to, from));
    }
    let usage = CmykScopeUsage { work_units: ledger.used, retained_bytes: retained, peak_resident_bytes: ledger.peak };
    poll(cancelled)?;
    if let Some(values) = defaults {
        publish_defaults(&DEFAULT_CMYK_TO_SRGB, &DEFAULT_SRGB_TO_CMYK, values, cancelled)?;
    }
    Ok((space, usage))
}
impl CmykSpace {
    /// One cumulative assigned/default preparation under the original limits.
    /// None is compatibility data; cancellation/limits never become fallback.
    pub fn prepare_scope_with_control(
        icc: Option<&Arc<Vec<u8>>>,
        needs_default: bool,
        limits: Limits,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(Option<Arc<Self>>, CmykScopeUsage)> {
        prepare_scope_core(icc, needs_default, limits, cancelled)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DefaultCmykRequirements {
    pub retained_bytes: usize,
    pub peak_resident_bytes: usize,
    pub work_units: u64,
    pub cold: bool,
}
struct DefaultPart {
    peak: usize,
    retained: usize,
    work: u64,
    planning: u64,
    missing: bool,
}
fn default_planning_work(source: &Profile, destination: &Profile, cancelled: &dyn Fn() -> bool) -> Result<u64> {
    // Opposite fixed models make requirements' model equality stop before any
    // curve/CLUT payload comparison. Inventory and selected-side costing only
    // inspect bounded stage/curve/axis metadata, never sample or copy grid data.
    if !matches!((source.color_space, destination.color_space), (ColorSpace::Cmyk, ColorSpace::Rgb) | (ColorSpace::Rgb, ColorSpace::Cmyk)) {
        return Err(Error::Limit("default CMYK model pair"));
    }
    let mut stages = 0u64;
    for profile in [source, destination] {
        poll(cancelled)?;
        for lut in profile.a2b.iter().chain(&profile.b2a).flatten() {
            poll(cancelled)?;
            if lut.stages.len() > 4096 {
                return Err(Error::Limit("default CMYK stage count"));
            }
            stages = work(stages, lut.stages.len() as u64)?;
        }
    }
    // One complete inventory plus at most six side walks; each stage inspects
    // at most 15 curves/axes. Fixed headers, scalar curves and profile checks fit
    // the separate 65,536-unit allowance. This bound precedes the actual pass.
    stages.checked_mul(512).and_then(|n| n.checked_add(65536)).ok_or(Error::Limit("default CMYK planning work"))
}
fn default_part(
    cache: &OnceLock<Option<Transform>>,
    source: &Profile,
    destination: &Profile,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<DefaultPart> {
    if let Some(current) = cache.get() {
        let retained = current.as_ref().map(|t| t.retained_bytes_with_control(limits, cancelled)).transpose()?.unwrap_or(0);
        return Ok(DefaultPart { peak: retained, retained, work: 32768, planning: 32768, missing: false });
    }
    let planning = default_planning_work(source, destination, cancelled)?;
    check(limits, 0, planning, cancelled)?;
    let options = TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, ..Default::default() };
    let required = match Transform::requirements_with_control(source, destination, options, limits, cancelled) {
        Ok(v) => v,
        Err(Error::Cms(_)) => return Ok(DefaultPart { peak: 0, retained: 0, work: 32768, planning: 32768, missing: true }),
        Err(e) => return Err(e),
    };
    // TransformRequirements.retained_bytes inventories the input profiles, not
    // the completed sampled transform. Until that owner exists, retain the full
    // constructor peak as its conservative output bound; no scratch credit.
    Ok(DefaultPart { peak: required.peak_resident_bytes, retained: required.peak_resident_bytes, work: required.work_units, planning, missing: true })
}
fn default_pair_envelope(first: &DefaultPart, second: &DefaultPart) -> Result<(usize, u64)> {
    // Cold retained is deliberately the full constructor peak, since the profile
    // census alone cannot describe its future core/grid/post allocations.
    let peak = first.peak.max(add(first.retained, second.peak)?);
    // Cover the public requirements pass, warm-up's repeated pass, candidate
    // planning, and each constructor's own requirements pass before publication.
    let planning = work(first.planning, second.planning)?.checked_mul(4).ok_or(Error::Limit("default CMYK planning work"))?;
    Ok((peak, work(work(first.work, second.work)?, planning)?))
}
/// Complete paired reservation before default table allocation. Initialized
/// cache entries (including legacy negative None) are borrowed without rebuilding.
pub fn default_requirements(limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<DefaultCmykRequirements> {
    check(limits, 0, 0, cancelled)?;
    if let (Some(to), Some(from)) = (DEFAULT_CMYK_TO_SRGB.get(), DEFAULT_SRGB_TO_CMYK.get()) {
        let to = to.as_ref().map(|v| v.retained_bytes_with_control(limits, cancelled)).transpose()?.unwrap_or(0);
        let from = from.as_ref().map(|v| v.retained_bytes_with_control(limits, cancelled)).transpose()?.unwrap_or(0);
        let bytes = add(to, from)?;
        check(limits, bytes, 65536, cancelled)?;
        return Ok(DefaultCmykRequirements { retained_bytes: bytes, peak_resident_bytes: bytes, work_units: 65536, cold: false });
    }
    // Builtin OnceLock profile initialization/encoding remains an outer-polled
    // provider boundary; all Transform copies and table work below are controlled.
    poll(cancelled)?;
    let coated = Builtin::CoatedCmyk.profile();
    poll(cancelled)?;
    let srgb = Builtin::Srgb.profile();
    poll(cancelled)?;
    let first = default_part(&DEFAULT_CMYK_TO_SRGB, coated, srgb, limits, cancelled)?;
    let second_limits = remaining(limits, first.retained, first.work)?;
    let second = default_part(&DEFAULT_SRGB_TO_CMYK, srgb, coated, second_limits, cancelled)?;
    let (peak, units) = default_pair_envelope(&first, &second)?;
    check(limits, peak, units, cancelled)?;
    Ok(DefaultCmykRequirements {
        retained_bytes: add(first.retained, second.retained)?,
        peak_resident_bytes: peak,
        work_units: units,
        cold: first.missing || second.missing,
    })
}
fn default_candidate(
    cache: &OnceLock<Option<Transform>>,
    source: &Profile,
    destination: &Profile,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Option<Transform>>> {
    if cache.get().is_some() {
        return Ok(None);
    }
    Ok(Some(compatibility(Transform::new_with_control(source, destination, Intent::RelativeColorimetric, true, limits, cancelled))?))
}
type DefaultCandidates = (Option<Option<Transform>>, Option<Option<Transform>>);

fn default_pair_candidates(
    to: &OnceLock<Option<Transform>>,
    from: &OnceLock<Option<Transform>>,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<DefaultCandidates> {
    poll(cancelled)?;
    let coated = Builtin::CoatedCmyk.profile();
    poll(cancelled)?;
    let srgb = Builtin::Srgb.profile();
    poll(cancelled)?;
    let first = default_part(to, coated, srgb, limits, cancelled)?;
    let second_limits = remaining(limits, first.retained, first.work)?;
    let second = default_part(from, srgb, coated, second_limits, cancelled)?;
    let (peak, units) = default_pair_envelope(&first, &second)?;
    check(limits, peak, units, cancelled)?;
    let first_value = default_candidate(to, coated, srgb, limits, cancelled)?;
    let second_value = default_candidate(from, srgb, coated, second_limits, cancelled)?;
    poll(cancelled)?;
    Ok((first_value, second_value))
}
fn publish_defaults(
    to: &OnceLock<Option<Transform>>,
    from: &OnceLock<Option<Transform>>,
    values: DefaultCandidates,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    // No cancellation/fallible preparation after this final fence. A concurrent
    // legacy initializer wins normally; discard our unused prepared candidate.
    poll(cancelled)?;
    if let Some(value) = values.0 {
        let _ = to.set(value);
    }
    if let Some(value) = values.1 {
        let _ = from.set(value);
    }
    Ok(())
}
pub fn warm_defaults(limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<()> {
    let required = default_requirements(limits, cancelled)?;
    if !required.cold {
        poll(cancelled)?;
        return Ok(());
    }
    let values = default_pair_candidates(&DEFAULT_CMYK_TO_SRGB, &DEFAULT_SRGB_TO_CMYK, limits, cancelled)?;
    publish_defaults(&DEFAULT_CMYK_TO_SRGB, &DEFAULT_SRGB_TO_CMYK, values, cancelled)
}

#[cfg(test)]
mod default_tests {
    use super::*;
    #[test]
    fn controlled_default_pair_preserves_legacy_recipe_and_pixel_words() {
        let to = OnceLock::new();
        let from = OnceLock::new();
        for (source, destination) in [(Builtin::CoatedCmyk.profile(), Builtin::Srgb.profile()), (Builtin::Srgb.profile(), Builtin::CoatedCmyk.profile())] {
            eprintln!(
                "default CMYK phase requirements: {:?}",
                Transform::requirements_with_control(
                    source,
                    destination,
                    TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, ..Default::default() },
                    Limits::default(),
                    &|| false
                )
            );
        }
        let first = default_part(&to, Builtin::CoatedCmyk.profile(), Builtin::Srgb.profile(), Limits::default(), &|| false).unwrap();
        let second = default_part(
            &from,
            Builtin::Srgb.profile(),
            Builtin::CoatedCmyk.profile(),
            remaining(Limits::default(), first.retained, first.work).unwrap(),
            &|| false,
        )
        .unwrap();
        let (peak, units) = default_pair_envelope(&first, &second).unwrap();
        eprintln!("default CMYK paired peak={peak}, work={units}");
        assert!(matches!(default_pair_candidates(&to, &from, Limits { resident_bytes: peak - 1, work_units: units }, &|| false), Err(Error::Limit(_))));
        assert!(matches!(default_pair_candidates(&to, &from, Limits { resident_bytes: peak, work_units: units - 1 }, &|| false), Err(Error::Limit(_))));
        assert!(to.get().is_none() && from.get().is_none());
        let candidates = default_pair_candidates(&to, &from, Limits { resident_bytes: peak, work_units: units }, &|| false).unwrap();
        publish_defaults(&to, &from, candidates, &|| false).unwrap();
        let actual = to.get().unwrap().as_ref().unwrap();
        let reverse = from.get().unwrap().as_ref().unwrap();
        let legacy = Transform::new(Builtin::CoatedCmyk.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
        let legacy_reverse = Transform::new(Builtin::Srgb.profile(), Builtin::CoatedCmyk.profile(), Intent::RelativeColorimetric, true).unwrap();
        assert_eq!(actual.pipeline(), legacy.pipeline());
        assert_eq!(reverse.pipeline(), legacy_reverse.pipeline());
        for pixel in [[0.0, 0.0, 0.0, 0.0], [0.25, 0.5, 0.75, 0.125], [1.0, 1.0, 1.0, 1.0]] {
            let mut a = [0.0; 3];
            let mut b = [0.0; 3];
            actual.eval_fast(&pixel, &mut a);
            legacy.eval_fast(&pixel, &mut b);
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
        for pixel in [[0.0, 0.0, 0.0], [0.125, 0.5, 0.875], [1.0, 1.0, 1.0]] {
            let mut a = [0.0; 4];
            let mut b = [0.0; 4];
            reverse.eval_fast(&pixel, &mut a);
            legacy_reverse.eval_fast(&pixel, &mut b);
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
        assert!(CmykSpace::for_profile_with_control(None, Limits::default(), &|| false).unwrap().is_none());
    }
    #[test]
    fn cancelled_final_fence_publishes_neither_default_candidate() {
        let to = OnceLock::new();
        let from = OnceLock::new();
        // Negative values make this final-publication test independent of table costs.
        assert!(matches!(publish_defaults(&to, &from, (Some(None), Some(None)), &|| true), Err(Error::Cancelled)));
        assert!(to.get().is_none());
        assert!(from.get().is_none());
    }
    #[test]
    fn resource_or_construction_cancellation_leaves_local_defaults_uninitialized() {
        let to = OnceLock::new();
        let from = OnceLock::new();
        assert!(matches!(default_pair_candidates(&to, &from, Limits { resident_bytes: 1, work_units: 1 }, &|| false), Err(Error::Limit(_))));
        assert!(to.get().is_none());
        assert!(from.get().is_none());
        let calls = std::cell::Cell::new(0);
        assert!(matches!(
            default_pair_candidates(&to, &from, Limits::default(), &|| {
                calls.set(calls.get() + 1);
                calls.get() > 20
            }),
            Err(Error::Cancelled)
        ));
        assert!(to.get().is_none());
        assert!(from.get().is_none());
    }
    #[test]
    fn initialized_negative_default_is_not_rebuilt_or_replaced() {
        let to = OnceLock::new();
        let from = OnceLock::new();
        assert!(to.set(None).is_ok());
        assert!(from.set(None).is_ok());
        let values = default_pair_candidates(&to, &from, Limits::default(), &|| false).unwrap();
        assert!(values.0.is_none());
        assert!(values.1.is_none());
        publish_defaults(&to, &from, values, &|| false).unwrap();
        assert!(to.get().unwrap().is_none());
        assert!(from.get().unwrap().is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn streamed_default_hasher_matches_profile_vec_hash_at_chunk_boundaries() {
        for n in [0, 1, 7, 8, 13, 1023, 1024, 1025, 4095, 4096, 4097, 8193] {
            let bytes = (0..n).map(|i| (i % 251) as u8).collect::<Vec<_>>();
            let mut expected = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut expected);
            assert_eq!(hash_bytes(&bytes, u64::MAX, &|| false).unwrap(), expected.finish());
        }
        let profile = Builtin::CoatedCmyk.profile();
        assert_eq!(hash_bytes(&profile.to_bytes(), u64::MAX, &|| false).unwrap(), profile.content_hash());
    }
    #[test]
    fn default_invalid_and_non_cmyk_preserve_none_but_resources_do_not_fallback() {
        assert!(prepare(None, Limits::default(), &|| false).unwrap().is_none());
        let invalid = Arc::new(vec![0; 140]);
        assert!(prepare(Some(&invalid), Limits::default(), &|| false).unwrap().is_none());
        let rgb = Builtin::Srgb.profile().to_bytes();
        assert!(prepare(Some(&rgb), Limits::default(), &|| false).unwrap().is_none());
        let coated = Builtin::CoatedCmyk.profile().to_bytes();
        assert!(prepare(Some(&coated), Limits::default(), &|| false).unwrap().is_none());
        assert!(matches!(prepare(Some(&coated), Limits { resident_bytes: 1, work_units: 1 }, &|| false), Err(Error::Limit(_))));
    }
    #[test]
    fn cold_private_factory_matches_legacy_space_and_readonly_cache_hit() {
        let mut raw = Builtin::CoatedCmyk.profile().to_bytes().as_ref().clone();
        raw.extend_from_slice(b"owned raw tail distinct identity");
        let bytes = Arc::new(raw);
        let original = bytes.as_ref().clone();
        assert!(peek(&bytes, Limits::default(), &|| false).unwrap().is_none());
        let current = prepare(Some(&bytes), Limits::default(), &|| false).unwrap().unwrap();
        assert!(peek(&bytes, Limits::default(), &|| false).unwrap().is_none());
        let legacy = CmykSpace::for_profile(Some(&bytes)).unwrap();
        assert_eq!(current.id, legacy.id);
        assert_eq!(current.to_srgb.pipeline(), legacy.to_srgb.pipeline());
        assert_eq!(current.from_srgb.pipeline(), legacy.from_srgb.pipeline());
        let hit = prepare(Some(&bytes), Limits::default(), &|| false).unwrap().unwrap();
        assert!(Arc::ptr_eq(&legacy, &hit));
        assert_eq!(bytes.as_ref(), &original);
    }
    #[test]
    fn cancellation_preserves_source_bytes_and_does_not_cache_a_partial_space() {
        let mut raw = Builtin::CoatedCmyk.profile().to_bytes().as_ref().clone();
        raw.extend_from_slice(b"owned cancellation source");
        let bytes = Arc::new(raw);
        let before = bytes.as_ref().clone();
        let calls = Cell::new(0);
        assert!(matches!(
            prepare(Some(&bytes), Limits::default(), &|| {
                calls.set(calls.get() + 1);
                calls.get() > 50
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(bytes.as_ref(), &before);
        assert!(peek(&bytes, Limits::default(), &|| false).unwrap().is_none());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod cumulative_scope_tests {
    use super::*;
    fn assigned() -> Arc<Vec<u8>> {
        // External synthetic input creation is not an operation resource receipt.
        photocraft_cms::synth::cmyk_profile(&photocraft_cms::synth::CmykParams {
            description: "Own cumulative cold brush profile".into(),
            tvi: [0.26, 0.26, 0.26, 0.3],
            grid_a2b: 5,
            grid_b2a: 9,
            ..Default::default()
        })
        .to_bytes()
    }
    #[test]
    fn cold_custom_scope_preserves_legacy_words_and_observed_limits() {
        let bytes = assigned();
        let before = hash_bytes(&bytes, u64::MAX, &|| false).unwrap();
        // Cold operation comes BEFORE the legacy oracle/cache initialization.
        let (cold, usage) = CmykSpace::prepare_scope_with_control(Some(&bytes), true, Limits::default(), &|| false).unwrap();
        let cold = cold.unwrap();
        assert!(usage.work_units <= Limits::default().work_units);
        assert!(usage.retained_bytes <= usage.peak_resident_bytes);
        let exact = Limits { resident_bytes: usage.peak_resident_bytes, work_units: usage.work_units };
        let (again, again_usage) = CmykSpace::prepare_scope_with_control(Some(&bytes), true, exact, &|| false).unwrap();
        assert!(again.is_some());
        assert_eq!(again_usage.work_units, usage.work_units);
        assert!(matches!(
            CmykSpace::prepare_scope_with_control(Some(&bytes), true, Limits { work_units: usage.work_units - 1, ..exact }, &|| false),
            Err(Error::Limit(_))
        ));
        assert!(matches!(
            CmykSpace::prepare_scope_with_control(Some(&bytes), true, Limits { resident_bytes: usage.peak_resident_bytes - 1, ..exact }, &|| false),
            Err(Error::Limit(_))
        ));
        // Genuine existing cache API is exercised only AFTER cold/boundary checks.
        let legacy = CmykSpace::for_profile(Some(&bytes)).unwrap();
        for input in [[0.0, 0.0, 0.0, 0.0], [0.25, 0.5, 0.75, 0.125], [1.0, 1.0, 1.0, 1.0]] {
            let mut a = [0.0; 3];
            let mut b = [0.0; 3];
            cold.to_srgb.eval_fast(&input, &mut a);
            legacy.to_srgb.eval_fast(&input, &mut b);
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
        for input in [[0.0, 0.0, 0.0], [0.8, 0.2, 0.4], [1.0, 1.0, 1.0]] {
            let mut a = [0.0; 4];
            let mut b = [0.0; 4];
            cold.from_srgb.eval_fast(&input, &mut a);
            legacy.from_srgb.eval_fast(&input, &mut b);
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
        let (warm, warm_usage) = CmykSpace::prepare_scope_with_control(Some(&bytes), true, Limits::default(), &|| false).unwrap();
        assert!(Arc::ptr_eq(warm.as_ref().unwrap(), &legacy));
        assert!(warm_usage.work_units < usage.work_units);
        assert_eq!(hash_bytes(&bytes, u64::MAX, &|| false).unwrap(), before);
    }
    #[test]
    fn compatibility_none_default_and_cancel_keep_one_ledger() {
        for bytes in [None, Some(Arc::new(vec![0; 140])), Some(Builtin::Srgb.profile().to_bytes())] {
            let (space, usage) = CmykSpace::prepare_scope_with_control(bytes.as_ref(), false, Limits::default(), &|| false).unwrap();
            assert!(space.is_none());
            assert!(usage.work_units >= 1024);
            assert!(matches!(
                CmykSpace::prepare_scope_with_control(bytes.as_ref(), false, Limits { work_units: 0, ..Limits::default() }, &|| false),
                Err(Error::Limit(_))
            ));
            assert!(matches!(CmykSpace::prepare_scope_with_control(bytes.as_ref(), true, Limits::default(), &|| true), Err(Error::Cancelled)));
        }
        let (space, usage) = CmykSpace::prepare_scope_with_control(None, true, Limits::default(), &|| false).unwrap();
        assert!(space.is_none());
        assert!(usage.work_units <= Limits::default().work_units);
        assert!(DEFAULT_CMYK_TO_SRGB.get().and_then(Option::as_ref).is_some());
        assert!(DEFAULT_SRGB_TO_CMYK.get().and_then(Option::as_ref).is_some());
        // Observe actual intermediate polls on a fresh owned profile allocation;
        // these are cancellation boundaries, not unique phase attribution.
        let bytes = assigned();
        let original = hash_bytes(&bytes, u64::MAX, &|| false).unwrap();
        for stop in [3, 20, 120] {
            let count = std::cell::Cell::new(0usize);
            let cancelled = || {
                let n = count.get();
                count.set(n + 1);
                n >= stop
            };
            assert!(matches!(CmykSpace::prepare_scope_with_control(Some(&bytes), true, Limits::default(), &cancelled), Err(Error::Cancelled)));
            assert_eq!(hash_bytes(&bytes, u64::MAX, &|| false).unwrap(), original);
        }
        // Private candidates still use the unchanged final publication fence.
        let to = OnceLock::new();
        let from = OnceLock::new();
        assert!(matches!(publish_defaults(&to, &from, (Some(None), Some(None)), &|| true), Err(Error::Cancelled)));
        assert!(to.get().is_none() && from.get().is_none());
    }
}
