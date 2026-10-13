//! Admission for decoded imports waiting for ordered profile questions.
//! Counts retained payload and the measured native model slice plus sparse-map
//! ownership. Cross-document sharing is conservatively counted again. Legacy
//! private metadata/allocator overhead is unmeasured; this is not an RSS bound.
use crate::{DocumentSnapshot, EngineError, Result};
use photocraft_doc::Document;
use photocraft_io::ImportOrigin;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};

type Held = (u64, Document, bool, Option<ImportOrigin>);

pub(crate) struct Admission {
    documents: usize,
    document_bytes: u64,
    ceiling: u64,
}

fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn limit() -> EngineError {
    EngineError::Other("pending import queue exceeds standard resident preparation limits; answer or cancel an existing profile question first".into())
}
fn slots(capacity: usize) -> Result<u64> {
    u64::try_from(capacity).ok().and_then(|n| n.checked_mul(std::mem::size_of::<Held>() as u64)).ok_or_else(limit)
}
impl Admission {
    /// Call after fallible reservation and before publishing the document/id.
    pub(crate) fn check_capacity(&self, capacity: usize, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        if capacity < self.documents || self.document_bytes.checked_add(slots(capacity)?).is_none_or(|n| n > self.ceiling) {
            return Err(limit());
        }
        poll(control)
    }
}

/// No clones or source reads. Both existing imports and the candidate must fit
/// together; individual successful decoding is not aggregate queue admission.
pub(crate) fn preflight(held: &[Held], capacity: usize, candidate: &Document, control: &Interrupt<'_>) -> Result<Admission> {
    poll(control)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let ceiling = MAX_CHECKED_RASTER_BYTES.min(preparation.resident_bytes as u64);
    let documents = held.len().checked_add(1).ok_or_else(limit)?;
    let capacity = capacity.max(documents);
    let queue_bytes = slots(capacity)?;
    if queue_bytes > ceiling {
        return Err(limit());
    }
    let mut document_bytes = 0u64;
    for doc in held.iter().map(|entry| &entry.1).chain(std::iter::once(candidate)) {
        poll(control)?;
        // The bounded structural/raster visitor rejects backed owners before
        // the native payload census and checks native tile storage without
        // materializing samples or changing the source document.
        let raster = DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
        let input = photocraft_io::retained_native_input_bytes(doc, preparation, control)
            .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(error.to_string()) })?;
        // Input includes immutable raster payload; map/default ownership is
        // separate. max conservatively retains the raster census floor too.
        let retained = (input as u64).max(raster.retained_bytes);
        document_bytes = document_bytes
            .checked_add(retained)
            .and_then(|n| n.checked_add(raster.clone_metadata_bytes))
            .filter(|n| n.checked_add(queue_bytes).is_some_and(|total| total <= ceiling))
            .ok_or_else(limit)?;
        poll(control)?;
    }
    let admission = Admission { documents, document_bytes, ceiling };
    admission.check_capacity(capacity, control)?;
    Ok(admission)
}
