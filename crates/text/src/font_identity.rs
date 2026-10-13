//! Byte identity only: no name lookup, registration, parsing, font fallback or
//! renderer qualification. Sources are explicitly supplied immutable allocations.
use photocraft_raster::Interrupt;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex, Weak};

const MAX_BYTES: usize = 16 << 20;
const MAX_ENTRIES: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontIdentityError {
    InputLimit,
    IdentityMismatch,
    Cancelled,
    Allocation,
}
impl std::fmt::Display for FontIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "authorized font byte identity: {self:?}")
    }
}
impl std::error::Error for FontIdentityError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontByteIdentity {
    digest: [u8; 32],
    length: usize,
}
impl FontByteIdentity {
    pub fn sha256(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn byte_len(&self) -> usize {
        self.length
    }
}
struct Entry {
    bytes: Weak<Vec<u8>>,
    identity: FontByteIdentity,
}
#[derive(Default)]
pub struct FontIdentityCache {
    entries: Vec<Entry>,
}
fn poll(control: &Interrupt<'_>) -> Result<(), FontIdentityError> {
    control.check().map_err(|_| FontIdentityError::Cancelled)
}
fn digest(bytes: &[u8], control: &Interrupt<'_>) -> Result<FontByteIdentity, FontIdentityError> {
    poll(control)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(FontIdentityError::InputLimit);
    }
    let mut digest = Sha256::new();
    for chunk in bytes.chunks(4096) {
        poll(control)?;
        digest.update(chunk);
    }
    let digest: [u8; 32] = digest.finalize().into();
    poll(control)?;
    Ok(FontByteIdentity { digest, length: bytes.len() })
}
fn matches(identity: FontByteIdentity, expected: &[u8; 32], control: &Interrupt<'_>) -> Result<FontByteIdentity, FontIdentityError> {
    poll(control)?;
    if &identity.digest != expected {
        return Err(FontIdentityError::IdentityMismatch);
    }
    poll(control)?;
    Ok(identity)
}
impl FontIdentityCache {
    /// Invalidate all supplied-byte identities; no source allocation is retained.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
    /// Pointer equality is used only after upgrading a live Weak owner. Safe Arc
    /// mutation either fails while another strong owner exists or disconnects
    /// Weak owners through make_mut; a reused address cannot forge a cache hit.
    /// Every hit still compares the independently computed digest to expected.
    pub fn identify(&mut self, bytes: &Arc<Vec<u8>>, expected: &[u8; 32], control: &Interrupt<'_>) -> Result<FontByteIdentity, FontIdentityError> {
        poll(control)?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(FontIdentityError::InputLimit);
        }
        for entry in &self.entries {
            poll(control)?;
            if let Some(source) = entry.bytes.upgrade()
                && Arc::ptr_eq(&source, bytes)
                && entry.identity.length == bytes.len()
            {
                return matches(entry.identity, expected, control);
            }
        }
        let identity = matches(digest(bytes, control)?, expected, control)?;
        // Only successful uncancelled proofs enter the bounded cache. Dead Weak
        // owners are removed; content is never reread through an ambient locator.
        self.entries.retain(|entry| entry.bytes.strong_count() != 0);
        poll(control)?;
        if self.entries.len() == MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.try_reserve(1).map_err(|_| FontIdentityError::Allocation)?;
        poll(control)?;
        self.entries.push(Entry { bytes: Arc::downgrade(bytes), identity });
        Ok(identity)
    }
}

// Only the immutable compiled Inter byte region is eligible for this static
// cache. Arbitrary borrowed slices are always rehashed: their address/lifetime
// alone is not evidence that bytes remain unchanged between calls.
static BUNDLED_INTER: Mutex<Option<FontByteIdentity>> = Mutex::new(None);
pub fn invalidate_bundled_identity() {
    *BUNDLED_INTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}
pub fn identify_borrowed(bytes: &[u8], expected: &[u8; 32], control: &Interrupt<'_>) -> Result<FontByteIdentity, FontIdentityError> {
    poll(control)?;
    let bundled = std::ptr::eq(bytes, &super::INTER_REGULAR[..]);
    if bundled {
        // The lock never spans hashing or caller work: bounded scalar copy only.
        let cached = *BUNDLED_INTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(identity) = cached {
            return matches(identity, expected, control);
        }
    }
    let identity = matches(digest(bytes, control)?, expected, control)?;
    if bundled {
        poll(control)?;
        *BUNDLED_INTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(identity);
    }
    poll(control)?;
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn expected(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }
    #[test]
    fn mutable_owner_and_cancelled_hit_do_not_reuse_old_identity() {
        let mut bytes = Arc::new(vec![1, 2, 3]);
        let old = expected(&bytes);
        let mut cache = FontIdentityCache::default();
        assert_eq!(cache.identify(&bytes, &old, &Interrupt::NONE).unwrap().sha256(), &old);
        assert_eq!(cache.identify(&bytes, &old, &Interrupt::cancel_only(&|| true)), Err(FontIdentityError::Cancelled));
        Arc::make_mut(&mut bytes)[0] = 4;
        assert_eq!(cache.identify(&bytes, &old, &Interrupt::NONE), Err(FontIdentityError::IdentityMismatch));
        let changed = expected(&bytes);
        assert_eq!(cache.identify(&bytes, &changed, &Interrupt::NONE).unwrap().sha256(), &changed);
    }
    #[test]
    fn cache_bound_and_invalidation_are_explicit() {
        let mut cache = FontIdentityCache::default();
        let mut retained = Vec::new();
        for value in 0..100u8 {
            let bytes = Arc::new(vec![value]);
            cache.identify(&bytes, &expected(&bytes), &Interrupt::NONE).unwrap();
            retained.push(bytes);
        }
        assert_eq!(cache.entry_count(), MAX_ENTRIES);
        cache.clear();
        assert_eq!(cache.entry_count(), 0);
        assert_eq!(cache.identify(&Arc::new(Vec::new()), &[0; 32], &Interrupt::NONE), Err(FontIdentityError::InputLimit));
    }
    #[test]
    fn arbitrary_borrowed_same_address_is_rehashed() {
        let mut bytes = [1, 2, 3];
        let old = expected(&bytes);
        identify_borrowed(&bytes, &old, &Interrupt::NONE).unwrap();
        bytes[0] = 4;
        assert_eq!(identify_borrowed(&bytes, &old, &Interrupt::NONE), Err(FontIdentityError::IdentityMismatch));
    }
}
