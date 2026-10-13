//! Bounded independently computed digests for immutable retained cache/resource Arc bytes.
//! Weak identity is a speed hint, never source/evaluator/profile authority.
use crate::XcfValidationError;
use photocraft_raster::Interrupt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
const CAPACITY: usize = 64;
struct Entry {
    bytes: Weak<Vec<u8>>,
    length: usize,
    digest: blake3::Hash,
}
struct Cache {
    entries: [Option<Entry>; CAPACITY],
    next: usize,
}
impl Cache {
    fn new() -> Self {
        Self { entries: std::array::from_fn(|_| None), next: 0 }
    }
    fn clear(&mut self) {
        for entry in &mut self.entries {
            *entry = None;
        }
        self.next = 0;
    }
    fn get(&mut self, bytes: &Arc<Vec<u8>>) -> Option<blake3::Hash> {
        for entry in &mut self.entries {
            let Some(value) = entry else {
                continue;
            };
            let Some(live) = value.bytes.upgrade() else {
                *entry = None;
                continue;
            };
            if value.length == bytes.len() && live.len() == bytes.len() && Arc::ptr_eq(&live, bytes) {
                return Some(value.digest);
            }
        }
        None
    }
    fn insert(&mut self, bytes: &Arc<Vec<u8>>, digest: blake3::Hash) {
        // A concurrent worker may already have inserted this independently computed digest.
        if self.get(bytes).is_some() {
            return;
        }
        let free = self.entries.iter().position(Option::is_none);
        let slot = free.unwrap_or(self.next);
        if let Some(entry) = self.entries.get_mut(slot) {
            *entry = Some(Entry { bytes: Arc::downgrade(bytes), length: bytes.len(), digest });
        }
        self.next = if slot + 1 == CAPACITY { 0 } else { slot + 1 };
    }
    fn invalidate(&mut self, bytes: &Arc<Vec<u8>>) {
        for entry in &mut self.entries {
            let remove = entry.as_ref().is_some_and(|value| value.bytes.upgrade().is_none_or(|live| Arc::ptr_eq(&live, bytes)));
            if remove {
                *entry = None;
            }
        }
    }
}
fn storage() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new()))
}
fn lock() -> MutexGuard<'static, Cache> {
    match storage().lock() {
        Ok(guard) => guard,
        Err(error) => {
            let mut guard = error.into_inner();
            guard.clear();
            guard
        }
    }
}
fn poll(control: &Interrupt<'_>) -> Result<(), XcfValidationError> {
    control.check().map_err(|_| XcfValidationError::new("cancelled", "retained byte digest cancelled"))
}
/// Explicitly discard all digest hints. This never mutates or retains payloads.
pub fn clear() {
    lock().clear();
}
/// Discard a live allocation's digest hint (and dead hints); no address-only match.
pub fn invalidate(bytes: &Arc<Vec<u8>>) {
    lock().invalidate(bytes);
}
/// Cache and resource callers retain descriptor, logical byte limits and
/// expected-hash/resolution grammar/comparison authority. A hit does not qualify
/// the source, resource kind, resolution fidelity, evaluator or a locator grant.
/// No cancellation callback is invoked while the cache mutex is held.
pub(crate) fn checked_digest(bytes: &Arc<Vec<u8>>, control: &Interrupt<'_>) -> Result<blake3::Hash, XcfValidationError> {
    poll(control)?;
    let found = { lock().get(bytes) };
    poll(control)?;
    if let Some(digest) = found {
        return Ok(digest);
    }
    let digest = crate::xcf_items::checked_payload_hash(bytes.as_slice(), control)?;
    poll(control)?;
    {
        lock().insert(bytes, digest);
    }
    poll(control)?;
    Ok(digest)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, PixelFormat, SampleType, xcf_items::SourceCache};
    fn source() -> SourceCache {
        let bytes = Arc::new(vec![13, 29, 0, 255]);
        SourceCache {
            blake3: blake3::hash(bytes.as_slice()).to_hex().to_string(),
            bytes,
            width: 4,
            height: 1,
            format: PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false),
        }
    }
    #[test]
    fn warm_digest_never_authenticates_a_forged_expected_hash() {
        let mut value = source();
        value.validate().unwrap();
        value.blake3 = "0".repeat(64);
        assert_eq!(value.validate().unwrap_err().code, "payload_hash");
        value.blake3 = "bad".into();
        assert_eq!(value.validate().unwrap_err().code, "payload_hash");
    }
    #[test]
    fn explicit_cow_and_unwrap_cannot_reuse_an_old_weak_digest() {
        let old = source();
        old.validate().unwrap();
        let original = old.bytes.clone();
        let mut changed = old.clone();
        Arc::make_mut(&mut changed.bytes)[0] = 42;
        assert!(changed.validate().is_err());
        old.validate().unwrap();
        assert_eq!(original.as_slice(), [13, 29, 0, 255]);
        changed.blake3 = blake3::hash(changed.bytes.as_slice()).to_hex().to_string();
        changed.validate().unwrap();
        let alone = source();
        alone.validate().unwrap();
        let weak = Arc::downgrade(&alone.bytes);
        let mut bytes = Arc::try_unwrap(alone.bytes).unwrap();
        assert!(weak.upgrade().is_none());
        bytes[0] = 99;
        assert_ne!(checked_digest(&Arc::new(bytes), &Interrupt::NONE).unwrap(), blake3::hash(&[13, 29, 0, 255]));
    }
    #[test]
    fn a_warm_hit_still_refuses_cancellation_and_bad_descriptor() {
        let mut value = source();
        value.validate().unwrap();
        let stop = || true;
        assert_eq!(value.validate_with(&Interrupt::cancel_only(&stop)).unwrap_err().code, "cancelled");
        value.width = 3;
        assert_eq!(value.validate().unwrap_err().code, "cache_bounds");
    }
    #[test]
    fn cache_is_bounded_weak_only_and_explicitly_invalidatable() {
        let mut state = Cache::new();
        let values: Vec<_> = (0u8..65).map(|n| Arc::new(vec![n])).collect();
        for bytes in &values {
            state.insert(bytes, blake3::hash(bytes.as_slice()));
        }
        assert_eq!(state.entries.iter().filter(|e| e.is_some()).count(), CAPACITY);
        assert!(values.iter().all(|bytes| Arc::strong_count(bytes) == 1));
        let bytes = values.last().unwrap();
        assert!(state.get(bytes).is_some());
        state.invalidate(bytes);
        assert!(state.get(bytes).is_none());
        state.clear();
        assert!(state.entries.iter().all(Option::is_none));
    }
}
