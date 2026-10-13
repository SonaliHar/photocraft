//! Bounded canonical little-endian tile bytes from an exact current owned pin.
//! No Document/native admission or locator authority is implied.
use photocraft_doc::BackedLayer;
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
    checked_view::{CheckedTilePin, NativeTileBytes, RasterViewError},
};
use std::io::{Read, Seek, SeekFrom};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalTileError {
    Backing(BackingError),
    View(RasterViewError),
    Bounds,
    Length,
}
impl std::fmt::Display for CanonicalTileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "canonical tile: {self:?}")
    }
}
impl std::error::Error for CanonicalTileError {}
/// Owns one physical pin and a prepaid4KiB inline transactional buffer. Borrowed
/// work/cancellation belongs to this writer call, never to persisted authority.
/// The pin/scratch fields drop before the retained same-ledger working lease.
pub struct CanonicalTileReader<'c, 'a> {
    pin: CheckedTilePin,
    scratch: [u8; 4096],
    position: u64,
    length: u64,
    control: &'c ReadControl<'a>,
    _lease: QuotaLease,
}
impl<'c, 'a> CanonicalTileReader<'c, 'a> {
    pub fn new(layer: &BackedLayer, coord: TileCoord, control: &'c ReadControl<'a>) -> Result<Self, CanonicalTileError> {
        control.check(1).map_err(CanonicalTileError::Backing)?;
        let lease = layer
            .quota()
            .reserve(QuotaKind::Resident, (std::mem::size_of::<Self>() as u64).checked_add(128).ok_or(CanonicalTileError::Bounds)?)
            .map_err(|_| CanonicalTileError::Backing(BackingError::Quota))?;
        let pin = layer.checked_view().pin_tile(coord, control).map_err(CanonicalTileError::View)?;
        let length = (TILE_SIZE as u64)
            .checked_mul(TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(pin.format().bytes_per_pixel() as u64))
            .ok_or(CanonicalTileError::Bounds)?;
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) if bytes.len() as u64 != length => return Err(CanonicalTileError::Length),
            NativeTileBytes::DefaultPixel(bytes) if bytes.len() != pin.format().bytes_per_pixel() => return Err(CanonicalTileError::Length),
            _ => {}
        }
        let out = Self { pin, scratch: [0; 4096], position: 0, length, control, _lease: lease };
        control.check(4096).map_err(CanonicalTileError::Backing)?;
        Ok(out)
    }
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn position(&self) -> u64 {
        self.position
    }
    /// Successful reads may be short (<=4KiB); caller output and position remain
    /// unchanged on every returned error. Partial samples are supported bitwise.
    pub fn try_read(&mut self, output: &mut [u8]) -> Result<usize, CanonicalTileError> {
        self.control.check(1).map_err(CanonicalTileError::Backing)?;
        let count = usize::try_from((self.length.checked_sub(self.position).ok_or(CanonicalTileError::Bounds)?).min(output.len() as u64).min(4096))
            .map_err(|_| CanonicalTileError::Bounds)?;
        let next = self.position.checked_add(count as u64).ok_or(CanonicalTileError::Bounds)?;
        let sample = self.pin.format().sample.bytes();
        if sample == 0 {
            return Err(CanonicalTileError::Length);
        }
        let bytes = self.pin.encoded();
        for index in 0..count {
            if index % 256 == 0 {
                self.control.check((count - index).min(256) as u64).map_err(CanonicalTileError::Backing)?;
            }
            let canonical = usize::try_from(self.position).map_err(|_| CanonicalTileError::Bounds)?.checked_add(index).ok_or(CanonicalTileError::Bounds)?;
            let native = native_index(canonical, sample, cfg!(target_endian = "big"))?;
            let byte = match bytes {
                NativeTileBytes::FullTile(data) => data.get(native),
                NativeTileBytes::DefaultPixel(data) => data.get(native % self.pin.format().bytes_per_pixel()),
            }
            .copied()
            .ok_or(CanonicalTileError::Length)?;
            *self.scratch.get_mut(index).ok_or(CanonicalTileError::Length)? = byte;
        }
        self.control.check(count as u64).map_err(CanonicalTileError::Backing)?;
        output.get_mut(..count).ok_or(CanonicalTileError::Length)?.copy_from_slice(self.scratch.get(..count).ok_or(CanonicalTileError::Length)?);
        self.position = next;
        Ok(count)
    }
    pub fn try_seek(&mut self, from: SeekFrom) -> Result<u64, CanonicalTileError> {
        self.control.check(1).map_err(CanonicalTileError::Backing)?;
        let proposed = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        let value = u64::try_from(proposed).map_err(|_| CanonicalTileError::Bounds)?;
        if value > self.length {
            return Err(CanonicalTileError::Bounds);
        }
        self.control.check(1).map_err(CanonicalTileError::Backing)?;
        self.position = value;
        Ok(value)
    }
}
impl Read for CanonicalTileReader<'_, '_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.try_read(out).map_err(std::io::Error::other)
    }
}
impl Seek for CanonicalTileReader<'_, '_> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.try_seek(from).map_err(std::io::Error::other)
    }
}

fn native_index(canonical: usize, sample: usize, big_endian: bool) -> Result<usize, CanonicalTileError> {
    if sample == 0 {
        return Err(CanonicalTileError::Length);
    }
    if big_endian {
        let base = canonical / sample * sample;
        base.checked_add(sample - 1 - canonical % sample).ok_or(CanonicalTileError::Bounds)
    } else {
        Ok(canonical)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;
    use photocraft_raster::backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    };
    use std::sync::Arc;
    #[test]
    fn explicit_endian_mapping_preserves_arbitrary_fragmented_sample_bits() {
        for sample in [1, 2, 4, 8] {
            for index in 0..3 * sample {
                assert_eq!(native_index(index, sample, false).unwrap(), index);
                assert_eq!(native_index(index, sample, true).unwrap(), index / sample * sample + (sample - 1 - index % sample));
            }
        }
        let big_nan = [0x7f, 0xf8, 0, 0, 0, 0, 0, 1];
        let expected = [1, 0, 0, 0, 0, 0, 0xf8, 0x7f];
        assert_eq!(std::array::from_fn::<_, 8, _>(|index| big_nan[native_index(index, 8, true).unwrap()]), expected);
        assert_eq!(native_index(0, 0, true), Err(CanonicalTileError::Length));
    }
    #[derive(Debug)]
    struct Literal(NativePlaneDescriptor);
    impl NativePlaneStore for Literal {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.0
        }
        fn read_exact_at(&self, _: u64, out: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
            control.check(out.len() as u64)?;
            out.fill(0x23);
            Ok(())
        }
    }
    #[test]
    fn actual_canonical_reader_raw_frame_and_independent_ruzstd_decode_agree() {
        let d = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]).unwrap();
        let quota = BackingQuota::new(QuotaLimits { resident: 4 << 20, scratch: 0, index: 1 << 20 });
        let control = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) };
        let raster = BackedRaster::new(Arc::new(Literal(d)), quota.clone(), 1, &control).unwrap();
        let layer = BackedLayer::new(raster, d, &control).unwrap();
        let reader = CanonicalTileReader::new(&layer, TileCoord::new(0, 0), &control).unwrap();
        let length = reader.length();
        let mut frame = crate::zstd_stream::RawFrame::from_reader(reader, length).unwrap();
        let mut encoded = Vec::new();
        frame.read_to_end(&mut encoded).unwrap();
        let mut decoded = Vec::new();
        ruzstd::decoding::StreamingDecoder::new(encoded.as_slice()).unwrap().read_to_end(&mut decoded).unwrap();
        let mut expected = vec![0; 256 * 256 * 4];
        expected[..4].fill(0x23);
        assert_eq!(decoded, expected);
        assert_eq!(blake3::hash(&decoded), blake3::hash(&expected));
        drop((frame, layer));
        assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    }
}
