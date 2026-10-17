//! Authorized held tip input and bounded worker decode. No store lock spans I/O.
use super::*;
use crate::{EngineError, Result as EngineResult};
use photocraft_raster::Interrupt;
/// Owned authorized input. Memory snapshots are copied under the worker token;
/// file handles retain their original open authority and never reopen a locator.
pub trait PreparedPresetRead: Send {
    fn read_with_interrupt(&mut self, output: &mut [u8], c: &Interrupt<'_>) -> EngineResult<usize>;
}
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct HeldFile(pub std::fs::File);
#[cfg(not(target_arch = "wasm32"))]
impl PreparedPresetRead for HeldFile {
    fn read_with_interrupt(&mut self, output: &mut [u8], c: &Interrupt<'_>) -> EngineResult<usize> {
        poll(c)?;
        self.0.read(output).map_err(|e| error(e.to_string()))
    }
}
pub(super) struct HeldMemory {
    pub backend: MemBackend,
    pub name: String,
    pub max: u64,
    pub snapshot: Option<std::io::Cursor<Vec<u8>>>,
}
impl PreparedPresetRead for HeldMemory {
    fn read_with_interrupt(&mut self, output: &mut [u8], c: &Interrupt<'_>) -> EngineResult<usize> {
        poll(c)?;
        if self.snapshot.is_none() {
            let files = self.backend.lock().map_err(error)?;
            let bytes = files.get(&self.name).ok_or_else(|| error("prepared memory tip missing"))?;
            if bytes.len() as u64 > self.max {
                return Err(error("prepared memory tip size changed"));
            }
            let mut owned = Vec::new();
            owned.try_reserve_exact(bytes.len()).map_err(|_| error("memory tip allocation"))?;
            if owned.capacity() as u64 > self.max {
                return Err(error("memory tip capacity limit"));
            }
            for chunk in bytes.chunks(4096) {
                poll(c)?;
                owned.extend_from_slice(chunk);
            }
            poll(c)?;
            self.snapshot = Some(std::io::Cursor::new(owned));
        }
        self.snapshot.as_mut().ok_or_else(|| error("memory tip snapshot missing"))?.read(output).map_err(|e| error(e.to_string()))
    }
}
#[derive(Clone, Copy)]
pub(crate) struct TipPreparationLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for TipPreparationLimits {
    fn default() -> Self {
        Self { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}
fn error(m: impl Into<String>) -> EngineError {
    EngineError::Other(m.into())
}
fn poll(c: &Interrupt<'_>) -> EngineResult<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn add(a: u64, b: u64) -> EngineResult<u64> {
    a.checked_add(b).ok_or_else(|| error("tip accounting overflow"))
}
fn mul(a: u64, b: u64) -> EngineResult<u64> {
    a.checked_mul(b).ok_or_else(|| error("tip accounting overflow"))
}
pub(crate) struct TipStoreWitness(Arc<Mutex<TipCache>>);
impl TipStoreWitness {
    pub(crate) fn matches(&self, s: &Session) -> bool {
        s.preset_store.as_ref().is_some_and(|store| Arc::ptr_eq(&store.cache, &self.0))
    }
}
pub(crate) struct PreparedTip {
    key: String,
    cache: Arc<Mutex<TipCache>>,
    input: std::result::Result<Box<dyn PreparedPresetRead>, String>,
    pub(crate) held_bytes: u64,
    max_bytes: u64,
}
impl PresetStore {
    pub(crate) fn prepare_tip(&self, key: &str, max_encoded: u64, c: &Interrupt<'_>) -> EngineResult<PreparedTip> {
        poll(c)?;
        if key.len() > 128 || !valid_name(&tip_file(key)) {
            return Err(error("invalid stored tip key"));
        }
        let mut owned = String::new();
        owned.try_reserve_exact(key.len()).map_err(|_| error("tip key allocation"))?;
        owned.push_str(key);
        // Preserve an unused candidate's error until the final recipe requests it.
        // This matters when a patch replaces a missing base stored tip.
        let input = if !self.tips.contains_key(key) {
            Err(format!("brush tip {key} is not in the preset store"))
        } else {
            self.backend.prepared_read(&tip_file(key), MAX_TIP_BYTES.min(max_encoded), c)
        };
        let (input, held_bytes) = match input {
            Ok((input, bytes)) => (Ok(input), bytes),
            Err(e) => (Err(e), 0),
        };
        poll(c)?;
        Ok(PreparedTip { key: owned, cache: Arc::clone(&self.cache), input, held_bytes, max_bytes: MAX_TIP_BYTES.min(max_encoded) })
    }
}
impl PreparedTip {
    pub(crate) fn key(&self) -> &str {
        &self.key
    }
    pub(crate) fn witness(&self) -> TipStoreWitness {
        TipStoreWitness(Arc::clone(&self.cache))
    }
    pub(crate) fn decode(&mut self, width: u32, height: u32, limits: TipPreparationLimits, c: &Interrupt<'_>) -> EngineResult<Arc<GrayTile>> {
        poll(c)?;
        let standard = TipPreparationLimits::default();
        if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
            return Err(error("tip limits are lower-only"));
        }
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            poll(c)?;
            if let Some((tile, _)) = cache.entries.get(&self.key) {
                let tile = Arc::clone(tile);
                if (tile.width, tile.height) != (width, height) {
                    return Err(error("stored tip cache dimensions changed"));
                }
                cache.tick = cache.tick.checked_add(1).ok_or_else(|| error("tip cache clock exhausted"))?;
                let tick = cache.tick;
                if let Some((_, used)) = cache.entries.get_mut(&self.key) {
                    *used = tick;
                }
                poll(c)?;
                return Ok(tile);
            }
        }
        let mut input = std::mem::replace(&mut self.input, Err("prepared tip input was already consumed".into())).map_err(error)?;
        let mut encoded = Vec::new();
        let mut block = [0u8; 8192];
        loop {
            poll(c)?;
            let n = input.read_with_interrupt(&mut block, c)?;
            if n == 0 {
                break;
            }
            let next = encoded.len().checked_add(n).ok_or_else(|| error("tip input length"))?;
            if next as u64 > self.max_bytes || next as u64 > limits.resident_bytes {
                return Err(error("tip encoded byte limit"));
            }
            encoded.try_reserve(n).map_err(|_| error("tip encoded allocation"))?;
            if encoded.capacity() as u64 > limits.resident_bytes {
                return Err(error("tip encoded capacity limit"));
            }
            encoded.extend_from_slice(block.get(..n).ok_or_else(|| error("tip read length"))?);
        }
        let tile = decode_controlled(&encoded, encoded.capacity() as u64, width, height, limits, c)?;
        let tile = Arc::new(tile);
        poll(c)?;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        poll(c)?;
        // Recheck after decoding outside the mutex. A concurrent fill wins, but
        // completed decodes remain counted just as an actual decoder invocation.
        let decodes = cache.decodes.checked_add(1).ok_or_else(|| error("tip decode counter exhausted"))?;
        let tick = cache.tick.checked_add(1).ok_or_else(|| error("tip cache clock exhausted"))?;
        let held = mul(cache.entries.capacity() as u64, 256)?;
        let scan = mul(cache.entries.len() as u64, 64)?;
        if add(encoded.capacity() as u64, add(held, add(cache.bytes, mul(tile_bytes(&tile), 2)?)?)?)? > limits.resident_bytes || scan > limits.work_units {
            return Err(error("tip cache admission limit"));
        }
        if let Some((old, _)) = cache.entries.get(&self.key) {
            let old = Arc::clone(old);
            if (old.width, old.height) != (width, height) {
                return Err(error("stored tip cache dimensions changed"));
            }
            cache.decodes = decodes;
            cache.tick = tick;
            if let Some((_, used)) = cache.entries.get_mut(&self.key) {
                *used = tick;
            }
            return Ok(old);
        }
        cache.entries.try_reserve(1).map_err(|_| error("tip cache allocation"))?;
        if add(encoded.capacity() as u64, add(mul(cache.entries.capacity() as u64, 256)?, add(cache.bytes, mul(tile_bytes(&tile), 2)?)?)?)?
            > limits.resident_bytes
        {
            return Err(error("tip cache capacity limit"));
        }
        // Stage eviction names before mutation. Original least-used policy/order;
        // at most existing entry count scans, with aggregate quadratic work paid.
        let work = mul(mul(mul(cache.entries.len() as u64, cache.entries.len() as u64)?, cache.entries.len() as u64 + 1)?, 128)?;
        if work > limits.work_units {
            return Err(error("tip cache eviction work limit"));
        }
        let retirement_bytes = mul(cache.entries.len() as u64, (std::mem::size_of::<String>() + 128) as u64)?;
        if add(
            encoded.capacity() as u64,
            add(retirement_bytes, add(mul(cache.entries.capacity() as u64, 256)?, add(cache.bytes, mul(tile_bytes(&tile), 2)?)?)?)?,
        )? > limits.resident_bytes
        {
            return Err(error("tip eviction storage limit"));
        }
        for key in cache.entries.keys() {
            poll(c)?;
            if key.len() > 128 {
                return Err(error("tip cache key limit"));
            }
        }
        let mut retired = Vec::new();
        retired.try_reserve_exact(cache.entries.len()).map_err(|_| error("tip eviction allocation"))?;
        if retired.capacity() > cache.entries.len() {
            return Err(error("tip eviction capacity limit"));
        }
        let mut bytes = add(cache.bytes, tile_bytes(&tile))?;
        while bytes > cache.limit {
            poll(c)?;
            let mut oldest: Option<(&String, u64)> = None;
            for (key, (value, used)) in &cache.entries {
                poll(c)?;
                if retired.iter().any(|r: &String| r == key) {
                    continue;
                }
                if oldest.is_none_or(|(_, u)| *used < u) {
                    oldest = Some((key, *used));
                }
                let _ = value;
            }
            let Some((key, _)) = oldest else {
                break;
            };
            let value = cache.entries.get(key).ok_or_else(|| error("tip eviction entry"))?;
            bytes = bytes.saturating_sub(tile_bytes(&value.0));
            let mut key_copy = String::new();
            key_copy.try_reserve_exact(key.len()).map_err(|_| error("tip eviction key allocation"))?;
            key_copy.push_str(key);
            retired.push(key_copy);
        }
        let mut owned_key = String::new();
        owned_key.try_reserve_exact(self.key.len()).map_err(|_| error("tip cache key allocation"))?;
        owned_key.push_str(&self.key);
        poll(c)?;
        cache.entries.insert(owned_key, (Arc::clone(&tile), tick));
        for key in retired {
            cache.entries.remove(&key);
        }
        cache.bytes = bytes;
        cache.decodes = decodes;
        cache.tick = tick;
        Ok(tile)
    }
}
fn decode_controlled(encoded: &[u8], encoded_capacity: u64, width: u32, height: u32, l: TipPreparationLimits, c: &Interrupt<'_>) -> EngineResult<GrayTile> {
    poll(c)?;
    let header = encoded.get(..TIP_HEADER).ok_or_else(|| error("truncated tip header"))?;
    if header.get(..6) != Some(TIP_MAGIC.as_slice()) {
        return Err(error("not a PhotoCraft tip"));
    }
    let read = |offset| -> EngineResult<u32> {
        let bytes: [u8; 4] =
            header.get(offset..offset + 4).ok_or_else(|| error("tip dimension header"))?.try_into().map_err(|_| error("tip dimension header"))?;
        Ok(u32::from_le_bytes(bytes))
    };
    let (w, h) = (read(7)?, read(11)?);
    if w == 0 || h == 0 || w > MAX_TIP_SIDE || h > MAX_TIP_SIDE || (w, h) != (width, height) {
        return Err(error("stored tip dimensions mismatch"));
    }
    let count = mul(u64::from(w), u64::from(h))?;
    if count > MAX_TIP_SAMPLES {
        return Err(error("tip sample limit"));
    }
    let bits = header.get(6).copied().ok_or_else(|| error("tip depth"))?;
    let per = match bits {
        8 => 1,
        16 => 2,
        _ => return Err(error("unsupported tip depth")),
    };
    let want = mul(count, per)?;
    let work = add(mul(encoded.len() as u64, 16)?, mul(count, 32)?)?;
    let live = add(encoded_capacity, add(mul(want, 2)?, mul(count, 4)?)?)?;
    if work > l.work_units || live > l.resident_bytes {
        return Err(error("tip decode budget"));
    }
    let mut raw = Vec::new();
    raw.try_reserve_exact(usize::try_from(want).map_err(|_| error("tip raw length"))?).map_err(|_| error("tip raw allocation"))?;
    if add(encoded_capacity, add(raw.capacity() as u64, mul(count, 4)?)?)? > l.resident_bytes {
        return Err(error("tip actual raw capacity"));
    }
    let mut decoder = flate2::read::ZlibDecoder::new(encoded.get(TIP_HEADER..).ok_or_else(|| error("tip data offset"))?);
    let mut block = [0u8; 8192];
    loop {
        poll(c)?;
        let n = decoder.read(&mut block).map_err(|e| error(format!("tip data: {e}")))?;
        if n == 0 {
            break;
        }
        let next = raw.len().checked_add(n).ok_or_else(|| error("tip decoded length"))?;
        if next as u64 > want {
            return Err(error("tip decoded size mismatch"));
        }
        raw.extend_from_slice(block.get(..n).ok_or_else(|| error("tip decoder length"))?);
    }
    if raw.len() as u64 != want {
        return Err(error("tip decoded size mismatch"));
    }
    let mut data = Vec::new();
    data.try_reserve_exact(usize::try_from(count).map_err(|_| error("tip samples length"))?).map_err(|_| error("tip sample allocation"))?;
    if add(encoded_capacity, add(raw.capacity() as u64, mul(data.capacity() as u64, 4)?)?)? > l.resident_bytes {
        return Err(error("tip actual decode capacity"));
    }
    for chunk in raw.chunks(4096) {
        poll(c)?;
        if bits == 8 {
            for v in chunk {
                data.push(u16::from(*v) * 257);
            }
        } else {
            for v in chunk.as_chunks::<2>().0.iter() {
                let bytes: [u8; 2] = *v;
                data.push(u16::from_le_bytes(bytes));
            }
        }
    }
    poll(c)?;
    Ok(GrayTile { width: w, height: h, data: data.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_decode_matches_original_eight_and_sixteen_bit_codec_and_cache_identity() {
        for data in [vec![0, 257, 65535, 257 * 73], vec![0, 1, 65535, 17329]] {
            let tile = GrayTile { width: 2, height: 2, data: data.into() };
            let encoded = encode_tip(&tile);
            let expected = decode_tip(&encoded).unwrap();
            let backend = MemBackend::default();
            let key = tip_hash(&tile);
            backend.write(&tip_file(&key), &encoded).unwrap();
            let mut store = open(Box::new(backend)).store;
            store.tips.insert(key.clone(), encoded.len() as u64);
            let mut input = store.prepare_tip(&key, MAX_TIP_BYTES, &Interrupt::NONE).unwrap();
            let first = input.decode(2, 2, TipPreparationLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(&*first, &expected);
            assert_eq!(store.tip_cache().decodes, 1);
            let second = input.decode(2, 2, TipPreparationLimits::default(), &Interrupt::NONE).unwrap();
            assert!(Arc::ptr_eq(&first, &second));
            assert_eq!(store.tip_cache().decodes, 1);
        }
    }
    #[test]
    fn cancelled_held_memory_read_and_truncated_decode_publish_no_tip() {
        let tile = GrayTile { width: 2, height: 2, data: vec![1, 2, 3, 4].into() };
        let encoded = encode_tip(&tile);
        let backend = MemBackend::default();
        let key = tip_hash(&tile);
        backend.write(&tip_file(&key), &encoded).unwrap();
        let mut store = open(Box::new(backend.clone())).store;
        store.tips.insert(key.clone(), encoded.len() as u64);
        let mut input = store.prepare_tip(&key, MAX_TIP_BYTES, &Interrupt::NONE).unwrap();
        let cancel = || true;
        assert!(matches!(input.decode(2, 2, TipPreparationLimits::default(), &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert_eq!(store.tip_cache().decodes, 0);
        assert_eq!(store.tip_cache().tips, 0);
        let bad = &encoded[..TIP_HEADER];
        backend.write(&tip_file(&key), bad).unwrap();
        let mut input = store.prepare_tip(&key, MAX_TIP_BYTES, &Interrupt::NONE).unwrap();
        assert!(input.decode(2, 2, TipPreparationLimits::default(), &Interrupt::NONE).is_err());
        assert_eq!(store.tip_cache().tips, 0);
    }
}
