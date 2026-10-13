//! Optional staged verification of exact canonical raw-block Zstandard tile objects.
//! Ordinary compressed objects remain supported by the existing native loader.
//! Caller staging must stay unpublished until success and be discarded on any error.
use crate::{FormatError, zip::stream::Reader};
use photocraft_color::PixelFormat;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};
use std::io::{Read, Seek, Write};

#[derive(Debug)]
pub enum VerificationError {
    Backing(BackingError),
    Container(FormatError),
    Integrity,
    Unsupported,
}
impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "canonical staged verification: {self:?}")
    }
}
impl std::error::Error for VerificationError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedCanonicalTile {
    format: PixelFormat,
    length: u64,
    hash: [u8; 32],
}
impl VerifiedCanonicalTile {
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
}
/// Only exact current canonical RawFrame encoding is accepted. Callback storage,
/// archive input/index and ZIP metadata remain caller-owned and explicitly charged
/// elsewhere. The callback must not publish; late CRC/hash failures require discard.
pub fn verify_tile<R: Read + Seek>(
    archive: &mut Reader<R>,
    format: PixelFormat,
    hash: [u8; 32],
    quota: &BackingQuota,
    control: &ReadControl<'_>,
    mut stage: impl FnMut(u64, &[u8]) -> Result<(), BackingError>,
) -> Result<VerifiedCanonicalTile, VerificationError> {
    control.check(1).map_err(VerificationError::Backing)?;
    let bpp = format.bytes_per_pixel();
    if bpp == 0 || bpp > 40 {
        return Err(VerificationError::Unsupported);
    }
    let length = 256_u64.checked_mul(256).and_then(|n| n.checked_mul(bpp as u64)).ok_or(VerificationError::Integrity)?;
    let encoded = crate::zstd_stream::encoded_len(length).map_err(VerificationError::Container)?;
    // The existing ZIP copier uses a64KiB inline buffer; reserve that plus all
    // verifier/Hasher state before constructing it. No decoded payload allocation.
    let _lease = quota
        .reserve(QuotaKind::Resident, (std::mem::size_of::<Verifier<'_, '_>>() as u64).checked_add(65536 + 256).ok_or(VerificationError::Integrity)?)
        .map_err(|_| VerificationError::Backing(BackingError::Quota))?;
    let mut path = [0; 74];
    path.get_mut(..6).ok_or(VerificationError::Integrity)?.copy_from_slice(b"tiles/");
    path.get_mut(70..).ok_or(VerificationError::Integrity)?.copy_from_slice(b".zst");
    for (i, byte) in hash.iter().enumerate() {
        let at = i.checked_mul(2).and_then(|n| n.checked_add(6)).ok_or(VerificationError::Integrity)?;
        *path.get_mut(at).ok_or(VerificationError::Integrity)? = *b"0123456789abcdef".get(usize::from(byte >> 4)).ok_or(VerificationError::Integrity)?;
        *path.get_mut(at + 1).ok_or(VerificationError::Integrity)? = *b"0123456789abcdef".get(usize::from(byte & 15)).ok_or(VerificationError::Integrity)?;
    }
    let name = std::str::from_utf8(&path).map_err(|_| VerificationError::Integrity)?;
    let stored = archive.stored_size(name).map_err(|error| match error {
        FormatError::Unsupported(_) => VerificationError::Unsupported,
        other => VerificationError::Container(other),
    })?;
    if stored > encoded {
        return Err(VerificationError::Integrity);
    }
    let mut verifier = Verifier {
        control,
        stage: &mut stage,
        header: [0; 14],
        have: 0,
        frame: true,
        remaining: 0,
        last: false,
        done: false,
        offset: 0,
        length,
        hasher: blake3::Hasher::new(),
        error: None,
    };
    let copied = archive.copy_by_name(name, &mut verifier, encoded, control.interrupt);
    if let Some(error) = verifier.error.take() {
        return Err(error);
    }
    let count = copied.map_err(VerificationError::Container)?;
    if count != encoded || !verifier.done || verifier.offset != length || verifier.hasher.finalize().as_bytes() != &hash {
        return Err(VerificationError::Integrity);
    }
    control.check(1).map_err(VerificationError::Backing)?;
    Ok(VerifiedCanonicalTile { format, length, hash })
}
type StagingCallback<'s> = dyn FnMut(u64, &[u8]) -> Result<(), BackingError> + 's;
struct Verifier<'c, 'a> {
    control: &'c ReadControl<'a>,
    stage: &'c mut StagingCallback<'c>,
    header: [u8; 14],
    have: usize,
    frame: bool,
    remaining: u64,
    last: bool,
    done: bool,
    offset: u64,
    length: u64,
    hasher: blake3::Hasher,
    error: Option<VerificationError>,
}
impl Verifier<'_, '_> {
    fn consume(&mut self, mut bytes: &[u8]) -> Result<(), VerificationError> {
        while !bytes.is_empty() {
            self.control.check(1).map_err(VerificationError::Backing)?;
            if self.done {
                return Err(VerificationError::Integrity);
            }
            if self.remaining > 0 {
                let count = bytes.len().min(4096).min(usize::try_from(self.remaining).map_err(|_| VerificationError::Integrity)?);
                let block = bytes.get(..count).ok_or(VerificationError::Integrity)?;
                let next = self.offset.checked_add(count as u64).ok_or(VerificationError::Integrity)?;
                self.control.check(count as u64).map_err(VerificationError::Backing)?;
                (self.stage)(self.offset, block).map_err(VerificationError::Backing)?;
                self.hasher.update(block);
                self.control.check(count as u64).map_err(VerificationError::Backing)?;
                self.offset = next;
                self.remaining = self.remaining.checked_sub(count as u64).ok_or(VerificationError::Integrity)?;
                bytes = bytes.get(count..).ok_or(VerificationError::Integrity)?;
                if self.remaining == 0 && self.last {
                    self.done = true;
                }
                continue;
            }
            let target: usize = if self.frame { 14 } else { 3 };
            let count = bytes.len().min(target.checked_sub(self.have).ok_or(VerificationError::Integrity)?);
            self.header
                .get_mut(self.have..self.have + count)
                .ok_or(VerificationError::Integrity)?
                .copy_from_slice(bytes.get(..count).ok_or(VerificationError::Integrity)?);
            self.have += count;
            bytes = bytes.get(count..).ok_or(VerificationError::Integrity)?;
            if self.have != target {
                continue;
            }
            self.have = 0;
            if self.frame {
                if self.header.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd]) {
                    return Err(VerificationError::Integrity);
                }
                if self.header.get(4..6) != Some(&[0xc0, 0x38]) {
                    return Err(VerificationError::Unsupported);
                }
                let declared = u64::from_le_bytes(self.header.get(6..14).and_then(|b| b.try_into().ok()).ok_or(VerificationError::Integrity)?);
                if declared != self.length {
                    return Err(VerificationError::Integrity);
                }
                self.frame = false;
            } else {
                let head: [u8; 3] = self.header.get(..3).and_then(|bytes| bytes.try_into().ok()).ok_or(VerificationError::Integrity)?;
                let bits = u32::from_le_bytes([head[0], head[1], head[2], 0]);
                if (bits >> 1) & 3 != 0 {
                    return Err(VerificationError::Unsupported);
                }
                let left = self.length.checked_sub(self.offset).ok_or(VerificationError::Integrity)?;
                let expected = left.min(128 * 1024);
                self.last = bits & 1 != 0;
                self.remaining = u64::from(bits >> 3);
                if self.remaining != expected || self.last != (left <= 128 * 1024) || self.remaining == 0 {
                    return Err(VerificationError::Integrity);
                }
            }
        }
        Ok(())
    }
}
impl Write for Verifier<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.error.is_some() {
            return Err(std::io::Error::other("failed canonical verifier"));
        }
        match self.consume(bytes) {
            Ok(()) => Ok(bytes.len()),
            Err(error) => {
                self.error = Some(error);
                Err(std::io::Error::other("canonical verification failed"))
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
