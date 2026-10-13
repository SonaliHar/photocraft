//! Allocation-free Zstandard raw-block frames for the streaming save path.
//!
//! RFC 8878 sections 3.1.1.1 and 3.1.1.2 permit raw blocks. A fixed 128 KiB
//! window keeps decoder residency independent of object length. Ordinary
//! incremental saves retain their existing compressed encoding.

use std::io::{self, Read};

use crate::{FormatError, Result};

const BLOCK_BYTES: usize = 128 * 1024;
const FRAME_HEADER_BYTES: usize = 14;

pub(crate) fn encoded_len(raw_bytes: u64) -> Result<u64> {
    let blocks = (raw_bytes / BLOCK_BYTES as u64).checked_add(u64::from(!raw_bytes.is_multiple_of(BLOCK_BYTES as u64))).ok_or_else(length_error)?.max(1);
    raw_bytes
        .checked_add(blocks.checked_mul(3).ok_or_else(length_error)?)
        .and_then(|length| length.checked_add(FRAME_HEADER_BYTES as u64))
        .ok_or_else(length_error)
}

fn length_error() -> FormatError {
    FormatError::LimitExceeded("streaming Zstandard frame length".into())
}

#[derive(Clone, Copy)]
enum Phase {
    Frame,
    Block,
    Payload,
    Done,
}

pub(crate) struct RawFrame<R> {
    raw: R,
    raw_len: u64,
    frame: [u8; FRAME_HEADER_BYTES],
    block: [u8; 3],
    phase: Phase,
    header_offset: usize,
    raw_offset: u64,
    block_end: u64,
}

impl<'a> RawFrame<&'a [u8]> {
    pub(crate) fn new(raw: &'a [u8]) -> Result<Self> {
        let length = u64::try_from(raw.len()).map_err(|_| length_error())?;
        Self::from_reader(raw, length)
    }
}

impl<R: Read> RawFrame<R> {
    /// The source must describe exactly `length` canonical bytes. This adapter
    /// owns no payload buffer and reads at most64KiB per call; the source owns
    /// its cancellation, integrity, lifetime and allocation accounting.
    pub(crate) fn from_reader(raw: R, length: u64) -> Result<Self> {
        encoded_len(length)?;
        // FCS flag 3, no single segment/checksum/dictionary; window exponent 7
        // declares 128 KiB. The eight-byte content size is little endian.
        let mut frame = [0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38, 0, 0, 0, 0, 0, 0, 0, 0];
        frame.get_mut(6..).ok_or_else(length_error)?.copy_from_slice(&length.to_le_bytes());
        Ok(Self { raw, raw_len: length, frame, block: [0; 3], phase: Phase::Frame, header_offset: 0, raw_offset: 0, block_end: 0 })
    }

    fn start_block(&mut self) -> io::Result<()> {
        let remaining = self.raw_len.checked_sub(self.raw_offset).ok_or_else(invalid_offset)?;
        let count = remaining.min(BLOCK_BYTES as u64);
        self.block_end = self.raw_offset.checked_add(count).ok_or_else(invalid_offset)?;
        let count = u32::try_from(count).map_err(|_| invalid_offset())?;
        let header = ((count << 3) | u32::from(self.block_end == self.raw_len)).to_le_bytes();
        self.block.copy_from_slice(header.get(..3).ok_or_else(invalid_offset)?);
        self.header_offset = 0;
        self.phase = Phase::Block;
        Ok(())
    }
}

fn invalid_offset() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid streaming Zstandard offset")
}

impl<R: Read> Read for RawFrame<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if matches!(self.phase, Phase::Payload) && self.raw_offset < self.block_end {
                let remaining = usize::try_from(self.block_end - self.raw_offset).map_err(|_| invalid_offset())?;
                let count = output.len().min(remaining).min(64 * 1024);
                let read = self.raw.read(output.get_mut(..count).ok_or_else(invalid_offset)?)?;
                if read == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short canonical Zstandard source"));
                }
                if read > count {
                    return Err(invalid_offset());
                }
                self.raw_offset = self.raw_offset.checked_add(read as u64).ok_or_else(invalid_offset)?;
                return Ok(read);
            }
            let source = match self.phase {
                Phase::Frame => self.frame.get(self.header_offset..).ok_or_else(invalid_offset)?,
                Phase::Block => self.block.get(self.header_offset..).ok_or_else(invalid_offset)?,
                Phase::Payload => &[],
                Phase::Done => return Ok(0),
            };
            let count = source.len().min(output.len());
            if count > 0 {
                output.get_mut(..count).ok_or_else(invalid_offset)?.copy_from_slice(source.get(..count).ok_or_else(invalid_offset)?);
                match self.phase {
                    Phase::Frame | Phase::Block => self.header_offset = self.header_offset.checked_add(count).ok_or_else(invalid_offset)?,
                    Phase::Payload => return Err(invalid_offset()),
                    Phase::Done => return Err(invalid_offset()),
                }
                return Ok(count);
            }
            match self.phase {
                Phase::Frame => self.start_block()?,
                Phase::Block => self.phase = Phase::Payload,
                Phase::Payload if self.raw_offset == self.raw_len => self.phase = Phase::Done,
                Phase::Payload => self.start_block()?,
                Phase::Done => return Ok(0),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frames_decode_at_empty_and_block_boundaries_with_fragmented_reads() {
        for length in [0, 1, 255, BLOCK_BYTES - 1, BLOCK_BYTES, BLOCK_BYTES + 1, 3 * BLOCK_BYTES + 9] {
            let raw: Vec<_> = (0..length).map(|index| (index.wrapping_mul(37) & 255) as u8).collect();
            for chunk in [1, 7, 64 * 1024] {
                let mut reader = RawFrame::new(&raw).unwrap();
                let mut encoded = Vec::new();
                let mut scratch = vec![0; chunk];
                loop {
                    let count = reader.read(&mut scratch).unwrap();
                    if count == 0 {
                        break;
                    }
                    encoded.extend_from_slice(&scratch[..count]);
                }
                assert_eq!(encoded.len() as u64, encoded_len(length as u64).unwrap());
                assert_eq!(&encoded[..6], &[0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38]);
                let mut decoded = Vec::new();
                ruzstd::decoding::StreamingDecoder::new(encoded.as_slice()).unwrap().read_to_end(&mut decoded).unwrap();
                assert_eq!(decoded, raw, "length {length}, read chunk {chunk}");
                assert_eq!(reader.read(&mut scratch).unwrap(), 0);
            }
        }
    }

    #[test]
    fn raw_frame_length_is_checked_without_allocating_payload() {
        assert_eq!(encoded_len(0).unwrap(), 17);
        assert_eq!(encoded_len(BLOCK_BYTES as u64).unwrap(), BLOCK_BYTES as u64 + 17);
        assert_eq!(encoded_len(BLOCK_BYTES as u64 + 1).unwrap(), BLOCK_BYTES as u64 + 21);
        assert!(encoded_len(u64::MAX).is_err());
    }

    struct Fragmented<'a> {
        bytes: &'a [u8],
        max_read: usize,
        fail_once: bool,
    }
    impl Read for Fragmented<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.fail_once {
                self.fail_once = false;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "injected canonical-source interruption"));
            }
            let count = out.len().min(self.max_read).min(self.bytes.len());
            out[..count].copy_from_slice(&self.bytes[..count]);
            self.bytes = &self.bytes[count..];
            Ok(count)
        }
    }

    #[test]
    fn owned_fragmented_source_matches_existing_frames_and_recovers_interruption() {
        let bytes: Vec<_> = (0..2 * BLOCK_BYTES + 17).map(|i| (i.wrapping_mul(29) & 255) as u8).collect();
        let mut expected = Vec::new();
        RawFrame::new(&bytes).unwrap().read_to_end(&mut expected).unwrap();
        for fragment in [1, 7, 4096, 65536] {
            let source = Fragmented { bytes: &bytes, max_read: fragment, fail_once: true };
            let mut reader = RawFrame::from_reader(source, bytes.len() as u64).unwrap();
            let mut actual = Vec::new();
            reader.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, expected, "source fragment {fragment}");
            let mut decoded = Vec::new();
            ruzstd::decoding::StreamingDecoder::new(actual.as_slice()).unwrap().read_to_end(&mut decoded).unwrap();
            assert_eq!(decoded, bytes);
        }
    }

    #[test]
    fn truncated_owned_source_errors_at_payload_and_u64_length_needs_no_payload_allocation() {
        let mut reader = RawFrame::from_reader(&b"abc"[..], 4).unwrap();
        let mut bytes = Vec::new();
        assert_eq!(reader.read_to_end(&mut bytes).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(&bytes[17..], b"abc");
        let length = (1u64 << 32) + 19;
        let mut reader = RawFrame::from_reader(io::empty(), length).unwrap();
        let mut header = [0; FRAME_HEADER_BYTES + 3];
        reader.read_exact(&mut header).unwrap();
        assert_eq!(&header[6..14], &length.to_le_bytes());
        assert_eq!(u32::from_le_bytes([header[14], header[15], header[16], 0]), (BLOCK_BYTES as u32) << 3);
        assert_eq!(reader.read(&mut [0; 1]).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert!(RawFrame::from_reader(io::empty(), u64::MAX).is_err());
    }
}
