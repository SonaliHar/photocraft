//! Seekable ZIP/ZIP64 STORE writer and bounded STORE/DEFLATE reader.
//! Layout follows PKWARE APPNOTE 6.3.10 sections 4.3 and 4.5.3.
//! This is container infrastructure, not qualification of the native_large XCF policy.
use crate::{FormatError, Result};
use photocraft_raster::Interrupt;
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
};

#[path = "zip_stream/accounted.rs"]
mod accounted;
#[path = "zip_stream/accounted_writer.rs"]
mod accounted_writer;
use accounted::{Accounting, OwnedVec};

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
const END64: u32 = 0x0606_4b50;
const LOCATOR: u32 = 0x0706_4b50;
const BLOCK: usize = 64 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_archive_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_entries: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { max_archive_bytes: 16 << 30, max_metadata_bytes: 64 << 20, max_entries: 1_000_000 }
    }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    if control.cancelled() { Err(FormatError::Cancelled) } else { Ok(()) }
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| FormatError::corrupt("ZIP64 offset overflow"))
}
fn field<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N]> {
    let end = offset.checked_add(N).ok_or_else(|| FormatError::corrupt("zip field overflow"))?;
    bytes.get(offset..end).and_then(|b| b.try_into().ok()).ok_or_else(|| FormatError::corrupt("truncated zip field"))
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(field(bytes, offset)?))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(field(bytes, offset)?))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(field(bytes, offset)?))
}
fn p16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn p32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\0', '\\', ':'])
        && !name.starts_with('/')
        && name
            .split('/')
            .enumerate()
            .all(|(i, part)| part != ".." && part != "." && (!part.is_empty() || i > 0 && name.ends_with('/') && i + 1 == name.split('/').count()))
}
struct Crc(u32);
impl Crc {
    fn new() -> Self {
        Self(u32::MAX)
    }
    fn update(&mut self, bytes: &[u8]) -> Result<()> {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            let mut table = [0; 256];
            for (index, entry) in table.iter_mut().enumerate() {
                let mut value = index as u32;
                for _ in 0..8 {
                    value = if value & 1 == 1 { 0xedb8_8320 ^ (value >> 1) } else { value >> 1 };
                }
                *entry = value;
            }
            table
        });
        for byte in bytes {
            let index = ((self.0 ^ u32::from(*byte)) & 255) as usize;
            let value = table.get(index).ok_or_else(|| FormatError::corrupt("CRC index"))?;
            self.0 = *value ^ (self.0 >> 8);
        }
        Ok(())
    }
    fn finish(self) -> u32 {
        !self.0
    }
}

/// The output must be a new empty stream. A failed entry poisons the writer.
pub struct Writer<W> {
    output: W,
    central: Vec<u8>,
    names: BTreeSet<String>,
    metadata: u64,
    limits: Limits,
    failed: bool,
    accounted: Option<accounted_writer::State>,
}
impl<W: Write + Seek> Writer<W> {
    /// Internal producer guard, before any object entry I/O. Does not mutate the
    /// writer or grant output/source authority.
    pub(crate) fn validate_accounted_quota(&self, quota: &photocraft_raster::backing::quota::BackingQuota) -> Result<()> {
        if self.failed {
            return Err(FormatError::corrupt("streaming zip writer has a failed entry"));
        }
        let state = self.accounted.as_ref().ok_or_else(|| FormatError::Unsupported("accounted object requires an accounted writer".into()))?;
        if !state.shares_quota(quota) {
            return Err(FormatError::LimitExceeded("canonical object and ZIP writer require the same quota ledger".into()));
        }
        Ok(())
    }
    /// Optional shared-ledger constructor. Output storage remains caller-owned.
    pub fn new_accounted(
        mut output: W,
        limits: Limits,
        control: &photocraft_raster::backing::ReadControl<'_>,
        quota: &photocraft_raster::backing::quota::BackingQuota,
    ) -> Result<Self> {
        let bytes = (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(|| FormatError::corrupt("ZIP writer owner size"))?;
        let state = accounted_writer::State::new(quota, control, bytes)?;
        if output.seek(SeekFrom::End(0))? != 0 {
            return Err(FormatError::Unsupported("streaming zip output must be empty".into()));
        }
        accounted::charge(Some((control, quota)), control.interrupt, 0)?;
        Ok(Self { output, central: Vec::new(), names: BTreeSet::new(), metadata: 0, limits, failed: false, accounted: Some(state) })
    }
    pub fn add_accounted(&mut self, name: &str, size: u64, input: &mut impl Read, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<()> {
        if self.failed {
            return Err(FormatError::corrupt("streaming zip writer has a failed entry"));
        }
        let result = self
            .accounted
            .as_mut()
            .ok_or_else(|| FormatError::Unsupported("accounted add requires an accounted writer".into()))
            .and_then(|state| state.add(&mut self.output, name, size, input, self.limits, control));
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub fn finish_accounted(mut self, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<W> {
        if self.failed {
            return Err(FormatError::corrupt("streaming zip writer has a failed entry"));
        }
        self.accounted.as_ref().ok_or_else(|| FormatError::Unsupported("accounted finish requires an accounted writer".into()))?.finish(
            &mut self.output,
            self.limits,
            control,
        )?;
        Ok(self.output)
    }
    pub fn new(mut output: W, limits: Limits) -> Result<Self> {
        if output.seek(SeekFrom::End(0))? != 0 {
            return Err(FormatError::Unsupported("streaming zip output must be empty".into()));
        }
        Ok(Self { output, central: Vec::new(), names: BTreeSet::new(), metadata: 0, limits, failed: false, accounted: None })
    }
    pub fn add(&mut self, name: &str, size: u64, input: &mut impl Read, control: &Interrupt<'_>) -> Result<()> {
        if self.accounted.is_some() {
            self.failed = true;
            return Err(FormatError::Unsupported("accounted writer requires add_accounted".into()));
        }
        if self.failed {
            return Err(FormatError::corrupt("streaming zip writer has a failed entry"));
        }
        let result = self.add_inner(name, size, input, control);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn add_inner(&mut self, name: &str, size: u64, input: &mut impl Read, control: &Interrupt<'_>) -> Result<()> {
        poll(control)?;
        let name_length = u16::try_from(name.len()).map_err(|_| FormatError::LimitExceeded("zip name too long".into()))?;
        if !valid_name(name) || self.names.contains(name) {
            return Err(FormatError::corrupt("invalid or duplicate zip name"));
        }
        if self.names.len() as u64 >= self.limits.max_entries {
            return Err(FormatError::LimitExceeded("zip entry count".into()));
        }
        let metadata = add(self.metadata, add(name.len() as u64 * 2, 160)?)?;
        if metadata > self.limits.max_metadata_bytes {
            return Err(FormatError::LimitExceeded("zip index metadata".into()));
        }
        let offset = self.output.stream_position()?;
        let payload = add(offset, add(50, name.len() as u64)?)?;
        let end = add(payload, size)?;
        // Reserve central-directory and terminator space before writing any payload.
        if add(end, add(metadata, 98)?)? > self.limits.max_archive_bytes {
            return Err(FormatError::LimitExceeded("zip output bytes".into()));
        }
        self.central.try_reserve_exact(74 + name.len()).map_err(|_| FormatError::LimitExceeded("zip index allocation".into()))?;
        let mut header = Vec::with_capacity(50 + name.len());
        p32(&mut header, LOCAL);
        p16(&mut header, 45);
        p16(&mut header, 0x0800);
        p16(&mut header, 0);
        p16(&mut header, 0);
        p16(&mut header, 0x21);
        p32(&mut header, 0);
        p32(&mut header, u32::MAX);
        p32(&mut header, u32::MAX);
        p16(&mut header, name_length);
        p16(&mut header, 20);
        header.extend_from_slice(name.as_bytes());
        p16(&mut header, 1);
        p16(&mut header, 16);
        p64(&mut header, size);
        p64(&mut header, size);
        self.output.write_all(&header)?;
        let mut buffer = [0; BLOCK];
        let mut remaining = size;
        let mut crc = Crc::new();
        while remaining != 0 {
            poll(control)?;
            let count = remaining.min(BLOCK as u64) as usize;
            let block = buffer.get_mut(..count).ok_or_else(|| FormatError::corrupt("zip buffer range"))?;
            input.read_exact(block)?;
            crc.update(block)?;
            self.output.write_all(block)?;
            remaining -= count as u64;
        }
        poll(control)?;
        let mut extra = [0];
        if input.read(&mut extra)? != 0 {
            return Err(FormatError::corrupt("zip input exceeds declared size"));
        }
        let crc = crc.finish();
        self.output.seek(SeekFrom::Start(add(offset, 14)?))?;
        self.output.write_all(&crc.to_le_bytes())?;
        self.output.seek(SeekFrom::Start(end))?;
        let central = &mut self.central;
        p32(central, CENTRAL);
        p16(central, 45);
        p16(central, 45);
        p16(central, 0x0800);
        p16(central, 0);
        p16(central, 0);
        p16(central, 0x21);
        p32(central, crc);
        p32(central, u32::MAX);
        p32(central, u32::MAX);
        p16(central, name_length);
        p16(central, 28);
        p16(central, 0);
        p16(central, 0);
        p16(central, 0);
        p32(central, 0);
        p32(central, u32::MAX);
        central.extend_from_slice(name.as_bytes());
        p16(central, 1);
        p16(central, 24);
        p64(central, size);
        p64(central, size);
        p64(central, offset);
        self.names.insert(name.into());
        self.metadata = metadata;
        Ok(())
    }
    pub fn finish(mut self, control: &Interrupt<'_>) -> Result<W> {
        if self.accounted.is_some() {
            return Err(FormatError::Unsupported("accounted writer requires finish_accounted".into()));
        }
        poll(control)?;
        if self.failed {
            return Err(FormatError::corrupt("streaming zip writer has a failed entry"));
        }
        let offset = self.output.stream_position()?;
        let size = self.central.len() as u64;
        let end64_offset = add(offset, size)?;
        if add(end64_offset, 98)? > self.limits.max_archive_bytes {
            return Err(FormatError::LimitExceeded("zip output bytes".into()));
        }
        for block in self.central.chunks(BLOCK) {
            poll(control)?;
            self.output.write_all(block)?;
        }
        let mut end = Vec::with_capacity(98);
        p32(&mut end, END64);
        p64(&mut end, 44);
        p16(&mut end, 45);
        p16(&mut end, 45);
        p32(&mut end, 0);
        p32(&mut end, 0);
        p64(&mut end, self.names.len() as u64);
        p64(&mut end, self.names.len() as u64);
        p64(&mut end, size);
        p64(&mut end, offset);
        p32(&mut end, LOCATOR);
        p32(&mut end, 0);
        p64(&mut end, end64_offset);
        p32(&mut end, 1);
        p32(&mut end, END);
        p16(&mut end, 0);
        p16(&mut end, 0);
        p16(&mut end, u16::MAX);
        p16(&mut end, u16::MAX);
        p32(&mut end, u32::MAX);
        p32(&mut end, u32::MAX);
        p16(&mut end, 0);
        self.output.write_all(&end)?;
        self.output.flush()?;
        poll(control)?;
        Ok(self.output)
    }
}

#[derive(Debug)]
struct Entry {
    name: String,
    _name_lease: Option<photocraft_raster::backing::quota::QuotaLease>,
    offset: u64,
    compressed: u64,
    uncompressed: u64,
    method: u16,
    crc: u32,
}
pub struct Reader<R> {
    input: R,
    entries: OwnedVec<Entry>,
    _owner_lease: Option<photocraft_raster::backing::quota::QuotaLease>,
}

/// Read bounded metadata/data without ever collecting the archive.
fn read_at(
    input: &mut (impl Read + Seek),
    offset: u64,
    length: u64,
    file_length: u64,
    control: &Interrupt<'_>,
    accounting: Accounting<'_, '_>,
) -> Result<OwnedVec<u8>> {
    if add(offset, length)? > file_length {
        return Err(FormatError::corrupt("zip record out of range"));
    }
    accounted::charge(accounting, control, 1)?;
    let mut bytes = OwnedVec::bytes(length, accounting, control)?;
    input.seek(SeekFrom::Start(offset))?;
    for block in bytes.chunks_mut(BLOCK) {
        accounted::charge(accounting, control, block.len() as u64)?;
        input.read_exact(block)?;
        accounted::charge(accounting, control, 0)?;
    }
    Ok(bytes)
}
fn extended(extra: &[u8], need: [bool; 3]) -> Result<[Option<u64>; 3]> {
    let mut position = 0usize;
    let mut found = None;
    while position < extra.len() {
        let id = u16_at(extra, position)?;
        let length = usize::from(u16_at(extra, position.checked_add(2).ok_or_else(|| FormatError::corrupt("zip extra offset"))?)?);
        let start = position.checked_add(4).ok_or_else(|| FormatError::corrupt("zip extra offset"))?;
        let end = start.checked_add(length).ok_or_else(|| FormatError::corrupt("zip extra size"))?;
        let payload = extra.get(start..end).ok_or_else(|| FormatError::corrupt("truncated zip extra"))?;
        if id == 1 {
            if found.is_some() {
                return Err(FormatError::corrupt("duplicate ZIP64 extra"));
            }
            found = Some(payload);
        }
        position = end;
    }
    let mut result = [None; 3];
    let mut offset = 0usize;
    for (target, required) in result.iter_mut().zip(need) {
        if required {
            *target = Some(u64_at(found.ok_or_else(|| FormatError::corrupt("missing ZIP64 extra"))?, offset)?);
            offset = offset.checked_add(8).ok_or_else(|| FormatError::corrupt("ZIP64 extra offset"))?;
        }
    }
    Ok(result)
}
fn resolved(ordinary: u32, extended: Option<u64>) -> Result<u64> {
    if ordinary == u32::MAX { extended.ok_or_else(|| FormatError::corrupt("missing ZIP64 value")) } else { Ok(u64::from(ordinary)) }
}
impl<R: Read + Seek> Reader<R> {
    /// Validated STORE payload length without reading or constructing a decoder.
    /// This metadata query grants no storage authority. Normal copy remains unchanged.
    pub fn stored_size(&self, name: &str) -> Result<u64> {
        let entry = self.entries.iter().find(|entry| entry.name == name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?;
        if entry.method != 0 {
            return Err(FormatError::Unsupported("canonical object requires ZIP STORE".into()));
        }
        Ok(entry.uncompressed)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }
    /// Logical metadata capacities are charged before allocation; retained index/name
    /// leases travel with Reader. Input ownership and per-copy decode/output budgets
    /// remain caller responsibilities. No operation callbacks are stored.
    pub fn new_accounted(
        input: R,
        limits: Limits,
        control: &photocraft_raster::backing::ReadControl<'_>,
        quota: &photocraft_raster::backing::quota::BackingQuota,
    ) -> Result<Self> {
        Self::new_inner(input, limits, control.interrupt, Some((control, quota)))
    }
    pub fn new(input: R, limits: Limits, control: &Interrupt<'_>) -> Result<Self> {
        Self::new_inner(input, limits, control, None)
    }
    fn new_inner(mut input: R, limits: Limits, control: &Interrupt<'_>, accounting: Accounting<'_, '_>) -> Result<Self> {
        accounted::charge(accounting, control, 1)?;
        let owner_lease = accounted::reserve(
            accounting,
            (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or_else(|| FormatError::LimitExceeded("ZIP owner metadata".into()))?,
        )?;
        poll(control)?;
        let length = input.seek(SeekFrom::End(0))?;
        if length > limits.max_archive_bytes {
            return Err(FormatError::LimitExceeded("zip archive bytes".into()));
        }
        if length < 22 {
            return Err(FormatError::corrupt("zip archive too short"));
        }
        let tail_start = length.saturating_sub(22 + u16::MAX as u64);
        if length - tail_start > limits.max_metadata_bytes {
            return Err(FormatError::LimitExceeded("zip tail metadata".into()));
        }
        let tail = read_at(&mut input, tail_start, length - tail_start, length, control, accounting)?;
        let end = (0..=tail.len().saturating_sub(22))
            .rev()
            .find(|offset| {
                u32_at(&tail, *offset).ok() == Some(END)
                    && u16_at(&tail, offset.saturating_add(20)).ok().is_some_and(|comment| offset.checked_add(22 + usize::from(comment)) == Some(tail.len()))
            })
            .ok_or_else(|| FormatError::corrupt("zip end record not found or trailing data"))?;
        if u16_at(&tail, end + 4)? != 0 || u16_at(&tail, end + 6)? != 0 || u16_at(&tail, end + 8)? != u16_at(&tail, end + 10)? {
            return Err(FormatError::Unsupported("multi-disk zip".into()));
        }
        let end_offset = add(tail_start, end as u64)?;
        let count32 = u16_at(&tail, end + 10)?;
        let size32 = u32_at(&tail, end + 12)?;
        let offset32 = u32_at(&tail, end + 16)?;
        let (count, directory_size, directory_offset, terminator) = if count32 == u16::MAX || size32 == u32::MAX || offset32 == u32::MAX {
            let locator_offset = end_offset.checked_sub(20).ok_or_else(|| FormatError::corrupt("missing ZIP64 locator"))?;
            let locator = read_at(&mut input, locator_offset, 20, length, control, accounting)?;
            if u32_at(&locator, 0)? != LOCATOR || u32_at(&locator, 4)? != 0 || u32_at(&locator, 16)? != 1 {
                return Err(FormatError::corrupt("invalid ZIP64 locator"));
            }
            let record_offset = u64_at(&locator, 8)?;
            let record = read_at(&mut input, record_offset, 56, length, control, accounting)?;
            let record_size = u64_at(&record, 4)?;
            if u32_at(&record, 0)? != END64
                || record_size < 44
                || record_size > limits.max_metadata_bytes
                || add(record_offset, add(record_size, 12)?)? != locator_offset
                || u32_at(&record, 16)? != 0
                || u32_at(&record, 20)? != 0
                || u64_at(&record, 24)? != u64_at(&record, 32)?
            {
                return Err(FormatError::corrupt("invalid ZIP64 end record"));
            }
            (u64_at(&record, 32)?, u64_at(&record, 40)?, u64_at(&record, 48)?, record_offset)
        } else {
            (u64::from(count32), u64::from(size32), u64::from(offset32), end_offset)
        };
        if count > limits.max_entries || directory_size > limits.max_metadata_bytes {
            return Err(FormatError::LimitExceeded("zip index metadata or count".into()));
        }
        if add(directory_offset, directory_size)? != terminator || count > directory_size / 46 {
            return Err(FormatError::corrupt("zip directory size or count"));
        }
        let entry_count = usize::try_from(count).map_err(|_| FormatError::LimitExceeded("zip entry count exceeds address space".into()))?;
        let index_size = count
            .checked_mul(256)
            .and_then(|size| size.checked_add(directory_size))
            .and_then(|size| size.checked_add(tail.len() as u64))
            .and_then(|size| size.checked_add(128 * 1024))
            .ok_or_else(|| FormatError::LimitExceeded("zip index size overflow".into()))?;
        if index_size > limits.max_metadata_bytes {
            return Err(FormatError::LimitExceeded("zip index metadata".into()));
        }
        let directory = read_at(&mut input, directory_offset, directory_size, length, control, accounting)?;
        let mut entries = OwnedVec::<Entry>::with_capacity(entry_count, accounting, control)?;
        let mut ranges = OwnedVec::<(u64, u64)>::with_capacity(entry_count, accounting, control)?;
        let mut names = BTreeSet::new();
        let mut cursor = 0usize;
        let mut metadata = index_size;
        for _ in 0..count {
            poll(control)?;
            let header_end = cursor.checked_add(46).ok_or_else(|| FormatError::corrupt("zip directory offset"))?;
            let header = directory.get(cursor..header_end).ok_or_else(|| FormatError::corrupt("truncated zip directory"))?;
            if u32_at(header, 0)? != CENTRAL {
                return Err(FormatError::corrupt("zip directory signature"));
            }
            let flags = u16_at(header, 8)?;
            let method = u16_at(header, 10)?;
            if flags & !0x0808 != 0 || !matches!(method, 0 | 8) {
                return Err(FormatError::Unsupported("zip flags or compression".into()));
            }
            if u16_at(header, 34)? != 0 {
                return Err(FormatError::Unsupported("multi-disk zip entry".into()));
            }
            let name_length = usize::from(u16_at(header, 28)?);
            let extra_length = usize::from(u16_at(header, 30)?);
            let comment_length = usize::from(u16_at(header, 32)?);
            let name_end = header_end.checked_add(name_length).ok_or_else(|| FormatError::corrupt("zip name overflow"))?;
            let extra_end = name_end.checked_add(extra_length).ok_or_else(|| FormatError::corrupt("zip extra overflow"))?;
            let next = extra_end
                .checked_add(comment_length)
                .filter(|end| *end <= directory.len())
                .ok_or_else(|| FormatError::corrupt("zip directory entry extent"))?;
            let name_bytes = directory.get(header_end..name_end).ok_or_else(|| FormatError::corrupt("truncated zip name"))?;
            accounted::charge(accounting, control, name_length as u64)?;
            let name = std::str::from_utf8(name_bytes).map_err(|_| FormatError::corrupt("zip name encoding"))?;
            if !valid_name(name) || (accounting.is_none() && !names.insert(name.to_owned())) {
                return Err(FormatError::corrupt("invalid or duplicate zip name"));
            }
            metadata = add(metadata, name_length as u64 * 2)?;
            if metadata > limits.max_metadata_bytes {
                return Err(FormatError::LimitExceeded("zip name metadata".into()));
            }
            let raw_uncompressed = u32_at(header, 24)?;
            let raw_compressed = u32_at(header, 20)?;
            let raw_offset = u32_at(header, 42)?;
            let ext = extended(
                directory.get(name_end..extra_end).ok_or_else(|| FormatError::corrupt("zip extra range"))?,
                [raw_uncompressed == u32::MAX, raw_compressed == u32::MAX, raw_offset == u32::MAX],
            )?;
            let mut values = ext.into_iter();
            let uncompressed = resolved(raw_uncompressed, values.next().flatten())?;
            let compressed = resolved(raw_compressed, values.next().flatten())?;
            let local_offset = resolved(raw_offset, values.next().flatten())?;
            if method == 0 && compressed != uncompressed {
                return Err(FormatError::corrupt("stored zip size mismatch"));
            }
            let local = read_at(&mut input, local_offset, 30, directory_offset, control, accounting)?;
            if u32_at(&local, 0)? != LOCAL || u16_at(&local, 6)? != flags || u16_at(&local, 8)? != method {
                return Err(FormatError::corrupt("zip local/central header mismatch"));
            }
            let local_name_length = u64::from(u16_at(&local, 26)?);
            let local_extra_length = u64::from(u16_at(&local, 28)?);
            let variable = read_at(&mut input, add(local_offset, 30)?, add(local_name_length, local_extra_length)?, directory_offset, control, accounting)?;
            let local_name_length = usize::try_from(local_name_length).map_err(|_| FormatError::corrupt("zip local name length"))?;
            if variable.get(..local_name_length) != Some(name_bytes) {
                return Err(FormatError::corrupt("zip local/central names differ"));
            }
            let offset = add(local_offset, add(30, variable.len() as u64)?)?;
            let payload_end = add(offset, compressed)?;
            if payload_end > directory_offset {
                return Err(FormatError::corrupt("zip entry overlaps directory"));
            }
            let crc = u32_at(header, 16)?;
            let mut range_end = payload_end;
            if flags & 8 == 0 {
                let local_compressed = u32_at(&local, 18)?;
                let local_uncompressed = u32_at(&local, 22)?;
                let ext = extended(
                    variable.get(local_name_length..).ok_or_else(|| FormatError::corrupt("zip local extra"))?,
                    [local_uncompressed == u32::MAX, local_compressed == u32::MAX, false],
                )?;
                let mut values = ext.into_iter();
                if u32_at(&local, 14)? != crc
                    || resolved(local_uncompressed, values.next().flatten())? != uncompressed
                    || resolved(local_compressed, values.next().flatten())? != compressed
                {
                    return Err(FormatError::corrupt("zip local/central size or CRC mismatch"));
                }
            } else {
                let zip64 = u32_at(&local, 18)? == u32::MAX || u32_at(&local, 22)? == u32::MAX;
                let first = read_at(&mut input, payload_end, 4, directory_offset, control, accounting)?;
                let descriptor_offset = if u32_at(&first, 0)? == 0x0807_4b50 { add(payload_end, 4)? } else { payload_end };
                let size = if zip64 { 20 } else { 12 };
                let descriptor = read_at(&mut input, descriptor_offset, size, directory_offset, control, accounting)?;
                let (c, u) = if zip64 {
                    (u64_at(&descriptor, 4)?, u64_at(&descriptor, 12)?)
                } else {
                    (u64::from(u32_at(&descriptor, 4)?), u64::from(u32_at(&descriptor, 8)?))
                };
                if u32_at(&descriptor, 0)? != crc || c != compressed || u != uncompressed {
                    return Err(FormatError::corrupt("zip data descriptor mismatch"));
                }
                range_end = add(descriptor_offset, size)?;
            }
            ranges.push((local_offset, range_end));
            let copied_name = accounted::name(name, accounting, control)?;
            entries.push(Entry { name: copied_name.value, _name_lease: copied_name.lease, offset, compressed, uncompressed, method, crc });
            cursor = next;
        }
        if cursor != directory.len() {
            return Err(FormatError::corrupt("zip directory has unindexed records"));
        }
        if accounting.is_some() {
            accounted::sort(&mut entries, accounting, control, |a, b| {
                accounted::charge(
                    accounting,
                    control,
                    (a.name.len() as u64).checked_add(b.name.len() as u64).ok_or_else(|| FormatError::LimitExceeded("ZIP comparison work".into()))?,
                )?;
                Ok(a.name.cmp(&b.name))
            })?;
            for pair in entries.windows(2) {
                accounted::charge(
                    accounting,
                    control,
                    pair.iter()
                        .try_fold(0_u64, |n, entry| n.checked_add(entry.name.len() as u64))
                        .ok_or_else(|| FormatError::LimitExceeded("ZIP duplicate work".into()))?,
                )?;
                if pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.name == b.name) {
                    return Err(FormatError::corrupt("invalid or duplicate zip name"));
                }
            }
            accounted::sort(&mut ranges, accounting, control, |a, b| Ok(a.cmp(b)))?;
        } else {
            ranges.sort_unstable();
        }
        let mut previous = 0;
        for (start, end) in ranges.iter().copied() {
            poll(control)?;
            if start < previous {
                return Err(FormatError::corrupt("overlapping zip entries"));
            }
            previous = end;
        }
        accounted::charge(accounting, control, 1)?;
        Ok(Self { input, entries, _owner_lease: owner_lease })
    }
    pub fn copy_by_name(&mut self, name: &str, output: &mut impl Write, max: u64, control: &Interrupt<'_>) -> Result<u64> {
        poll(control)?;
        let entry = self.entries.iter().find(|entry| entry.name == name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?;
        if entry.uncompressed > max {
            return Err(FormatError::LimitExceeded(format!("zip entry `{name}` is {} bytes (max {max})", entry.uncompressed)));
        }
        self.input.seek(SeekFrom::Start(entry.offset))?;
        let mut compressed = (&mut self.input).take(entry.compressed);
        let mut crc = Crc::new();
        let count = if entry.method == 0 {
            copy_checked(&mut compressed, output, entry.uncompressed, &mut crc, control)?
        } else {
            let mut decoder = flate2::Decompress::new(false);
            let mut input_buffer = [0; BLOCK];
            let mut output_buffer = [0; BLOCK];
            let mut input_start = 0usize;
            let mut input_end = 0usize;
            let mut count = 0u64;
            loop {
                poll(control)?;
                if input_start == input_end {
                    input_end = compressed.read(&mut input_buffer)?;
                    if input_end > input_buffer.len() {
                        return Err(FormatError::corrupt("zip reader returned invalid count"));
                    }
                    input_start = 0;
                }
                let input = input_buffer.get(input_start..input_end).ok_or_else(|| FormatError::corrupt("zip DEFLATE input range"))?;
                let remaining = entry.uncompressed.checked_sub(count).ok_or_else(|| FormatError::corrupt("zip expanded bytes exceed declaration"))?;
                // A one-byte probe also requires a valid stream terminator when
                // the declared payload has already been produced.
                let capacity = remaining.min(BLOCK as u64).max(1) as usize;
                let buffer = output_buffer.get_mut(..capacity).ok_or_else(|| FormatError::corrupt("zip DEFLATE output range"))?;
                let before_in = decoder.total_in();
                let before_out = decoder.total_out();
                let status = decoder.decompress(input, buffer, flate2::FlushDecompress::None).map_err(|_| FormatError::corrupt("zip DEFLATE stream"))?;
                let consumed = decoder.total_in().checked_sub(before_in).ok_or_else(|| FormatError::corrupt("zip DEFLATE input count"))?;
                let produced = decoder.total_out().checked_sub(before_out).ok_or_else(|| FormatError::corrupt("zip DEFLATE output count"))?;
                let consumed = usize::try_from(consumed).map_err(|_| FormatError::corrupt("zip DEFLATE input count"))?;
                let produced_size = usize::try_from(produced).map_err(|_| FormatError::corrupt("zip DEFLATE output count"))?;
                if consumed > input.len() || produced > remaining {
                    return Err(FormatError::corrupt("zip DEFLATE payload count"));
                }
                input_start = input_start.checked_add(consumed).ok_or_else(|| FormatError::corrupt("zip DEFLATE input count"))?;
                count = add(count, produced)?;
                let block = buffer.get(..produced_size).ok_or_else(|| FormatError::corrupt("zip DEFLATE output count"))?;
                poll(control)?;
                crc.update(block)?;
                output.write_all(block)?;
                if status == flate2::Status::StreamEnd {
                    if decoder.total_in() != entry.compressed || input_start != input_end || compressed.limit() != 0 {
                        return Err(FormatError::corrupt("zip DEFLATE stream length"));
                    }
                    break;
                }
                if consumed == 0 && produced == 0 {
                    return Err(FormatError::corrupt("zip DEFLATE truncated or stalled stream"));
                }
            }
            count
        };
        if count != entry.uncompressed || compressed.limit() != 0 || crc.finish() != entry.crc {
            return Err(FormatError::corrupt("zip payload size or CRC"));
        }
        poll(control)?;
        Ok(count)
    }
    pub fn read_by_name(&mut self, name: &str, max: usize, control: &Interrupt<'_>) -> Result<Vec<u8>> {
        poll(control)?;
        let size = self.entries.iter().find(|entry| entry.name == name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?.uncompressed;
        if size > max as u64 {
            return Err(FormatError::LimitExceeded("zip expanded entry bytes".into()));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(usize::try_from(size).map_err(|_| FormatError::LimitExceeded("zip expanded entry address space".into()))?)
            .map_err(|_| FormatError::LimitExceeded("zip expanded entry allocation".into()))?;
        self.copy_by_name(name, &mut bytes, max as u64, control)?;
        Ok(bytes)
    }
}
fn copy_checked(input: &mut impl Read, output: &mut impl Write, expected: u64, crc: &mut Crc, control: &Interrupt<'_>) -> Result<u64> {
    let mut buffer = [0; BLOCK];
    let mut count = 0u64;
    loop {
        poll(control)?;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count = add(count, read as u64)?;
        if count > expected {
            return Err(FormatError::corrupt("zip expanded bytes exceed declaration"));
        }
        let block = buffer.get(..read).ok_or_else(|| FormatError::corrupt("zip reader returned invalid count"))?;
        crc.update(block)?;
        output.write_all(block)?;
    }
    Ok(count)
}
