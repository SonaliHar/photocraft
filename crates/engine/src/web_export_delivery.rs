//! Explicit byte delivery for Save for Web; native filesystem commands do not use it.
use crate::{EngineError, Result};
#[cfg(test)]
use photocraft_format::zip::stream::{Limits as ZipLimits, Writer};
#[cfg(test)]
use std::io::{Cursor, Seek, SeekFrom, Write};

pub type Deliver = fn(&str, &[u8]) -> std::result::Result<(), String>;
#[derive(Clone, Copy)]
pub(crate) struct Delivery {
    callback: Deliver,
    #[cfg(test)]
    limits: Limits,
}
#[cfg(test)]
#[derive(Clone, Copy)]
struct Limits {
    output: usize,
    metadata: u64,
    entries: u64,
    name: usize,
}
#[cfg(not(test))]
#[derive(Clone, Copy)]
struct Limits;
impl Default for Limits {
    fn default() -> Self {
        #[cfg(test)]
        {
            Self { output: 64 << 20, metadata: 1 << 20, entries: 4096, name: 1024 }
        }
        #[cfg(not(test))]
        {
            Self
        }
    }
}
impl Limits {
    fn output(self) -> usize {
        #[cfg(test)]
        {
            self.output
        }
        #[cfg(not(test))]
        {
            64 << 20
        }
    }
    fn metadata(self) -> u64 {
        #[cfg(test)]
        {
            self.metadata
        }
        #[cfg(not(test))]
        {
            1 << 20
        }
    }
    fn entries(self) -> u64 {
        #[cfg(test)]
        {
            self.entries
        }
        #[cfg(not(test))]
        {
            4096
        }
    }
    fn name(self) -> usize {
        #[cfg(test)]
        {
            self.name
        }
        #[cfg(not(test))]
        {
            1024
        }
    }
}

fn fail(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Save for Web delivery: {message}"))
}
fn relative_name(name: &str, cap: usize) -> Result<()> {
    if name.is_empty()
        || name.len() > cap
        || name.contains(['\0', '\\', ':'])
        || name.starts_with('/')
        || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(fail("invalid or oversized relative output name"));
    }
    Ok(())
}
impl Delivery {
    fn limits(self) -> Limits {
        #[cfg(test)]
        {
            self.limits
        }
        #[cfg(not(test))]
        {
            Limits
        }
    }
    pub(crate) fn new(callback: Deliver) -> Self {
        Self {
            callback,
            #[cfg(test)]
            limits: Limits::default(),
        }
    }
    pub(crate) fn send(self, name: &str, bytes: &[u8]) -> Result<()> {
        relative_name(name, self.limits().name())?;
        if bytes.len() > self.limits().output() {
            return Err(fail("output exceeds 64 MiB delivery budget"));
        }
        (self.callback)(name, bytes).map_err(fail)
    }
    pub(crate) fn check_name(self, name: &str) -> Result<()> {
        relative_name(name, self.limits().name())
    }
    pub(crate) fn archive(self) -> Result<Archive> {
        Ok(Archive { delivery: self, zip: Zip64Bytes::new(self.limits()), failed: false })
    }

    pub(crate) fn check_plan(self, entries: usize) -> Result<()> {
        if entries as u64 > self.limits().entries() {
            return Err(fail("slice count exceeds 4096 entry budget"));
        }
        Ok(())
    }
    pub(crate) fn check_source_metadata(self, bytes: usize) -> Result<()> {
        if bytes as u64 > self.limits().metadata() {
            return Err(fail("HTML metadata exceeds 1 MiB budget"));
        }
        Ok(())
    }
}
pub(crate) struct Archive {
    delivery: Delivery,
    failed: bool,
    zip: Zip64Bytes,
}
impl Archive {
    pub(crate) fn add(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        if self.failed {
            return Err(fail("archive previously failed"));
        }
        let result = relative_name(name, self.delivery.limits().name()).and_then(|()| self.zip.add(name, bytes));
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub(crate) fn finish(self, name: &str) -> Result<usize> {
        if self.failed {
            return Err(fail("archive previously failed"));
        }
        let bytes = self.zip.finish()?;
        self.delivery.send(name, &bytes)?;
        Ok(bytes.len())
    }
}
#[cfg(test)]
struct BoundedOutput {
    data: Vec<u8>,
    position: usize,
    cap: usize,
}
#[cfg(test)]
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let error = || std::io::Error::other("Save for Web bounded output allocation/length");
        let end = self.position.checked_add(bytes.len()).filter(|end| *end <= self.cap).ok_or_else(error)?;
        if end > self.data.len() {
            self.data.try_reserve_exact(end - self.data.len()).map_err(|_| error())?;
            self.data.resize(end, 0);
        }
        self.data.get_mut(self.position..end).ok_or_else(error)?.copy_from_slice(bytes);
        self.position = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
impl Seek for BoundedOutput {
    fn seek(&mut self, offset: SeekFrom) -> std::io::Result<u64> {
        let position = match offset {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::End(value) => self.data.len() as i128 + i128::from(value),
            SeekFrom::Current(value) => self.position as i128 + i128::from(value),
        };
        let position =
            usize::try_from(position).ok().filter(|value| *value <= self.cap).ok_or_else(|| std::io::Error::other("Save for Web bounded output seek"))?;
        self.position = position;
        Ok(position as u64)
    }
}
#[cfg(test)]
#[path = "web_export_delivery_tests.rs"]
mod tests;

/// Browser-only bound on intermediate edge bands, horizontal runs and covered/merge scans.
/// Conservative bounds are checked before calling the unchanged document slice algorithms.
pub(crate) struct PlanBudget {
    work_left: usize,
    runs_left: usize,
}
impl Default for PlanBudget {
    fn default() -> Self {
        Self { work_left: 1 << 20, runs_left: 4096 }
    }
}
impl PlanBudget {
    pub(crate) fn selected_upper(resolved: usize, stored: usize, p: &serde_json::Value) -> usize {
        // The established planner gives explicit numbers precedence over slices=user.
        if let Some(numbers) = p.get("numbers").and_then(serde_json::Value::as_array) {
            resolved.min(numbers.len())
        } else if p.get("slices").and_then(serde_json::Value::as_str) == Some("user") {
            stored
        } else {
            resolved
        }
    }
    pub(crate) fn charge_work(&mut self, work: usize) -> Result<()> {
        if work > self.work_left {
            return Err(fail("slice planner work budget exceeded"));
        }
        self.work_left -= work;
        Ok(())
    }
    pub(crate) fn charge_stacking_scans(&mut self, selected: usize, stored: usize) -> Result<()> {
        // All selected slices evaluate every selected z(o) and z(r), each a
        // stored-list scan. Also account for each selected slice's later get().
        // Charge all passes together before cloning the document or staging above.
        let comparisons = selected.checked_mul(2).and_then(|v| v.checked_add(1)).ok_or_else(|| fail("slice stacking work overflow"))?;
        let work = selected.checked_mul(stored).and_then(|v| v.checked_mul(comparisons)).ok_or_else(|| fail("slice stacking work overflow"))?;
        self.charge_work(work)
    }
    pub(crate) fn before_grid(
        &mut self,
        canvas: photocraft_geom::Rect,
        taken: &mut dyn Iterator<Item = photocraft_geom::Rect>,
        upper_count: usize,
    ) -> Result<usize> {
        if upper_count > 4096 {
            return Err(fail("slice count exceeds 4096 entry budget"));
        }
        if canvas.is_empty() {
            return Ok(0);
        }
        let capacity = upper_count.checked_mul(2).and_then(|v| v.checked_add(2)).ok_or_else(|| fail("slice edge capacity overflow"))?;
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        xs.try_reserve_exact(capacity).map_err(|_| fail("slice X edge allocation"))?;
        ys.try_reserve_exact(capacity).map_err(|_| fail("slice Y edge allocation"))?;
        xs.extend([canvas.x0, canvas.x1]);
        ys.extend([canvas.y0, canvas.y1]);
        let mut count = 0usize;
        for rect in taken {
            count = count.checked_add(1).ok_or_else(|| fail("slice count overflow"))?;
            if count > upper_count {
                return Err(fail("slice edge count changed"));
            }
            let rect = rect.intersect(&canvas);
            if !rect.is_empty() {
                xs.extend([rect.x0, rect.x1]);
                ys.extend([rect.y0, rect.y1]);
            }
        }
        xs.sort_unstable();
        xs.dedup();
        ys.sort_unstable();
        ys.dedup();
        let bands = xs.len().saturating_sub(1).checked_mul(ys.len().saturating_sub(1)).ok_or_else(|| fail("slice grid overflow"))?;
        // One run per cell bounds the intermediate run Vec. Worst-case covered scans
        // visit all taken rectangles; vertical merging may compare all run pairs.
        let work = bands
            .checked_mul(count)
            .and_then(|v| bands.checked_mul(bands).and_then(|merge| v.checked_add(merge)))
            .ok_or_else(|| fail("slice grid work overflow"))?;
        if bands > self.runs_left || work > self.work_left {
            return Err(fail("slice grid work/run budget exceeded"));
        }
        self.runs_left -= bands;
        self.work_left -= work;
        Ok(bands)
    }
}
impl Delivery {
    pub(crate) fn preflight_source(self, doc: &photocraft_doc::Document, p: &serde_json::Value) -> Result<()> {
        self.check_plan(doc.slices.list.len())?;
        if doc.name.len() > 900 || doc.slices.group_name.len() > self.limits().name() {
            return Err(fail("source label exceeds filename budget"));
        }
        let mut metadata = 0usize;
        for slice in &doc.slices.list {
            if slice.name.len() > 900 {
                return Err(fail("slice name exceeds filename budget"));
            }
            for text in [&slice.url, &slice.target, &slice.message, &slice.alt, &slice.cell_text] {
                metadata = metadata
                    .checked_add(text.len().checked_mul(6).ok_or_else(|| fail("source HTML metadata overflow"))?)
                    .ok_or_else(|| fail("source HTML metadata overflow"))?;
                self.check_source_metadata(metadata)?;
            }
        }
        if let Some(numbers) = p.get("numbers").and_then(serde_json::Value::as_array) {
            self.check_plan(numbers.len())?;
            if numbers.iter().any(|v| v.as_u64().is_none()) {
                return Err(fail("slice numbers must be unsigned integers"));
            }
        }
        if let Some(name) = p.get("path").and_then(serde_json::Value::as_str).filter(|v| !v.is_empty()) {
            self.check_name(name)?;
        }
        if let Some(folder) = p.get("imagesFolder").and_then(serde_json::Value::as_str).filter(|v| !v.is_empty()) {
            self.check_name(folder)?;
        }
        if p.get("dir").and_then(serde_json::Value::as_str).is_some_and(|v| v.len() > self.limits().name()) {
            return Err(fail("folder hint exceeds filename budget"));
        }
        Ok(())
    }
}

/// Nongeneric bounded ZIP64 STORE writer for already-owned encoded entry bytes.
/// Headers, conservative metadata/output reservations and timestamps match the stream writer.
struct Zip64Bytes {
    output: Vec<u8>,
    central: Vec<u8>,
    names: Vec<String>,
    metadata: u64,
    #[cfg(test)]
    limits: Limits,
}
fn zip_limit(message: &str) -> EngineError {
    fail(photocraft_format::FormatError::LimitExceeded(message.into()))
}
fn put(dst: &mut [u8], offset: usize, bytes: &[u8]) -> Result<()> {
    let end = offset.checked_add(bytes.len()).ok_or_else(|| fail("ZIP header offset overflow"))?;
    dst.get_mut(offset..end).ok_or_else(|| fail("ZIP fixed header layout"))?.copy_from_slice(bytes);
    Ok(())
}
impl Zip64Bytes {
    fn limits(&self) -> Limits {
        #[cfg(test)]
        {
            self.limits
        }
        #[cfg(not(test))]
        {
            Limits
        }
    }
    fn new(#[cfg(test)] limits: Limits, #[cfg(not(test))] _limits: Limits) -> Self {
        Self {
            output: Vec::new(),
            central: Vec::new(),
            names: Vec::new(),
            metadata: 0,
            #[cfg(test)]
            limits,
        }
    }
    fn add(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let name_length = u16::try_from(name.len()).map_err(|_| zip_limit("zip name too long"))?;
        // Each duplicate scan is bounded by4096 entries and1MiB aggregate index/name metadata.
        if self.names.iter().any(|old| old == name) {
            return Err(fail(photocraft_format::FormatError::Corrupt("invalid or duplicate zip name".into())));
        }
        let count = u64::try_from(self.names.len()).map_err(|_| zip_limit("zip entry count"))?;
        if count >= self.limits().entries() {
            return Err(zip_limit("zip entry count"));
        }
        let name_bytes = u64::try_from(name.len()).map_err(|_| zip_limit("zip name length"))?;
        let metadata = self
            .metadata
            .checked_add(name_bytes.checked_mul(2).and_then(|v| v.checked_add(160)).ok_or_else(|| zip_limit("zip index metadata"))?)
            .ok_or_else(|| zip_limit("zip index metadata"))?;
        if metadata > self.limits().metadata() {
            return Err(zip_limit("zip index metadata"));
        }
        let offset = u64::try_from(self.output.len()).map_err(|_| zip_limit("zip output bytes"))?;
        let size = u64::try_from(bytes.len()).map_err(|_| zip_limit("zip entry size"))?;
        let entry_bytes = 50usize.checked_add(name.len()).and_then(|v| v.checked_add(bytes.len())).ok_or_else(|| zip_limit("zip output bytes"))?;
        let end = offset.checked_add(u64::try_from(entry_bytes).map_err(|_| zip_limit("zip output bytes"))?).ok_or_else(|| zip_limit("zip output bytes"))?;
        let reserved = end.checked_add(metadata).and_then(|v| v.checked_add(98)).ok_or_else(|| zip_limit("zip output bytes"))?;
        if reserved > u64::try_from(self.limits().output()).map_err(|_| zip_limit("zip output bytes"))? {
            return Err(zip_limit("zip output bytes"));
        }
        let central_bytes = 74usize.checked_add(name.len()).ok_or_else(|| zip_limit("zip index allocation"))?;
        // Reserve every growing owner before any logical entry publication.
        self.output.try_reserve_exact(entry_bytes).map_err(|_| zip_limit("zip output allocation"))?;
        self.central.try_reserve_exact(central_bytes).map_err(|_| zip_limit("zip index allocation"))?;
        self.names.try_reserve_exact(1).map_err(|_| zip_limit("zip names allocation"))?;
        let mut owned_name = String::new();
        owned_name.try_reserve_exact(name.len()).map_err(|_| zip_limit("zip name allocation"))?;
        owned_name.push_str(name);
        let crc = photocraft_format::zip::crc32(bytes);
        let mut local = [0u8; 30];
        put(&mut local, 0, &0x04034b50u32.to_le_bytes())?;
        put(&mut local, 4, &45u16.to_le_bytes())?;
        put(&mut local, 6, &0x0800u16.to_le_bytes())?;
        put(&mut local, 12, &0x21u16.to_le_bytes())?;
        put(&mut local, 14, &crc.to_le_bytes())?;
        put(&mut local, 18, &u32::MAX.to_le_bytes())?;
        put(&mut local, 22, &u32::MAX.to_le_bytes())?;
        put(&mut local, 26, &name_length.to_le_bytes())?;
        put(&mut local, 28, &20u16.to_le_bytes())?;
        let mut extra = [0u8; 20];
        put(&mut extra, 0, &1u16.to_le_bytes())?;
        put(&mut extra, 2, &16u16.to_le_bytes())?;
        put(&mut extra, 4, &size.to_le_bytes())?;
        put(&mut extra, 12, &size.to_le_bytes())?;
        let mut central = [0u8; 46];
        put(&mut central, 0, &0x02014b50u32.to_le_bytes())?;
        put(&mut central, 4, &45u16.to_le_bytes())?;
        put(&mut central, 6, &45u16.to_le_bytes())?;
        put(&mut central, 8, &0x0800u16.to_le_bytes())?;
        put(&mut central, 14, &0x21u16.to_le_bytes())?;
        put(&mut central, 16, &crc.to_le_bytes())?;
        put(&mut central, 20, &u32::MAX.to_le_bytes())?;
        put(&mut central, 24, &u32::MAX.to_le_bytes())?;
        put(&mut central, 28, &name_length.to_le_bytes())?;
        put(&mut central, 30, &28u16.to_le_bytes())?;
        put(&mut central, 42, &u32::MAX.to_le_bytes())?;
        let mut central_extra = [0u8; 28];
        put(&mut central_extra, 0, &1u16.to_le_bytes())?;
        put(&mut central_extra, 2, &24u16.to_le_bytes())?;
        put(&mut central_extra, 4, &size.to_le_bytes())?;
        put(&mut central_extra, 12, &size.to_le_bytes())?;
        put(&mut central_extra, 20, &offset.to_le_bytes())?;
        self.output.extend_from_slice(&local);
        self.output.extend_from_slice(name.as_bytes());
        self.output.extend_from_slice(&extra);
        self.output.extend_from_slice(bytes);
        self.central.extend_from_slice(&central);
        self.central.extend_from_slice(name.as_bytes());
        self.central.extend_from_slice(&central_extra);
        self.names.push(owned_name);
        self.metadata = metadata;
        Ok(())
    }
    fn finish(mut self) -> Result<Vec<u8>> {
        let offset = u64::try_from(self.output.len()).map_err(|_| zip_limit("zip output bytes"))?;
        let size = u64::try_from(self.central.len()).map_err(|_| zip_limit("zip index bytes"))?;
        let end64_offset = offset.checked_add(size).ok_or_else(|| zip_limit("zip output bytes"))?;
        let total = end64_offset.checked_add(98).ok_or_else(|| zip_limit("zip output bytes"))?;
        if total > u64::try_from(self.limits().output()).map_err(|_| zip_limit("zip output bytes"))? {
            return Err(zip_limit("zip output bytes"));
        }
        let count = u64::try_from(self.names.len()).map_err(|_| zip_limit("zip entry count"))?;
        let growth = self.central.len().checked_add(98).ok_or_else(|| zip_limit("zip output bytes"))?;
        self.output.try_reserve_exact(growth).map_err(|_| zip_limit("zip output allocation"))?;
        let mut end = [0u8; 98];
        put(&mut end, 0, &0x06064b50u32.to_le_bytes())?;
        put(&mut end, 4, &44u64.to_le_bytes())?;
        put(&mut end, 12, &45u16.to_le_bytes())?;
        put(&mut end, 14, &45u16.to_le_bytes())?;
        put(&mut end, 24, &count.to_le_bytes())?;
        put(&mut end, 32, &count.to_le_bytes())?;
        put(&mut end, 40, &size.to_le_bytes())?;
        put(&mut end, 48, &offset.to_le_bytes())?;
        put(&mut end, 56, &0x07064b50u32.to_le_bytes())?;
        put(&mut end, 64, &end64_offset.to_le_bytes())?;
        put(&mut end, 72, &1u32.to_le_bytes())?;
        put(&mut end, 76, &0x06054b50u32.to_le_bytes())?;
        put(&mut end, 84, &u16::MAX.to_le_bytes())?;
        put(&mut end, 86, &u16::MAX.to_le_bytes())?;
        put(&mut end, 88, &u32::MAX.to_le_bytes())?;
        put(&mut end, 92, &u32::MAX.to_le_bytes())?;
        self.output.extend_from_slice(&self.central);
        self.output.extend_from_slice(&end);
        Ok(self.output)
    }
}
