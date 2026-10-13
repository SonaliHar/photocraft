//! Safe initialized ring storage. Regenerated-block limits are checked before
//! this storage grows; no raw allocation or pointer-copy implementation remains.
use super::errors::DecodeBufferError;
use crate::io::Read;
use alloc::collections::VecDeque;

pub struct RingBuffer {
    values: VecDeque<u8>,
}
impl RingBuffer {
    pub fn logical_capacity_bytes(&self) -> usize {
        self.values.capacity()
    }
    pub fn new() -> Self {
        Self { values: VecDeque::new() }
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn reserve(&mut self, amount: usize) -> Result<(), DecodeBufferError> {
        let target = self.len().checked_add(amount).ok_or(DecodeBufferError::AllocationOrCapacity)?;
        let envelope = target.checked_next_power_of_two().ok_or(DecodeBufferError::AllocationOrCapacity)?;
        if target > self.values.capacity() {
            self.values.try_reserve_exact(amount).map_err(|_| DecodeBufferError::AllocationOrCapacity)?;
            if self.values.capacity() > envelope {
                return Err(DecodeBufferError::AllocationOrCapacity);
            }
        }
        Ok(())
    }
    pub fn get(&self, index: usize) -> Option<u8> {
        self.values.get(index).copied()
    }
    pub fn extend(&mut self, bytes: &[u8]) -> Result<(), DecodeBufferError> {
        self.reserve(bytes.len())?;
        self.values.extend(bytes.iter().copied());
        Ok(())
    }
    pub fn drop_first_n(&mut self, amount: usize) {
        let count = amount.min(self.values.len());
        self.values.drain(..count);
    }
    pub fn as_slices(&self) -> (&[u8], &[u8]) {
        self.values.as_slices()
    }
    pub fn extend_from_within(&mut self, start: usize, length: usize) -> Result<(), DecodeBufferError> {
        let end = start.checked_add(length).ok_or(DecodeBufferError::OffsetTooBig { offset: start, buf_len: self.len() })?;
        if end > self.len() {
            return Err(DecodeBufferError::OffsetTooBig { offset: end, buf_len: self.len() });
        }
        self.reserve(length)?;
        for index in start..end {
            let byte = self.get(index).ok_or(DecodeBufferError::OffsetTooBig { offset: index, buf_len: self.len() })?;
            self.values.push_back(byte);
        }
        Ok(())
    }
    pub fn extend_and_fill(&mut self, byte: u8, length: usize) -> Result<(), DecodeBufferError> {
        self.reserve(length)?;
        self.values.extend(core::iter::repeat_n(byte, length));
        Ok(())
    }
    pub fn extend_from_reader<R: Read>(&mut self, mut input: R, length: usize) -> Result<(), crate::io::Error> {
        self.reserve(length).map_err(io_error)?;
        let mut chunk = [0_u8; 4096];
        let mut left = length;
        while left != 0 {
            let count = left.min(chunk.len());
            input.read_exact(&mut chunk[..count])?;
            self.extend(&chunk[..count]).map_err(io_error)?;
            left -= count;
        }
        Ok(())
    }
}
fn io_error(error: DecodeBufferError) -> crate::io::Error {
    #[cfg(feature = "std")]
    {
        crate::io::Error::other(error)
    }
    #[cfg(not(feature = "std"))]
    {
        crate::io::Error::new(crate::io::ErrorKind::Other, alloc::boxed::Box::new(error))
    }
}
