//! Borrowed admission for the profile-question queue, not whole-process RSS.
use super::{Held, HeldMetadata};
use photocraft_doc::Document;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
use std::collections::VecDeque;

fn poll(control: &Interrupt<'_>) -> Result<(), String> {
    control.check().map_err(|_| "opening cancelled".to_string())
}
fn limit() -> String {
    "could not prepare this file for opening; answer or cancel an earlier profile question and try again".into()
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

pub(super) struct Entry<'a> {
    doc: &'a Document,
    name: &'a String,
    path: Option<&'a String>,
    warnings: &'a Vec<String>,
    bytes: Option<&'a std::sync::Arc<Vec<u8>>>,
    question: Option<&'a Value>,
}
impl<'a> Entry<'a> {
    pub(super) fn held(held: &'a Held) -> Self {
        Self {
            doc: &held.doc,
            name: &held.name,
            path: held.path.as_ref(),
            warnings: &held.warnings,
            bytes: held.bytes.as_ref(),
            question: held.question.as_ref(),
        }
    }
    pub(super) fn metadata(doc: &'a Document, metadata: &'a HeldMetadata) -> Self {
        Self {
            doc,
            name: &metadata.name,
            path: metadata.path.as_ref(),
            warnings: &metadata.warnings,
            bytes: metadata.bytes.as_ref(),
            question: metadata.question.as_ref(),
        }
    }
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
pub(super) fn prepare(queue: &VecDeque<Held>, candidate: Entry<'_>, clone_candidate: bool, control: &Interrupt<'_>) -> Result<Admission, String> {
    poll(control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let ceiling = MAX_CHECKED_RASTER_BYTES.min(limits.resident_bytes as u64);
    let count = queue.len().checked_add(1).ok_or_else(limit)?;
    let capacity = queue.capacity().max(count);
    if (capacity as u64).checked_mul(std::mem::size_of::<Held>() as u64).is_none_or(|n| n > ceiling) {
        return Err(limit());
    }
    let candidate_doc = candidate.doc;
    let mut retained = 0u64;
    for held in queue.iter().map(Entry::held).chain(std::iter::once(candidate)) {
        poll(control)?;
        let raster = photocraft_engine::DocumentSnapshot::resident_raster_footprint_of(held.doc, control).map_err(|e| e.to_string())?;
        let input = photocraft_io::retained_native_input_bytes(held.doc, limits, control)
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
        for warning in held.warnings {
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
    if clone_candidate {
        // A resident tab cannot surrender its history-owned Arc<Document>. Its
        // bounded ordinary clone must be prepared before closing the original.
        let model = photocraft_io::retained_native_model_bytes(candidate_doc, limits, control).map_err(|e| e.to_string())?;
        let raster = photocraft_engine::DocumentSnapshot::resident_raster_footprint_of(candidate_doc, control).map_err(|e| e.to_string())?;
        add(&mut retained, model as u64, ceiling)?;
        add(&mut retained, raster.clone_metadata_bytes, ceiling)?;
    }
    let admission = Admission { retained, count, ceiling };
    admission.check_capacity(capacity, control)?;
    Ok(admission)
}
pub(super) fn string(source: &str, control: &Interrupt<'_>) -> Result<String, String> {
    poll(control)?;
    if source.len() as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(limit());
    }
    let mut text = String::new();
    text.try_reserve_exact(source.len()).map_err(|_| "pending import name/path allocation failed".to_string())?;
    let mut offset = 0usize;
    while offset < source.len() {
        poll(control)?;
        let mut end = offset.checked_add((source.len() - offset).min(64 * 1024)).ok_or_else(limit)?;
        while !source.is_char_boundary(end) {
            end = end.checked_sub(1).ok_or_else(limit)?;
        }
        let chunk = source.get(offset..end).ok_or_else(limit)?;
        if chunk.is_empty() {
            return Err(limit());
        }
        text.push_str(chunk);
        offset = end;
    }
    poll(control)?;
    Ok(text)
}

/// The generic serde Value clone remains its existing bounded allocator path;
/// inspect depth/count/capacity before allowing it, never after ownership take.
pub(super) fn question(source: Option<&Value>, control: &Interrupt<'_>) -> Result<Option<Value>, String> {
    let Some(source) = source else { return Ok(None) };
    let mut nodes = 0;
    let mut bytes = 0;
    let limit = photocraft_io::NativeSaveLimits::default().preparation;
    value(source, 0, &mut nodes, &mut bytes, limit.resident_bytes as u64, control)?;
    let result = source.clone();
    poll(control)?;
    Ok(Some(result))
}
