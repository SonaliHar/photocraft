//! Borrowed admission for the profile-question queue, not whole-process RSS.
use super::Held;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
use std::collections::VecDeque;

fn poll(control: &Interrupt<'_>) -> Result<(), String> {
    control.check().map_err(|_| "opening cancelled".to_string())
}
fn limit() -> String {
    "pending profile questions exceed standard resident preparation limits; answer or cancel an earlier question first".into()
}
fn add(total: &mut u64, bytes: u64, ceiling: u64) -> Result<(), String> {
    *total = total.checked_add(bytes).filter(|n| *n <= ceiling).ok_or_else(limit)?;
    Ok(())
}
fn value(value: &Value, depth: usize, nodes: &mut usize, bytes: &mut u64, ceiling: u64, control: &Interrupt<'_>) -> Result<(), String> {
    poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(limit());
    }
    *nodes = nodes.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(limit)?;
    add(bytes, 128, ceiling)?;
    match value {
        Value::String(text) => add(bytes, text.capacity() as u64, ceiling)?,
        Value::Array(items) => {
            add(bytes, (items.capacity() as u64).checked_mul(std::mem::size_of::<Value>() as u64).ok_or_else(limit)?, ceiling)?;
            for item in items {
                value_visit(item, depth, nodes, bytes, ceiling, control)?;
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                add(bytes, key.capacity() as u64, ceiling)?;
                value_visit(item, depth, nodes, bytes, ceiling, control)?;
            }
        }
        _ => {}
    }
    poll(control)
}
fn value_visit(v: &Value, depth: usize, nodes: &mut usize, bytes: &mut u64, ceiling: u64, control: &Interrupt<'_>) -> Result<(), String> {
    value(v, depth.checked_add(1).ok_or_else(limit)?, nodes, bytes, ceiling, control)
}

pub(super) struct Admission {
    retained: u64,
    count: usize,
    ceiling: u64,
}
impl Admission {
    pub(super) fn check_capacity(&self, capacity: usize, control: &Interrupt<'_>) -> Result<(), String> {
        poll(control)?;
        if capacity < self.count {
            return Err(limit());
        }
        let slots = (capacity as u64).checked_mul(std::mem::size_of::<Held>() as u64).ok_or_else(limit)?;
        if self.retained.checked_add(slots).is_none_or(|n| n > self.ceiling) {
            return Err(limit());
        }
        poll(control)
    }
}
/// Reuses canonical native model/payload and raster-map censuses. Shared payloads
/// across separate imports are conservatively counted again. No source reads.
/// Legacy private model/map allocator overhead and dialog internals are unmeasured.
pub(super) fn prepare(queue: &VecDeque<Held>, candidate: &Held, control: &Interrupt<'_>) -> Result<Admission, String> {
    poll(control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let ceiling = MAX_CHECKED_RASTER_BYTES.min(limits.resident_bytes as u64);
    let count = queue.len().checked_add(1).ok_or_else(limit)?;
    let capacity = queue.capacity().max(count);
    if (capacity as u64).checked_mul(std::mem::size_of::<Held>() as u64).is_none_or(|n| n > ceiling) {
        return Err(limit());
    }
    let mut retained = 0u64;
    for held in queue.iter().chain(std::iter::once(candidate)) {
        poll(control)?;
        let raster = photocraft_engine::DocumentSnapshot::resident_raster_footprint_of(&held.doc, control).map_err(|e| e.to_string())?;
        let input = photocraft_io::retained_native_input_bytes(&held.doc, limits, control)
            .map_err(|e| if control.cancelled() { "opening cancelled".into() } else { e.to_string() })?;
        add(&mut retained, (input as u64).max(raster.retained_bytes), ceiling)?;
        add(&mut retained, raster.clone_metadata_bytes, ceiling)?;
        let mut metadata = held.name.capacity() as u64;
        if let Some(path) = &held.path {
            add(&mut metadata, path.capacity() as u64, ceiling)?;
        }
        add(&mut metadata, (held.warnings.capacity() as u64).checked_mul(std::mem::size_of::<String>() as u64).ok_or_else(limit)?, ceiling)?;
        if held.warnings.len() > 100_000 {
            return Err(limit());
        }
        for warning in &held.warnings {
            poll(control)?;
            add(&mut metadata, warning.capacity() as u64, ceiling)?;
        }
        if let Some(question) = &held.question {
            let mut nodes = 0usize;
            value(question, 0, &mut nodes, &mut metadata, ceiling, control)?;
        }
        // Includes conservative prompt/name/path copying headroom. It doesn't
        // make the existing generic dialog allocator fallible.
        add(&mut retained, metadata.checked_mul(4).ok_or_else(limit)?, ceiling)?;
        if let Some(bytes) = &held.bytes {
            add(&mut retained, bytes.capacity() as u64, ceiling)?;
        }
        poll(control)?;
    }
    let admission = Admission { retained, count, ceiling };
    admission.check_capacity(capacity, control)?;
    Ok(admission)
}
pub(super) fn string(source: &str, control: &Interrupt<'_>) -> Result<String, String> {
    poll(control)?;
    let mut text = String::new();
    text.try_reserve_exact(source.len()).map_err(|_| "pending import name/path allocation failed".to_string())?;
    text.push_str(source);
    poll(control)?;
    Ok(text)
}
