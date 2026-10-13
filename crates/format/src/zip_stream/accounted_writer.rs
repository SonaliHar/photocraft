//! Optional writer metadata ownership. Logical capacity, not RSS/allocator bytes.
use super::{
    accounted::{self, OwnedName, OwnedVec},
    *,
};
use photocraft_raster::backing::{
    ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::cmp::Ordering;
pub(super) struct State {
    central: OwnedVec<u8>,
    names: OwnedVec<OwnedName>,
    quota: BackingQuota,
    metadata: u64,
    _owner: QuotaLease,
}
impl State {
    pub(super) fn shares_quota(&self, quota: &BackingQuota) -> bool {
        self.quota.shares_ledger(quota)
    }
    pub(super) fn new(quota: &BackingQuota, control: &ReadControl<'_>, owner_bytes: u64) -> Result<Self> {
        charge(control, 1)?;
        let owner = quota.reserve(QuotaKind::Resident, owner_bytes).map_err(|_| FormatError::LimitExceeded("ZIP writer owner quota".into()))?;
        let account = Some((control, quota));
        let central = OwnedVec::with_capacity(0, account, control.interrupt)?;
        let names = OwnedVec::with_capacity(0, account, control.interrupt)?;
        charge(control, 0)?;
        Ok(Self { central, names, quota: quota.clone(), metadata: 0, _owner: owner })
    }
    pub(super) fn add(
        &mut self,
        output: &mut (impl Write + Seek),
        name: &str,
        size: u64,
        input: &mut impl Read,
        limits: Limits,
        control: &ReadControl<'_>,
    ) -> Result<()> {
        charge(control, 1)?;
        let name_length = u16::try_from(name.len()).map_err(|_| FormatError::LimitExceeded("zip name too long".into()))?;
        if !valid(name, control)? {
            return Err(FormatError::corrupt("invalid zip name"));
        }
        let position = position(&self.names, name, control)?;
        if self.names.len() as u64 >= limits.max_entries {
            return Err(FormatError::LimitExceeded("zip entry count".into()));
        }
        let metadata = super::add(self.metadata, super::add(name.len() as u64 * 2, 160)?)?;
        if metadata > limits.max_metadata_bytes {
            return Err(FormatError::LimitExceeded("zip index metadata".into()));
        }
        let account = Some((control, &self.quota));
        let required =
            self.central.len().checked_add(74).and_then(|n| n.checked_add(name.len())).ok_or_else(|| FormatError::corrupt("zip central capacity overflow"))?;
        if required > self.central.capacity() {
            let ceiling = usize::try_from(limits.max_metadata_bytes).unwrap_or(usize::MAX);
            let capacity = required.max(self.central.capacity().checked_mul(2).unwrap_or(required).min(ceiling));
            let mut grown = OwnedVec::with_capacity(capacity, account, control.interrupt)?;
            for block in self.central.chunks(BLOCK) {
                charge(control, block.len() as u64)?;
                grown.extend_from_slice(block)
            }
            self.central = grown;
        }
        let count = self.names.len().checked_add(1).ok_or_else(|| FormatError::corrupt("zip name count overflow"))?;
        if count > self.names.capacity() {
            let ceiling = usize::try_from(limits.max_entries).unwrap_or(usize::MAX);
            let capacity = count.max(self.names.capacity().checked_mul(2).unwrap_or(count).min(ceiling));
            let mut grown = OwnedVec::with_capacity(capacity, account, control.interrupt)?;
            for value in self.names.drain(..) {
                charge(control, std::mem::size_of::<OwnedName>() as u64)?;
                grown.push(value)
            }
            self.names = grown;
        }
        let owned_name = accounted::name(name, account, control.interrupt)?;
        let mut header = OwnedVec::with_capacity(50 + name.len(), account, control.interrupt)?;
        let _copy_lease = self
            .quota
            .reserve(QuotaKind::Resident, (BLOCK + std::mem::size_of::<Crc>()) as u64)
            .map_err(|_| FormatError::LimitExceeded("ZIP writer copy quota".into()))?;
        charge(control, BLOCK as u64)?;
        let mut buffer = [0; BLOCK];
        charge(control, 1)?;
        let offset = output.stream_position()?;
        let payload = super::add(offset, super::add(50, name.len() as u64)?)?;
        let end = super::add(payload, size)?;
        if super::add(end, super::add(metadata, 98)?)? > limits.max_archive_bytes {
            return Err(FormatError::LimitExceeded("zip output bytes".into()));
        }
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
        append_name(&mut header, name, control)?;
        p16(&mut header, 1);
        p16(&mut header, 16);
        p64(&mut header, size);
        p64(&mut header, size);
        charge(control, header.len() as u64)?;
        write_all(output, &header, control)?;
        let mut remaining = size;
        let mut crc = Crc::new();
        while remaining != 0 {
            let count = remaining.min(BLOCK as u64) as usize;
            charge(control, count as u64)?;
            let block = buffer.get_mut(..count).ok_or_else(|| FormatError::corrupt("zip copy range"))?;
            read_exact(input, block, control)?;
            charge(control, count as u64)?;
            crc.update(block)?;
            write_all(output, block, control)?;
            remaining = remaining.checked_sub(count as u64).ok_or_else(|| FormatError::corrupt("zip copy count"))?;
        }
        charge(control, 1)?;
        let mut extra = [0];
        if read_some(input, &mut extra, control)? != 0 {
            return Err(FormatError::corrupt("zip input exceeds declared size"));
        }
        let crc = crc.finish();
        charge(control, 1)?;
        output.seek(SeekFrom::Start(super::add(offset, 14)?))?;
        write_all(output, &crc.to_le_bytes(), control)?;
        output.seek(SeekFrom::Start(end))?;
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
        append_name(central, name, control)?;
        p16(central, 1);
        p16(central, 24);
        p64(central, size);
        p64(central, size);
        p64(central, offset);
        self.names.push(owned_name);
        let mut at = self.names.len().checked_sub(1).ok_or_else(|| FormatError::corrupt("zip insert index"))?;
        while at > position {
            charge(control, std::mem::size_of::<OwnedName>() as u64)?;
            let previous = at.checked_sub(1).ok_or_else(|| FormatError::corrupt("zip insert index"))?;
            self.names.swap(previous, at);
            at = previous;
        }
        self.metadata = metadata;
        charge(control, 0)
    }
    pub(super) fn finish(&self, output: &mut (impl Write + Seek), limits: Limits, control: &ReadControl<'_>) -> Result<()> {
        charge(control, 1)?;
        let mut end = OwnedVec::with_capacity(98, Some((control, &self.quota)), control.interrupt)?;
        let offset = output.stream_position()?;
        let size = self.central.len() as u64;
        let end64_offset = super::add(offset, size)?;
        if super::add(end64_offset, 98)? > limits.max_archive_bytes {
            return Err(FormatError::LimitExceeded("zip output bytes".into()));
        }
        for block in self.central.chunks(BLOCK) {
            charge(control, block.len() as u64)?;
            write_all(output, block, control)?;
        }
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
        charge(control, end.len() as u64)?;
        write_all(output, &end, control)?;
        output.flush()?;
        charge(control, 0)
    }
}
fn charge(control: &ReadControl<'_>, units: u64) -> Result<()> {
    accounted::charge(None, control.interrupt, 0)?;
    control.check(units).map_err(|e| match e {
        photocraft_raster::backing::BackingError::Cancelled => FormatError::Cancelled,
        other => FormatError::LimitExceeded(format!("ZIP writer control: {other}")),
    })
}
fn append_name(bytes: &mut Vec<u8>, name: &str, control: &ReadControl<'_>) -> Result<()> {
    for block in name.as_bytes().chunks(4096) {
        charge(control, block.len() as u64)?;
        bytes.extend_from_slice(block)
    }
    Ok(())
}
fn valid(name: &str, control: &ReadControl<'_>) -> Result<bool> {
    if name.is_empty() || name.starts_with('/') {
        return Ok(false);
    }
    for block in name.as_bytes().chunks(4096) {
        charge(control, block.len() as u64)?;
        if block.iter().any(|b| matches!(b, 0 | b'\\' | b':')) {
            return Ok(false);
        }
    }
    let mut parts = name.split('/').peekable();
    let mut index = 0usize;
    while let Some(part) = parts.next() {
        charge(control, part.len() as u64 + 1)?;
        if part == ".." || part == "." || (part.is_empty() && (index == 0 || parts.peek().is_some())) {
            return Ok(false);
        }
        index = index.checked_add(1).ok_or_else(|| FormatError::corrupt("zip name components"))?;
    }
    Ok(true)
}
fn compare(a: &str, b: &str, control: &ReadControl<'_>) -> Result<Ordering> {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut offset = 0usize;
    let count = a.len().min(b.len());
    while offset < count {
        let end = offset.checked_add(4096).ok_or_else(|| FormatError::corrupt("zip name comparison"))?.min(count);
        charge(control, (end - offset) as u64 * 2)?;
        let order = a
            .get(offset..end)
            .ok_or_else(|| FormatError::corrupt("zip name comparison"))?
            .cmp(b.get(offset..end).ok_or_else(|| FormatError::corrupt("zip name comparison"))?);
        if !order.is_eq() {
            return Ok(order);
        }
        offset = end;
    }
    Ok(a.len().cmp(&b.len()))
}
fn position(names: &[OwnedName], name: &str, control: &ReadControl<'_>) -> Result<usize> {
    let mut low = 0usize;
    let mut high = names.len();
    while low < high {
        charge(control, 1)?;
        let mid = low + (high - low) / 2;
        match compare(&names.get(mid).ok_or_else(|| FormatError::corrupt("zip name index"))?.value, name, control)? {
            Ordering::Equal => return Err(FormatError::corrupt("duplicate zip name")),
            Ordering::Less => low = mid.checked_add(1).ok_or_else(|| FormatError::corrupt("zip name index"))?,
            Ordering::Greater => high = mid,
        }
    }
    Ok(low)
}

// Poll every fragmented read/write; finite Interrupted retry prevents adversarial
// no-progress sources from spinning indefinitely even with an unlimited controller.
fn read_some(input: &mut impl Read, out: &mut [u8], control: &ReadControl<'_>) -> Result<usize> {
    for _ in 0..32 {
        charge(control, 1)?;
        match input.read(out) {
            Ok(n) => {
                if n > out.len() {
                    return Err(FormatError::corrupt("ZIP reader returned oversized count"));
                }
                charge(control, n as u64)?;
                return Ok(n);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "ZIP input interrupted without progress").into())
}
fn read_exact(input: &mut impl Read, mut out: &mut [u8], control: &ReadControl<'_>) -> Result<()> {
    while !out.is_empty() {
        let n = read_some(input, out, control)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "ZIP input shorter than declared size").into());
        }
        out = out.get_mut(n..).ok_or_else(|| FormatError::corrupt("ZIP read count"))?;
    }
    Ok(())
}
fn write_all(output: &mut impl Write, mut bytes: &[u8], control: &ReadControl<'_>) -> Result<()> {
    let mut retries = 0u8;
    while !bytes.is_empty() {
        charge(control, 1)?;
        let block = bytes.get(..bytes.len().min(BLOCK)).ok_or_else(|| FormatError::corrupt("ZIP write block"))?;
        match output.write(block) {
            Ok(n) => {
                if n == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "ZIP output made no progress").into());
                }
                if n > block.len() {
                    return Err(FormatError::corrupt("ZIP writer returned oversized count"));
                }
                charge(control, n as u64)?;
                bytes = bytes.get(n..).ok_or_else(|| FormatError::corrupt("ZIP write count"))?;
                retries = 0
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                retries = retries.checked_add(1).ok_or_else(|| FormatError::corrupt("ZIP write retry count"))?;
                if retries >= 32 {
                    return Err(e.into());
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
