//! Optional Loader construction admission. Legacy Fetch implementations keep
//! their previous behavior. The owned route supplies a persistent model owner.
//! Complex metadata uses the same bounded logical clone envelope as save
//! preparation; this is not an allocator/RSS census.
//! Raw StoredColor is serialized in the separately bounded XCF state blob,
//! loaded through its existing byte-scaled model reservation and chunked parser.
//! This helper's unchanged 64 KiB limit applies to cloned manifest metadata,
//! not to that XCF state-blob path. No storage limit is raised by the new variant.
use crate::{FormatError, Result, convert::Fetch};
use std::io::Write;

pub(crate) fn cloned<T: Clone + serde::Serialize>(value: &T, fetch: &mut dyn Fetch) -> Result<T> {
    if !fetch.accounts_model() {
        return Ok(value.clone());
    }
    struct Counter<'a> {
        fetch: &'a mut dyn Fetch,
        bytes: usize,
    }
    impl Write for Counter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let next = self.bytes.checked_add(bytes.len()).ok_or_else(|| std::io::Error::other("metadata size overflow"))?;
            if next > 64 << 10 {
                return Err(std::io::Error::other("owned metadata exceeds bounded clone admission"));
            }
            self.fetch.model_poll(bytes.len() as u64).map_err(std::io::Error::other)?;
            self.bytes = next;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { fetch, bytes: 0 };
    serde_json::to_writer(&mut counter, value)?;
    let estimate = counter
        .bytes
        .checked_mul(64)
        .and_then(|n| n.checked_add(std::mem::size_of::<T>()))
        .ok_or_else(|| FormatError::LimitExceeded("owned metadata clone envelope".into()))?;
    counter.fetch.model_reserve(estimate as u64)?;
    counter.fetch.model_poll(0)?;
    let result = value.clone();
    counter.fetch.model_poll(0)?;
    Ok(result)
}

pub(crate) fn vector<T>(count: usize, fetch: &mut dyn Fetch) -> Result<Vec<T>> {
    if !fetch.accounts_model() {
        return Ok(Vec::with_capacity(count));
    }
    fetch.model_poll(0)?;
    let size = std::mem::size_of::<T>();
    let envelope = count.checked_add(16).and_then(|n| n.checked_mul(size)).ok_or_else(|| FormatError::LimitExceeded("owned model vector envelope".into()))?;
    fetch.model_reserve(envelope as u64)?;
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| FormatError::LimitExceeded("owned model vector allocation".into()))?;
    let actual = result.capacity().checked_mul(size).ok_or_else(|| FormatError::LimitExceeded("owned model vector capacity".into()))?;
    if actual > envelope {
        return Err(FormatError::LimitExceeded("owned model vector exceeded prepaid capacity".into()));
    }
    fetch.model_poll(0)?;
    Ok(result)
}
