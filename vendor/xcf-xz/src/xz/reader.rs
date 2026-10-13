use super::{BlockHeader, ChecksumCalculator, Index, StreamFooter, StreamHeader};
use crate::{
    Control, CountingReader, Lzma2Reader, Read, Result, error_invalid_data, error_out_of_memory,
    filter::{FilterType, bcj::BcjReader, delta::DeltaReader},
};

struct One<T>(alloc::boxed::Box<[T; 1]>);
impl<T> One<T> {
    fn new(value: T) -> Result<Self> {
        let mut v = alloc::vec::Vec::new();
        v.try_reserve_exact(1).map_err(|_| error_out_of_memory("filter holder allocation"))?;
        if v.capacity() != 1 {
            return Err(error_out_of_memory("unexpected holder capacity"));
        }
        v.push(value);
        // No excess capacity: into_boxed_slice has nothing to shrink/reallocate.
        let array = v.into_boxed_slice().try_into().map_err(|_| error_invalid_data("filter holder shape"))?;
        Ok(Self(array))
    }
    fn into_value(self) -> T {
        let [v] = *self.0;
        v
    }
}
impl<T> core::ops::Deref for One<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0[0]
    }
}
impl<T> core::ops::DerefMut for One<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0[0]
    }
}
impl<T: Read> Read for One<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0[0].read(buf)
    }
}
// Fixed stack state avoids another allocator; heap holders are fallible and charged.
#[allow(clippy::large_enum_variant)]
enum FilterReader<R: Read + crate::Control> {
    Counting(CountingReader<R>),
    Lzma2(Lzma2Reader<One<FilterReader<R>>>),
    Delta(DeltaReader<One<FilterReader<R>>>),
    Bcj(BcjReader<One<FilterReader<R>>>),
    Dummy,
}

impl<R: Read + crate::Control> Read for FilterReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            FilterReader::Counting(reader) => reader.read(buf),
            FilterReader::Lzma2(reader) => reader.read(buf),
            FilterReader::Delta(reader) => reader.read(buf),
            FilterReader::Bcj(reader) => reader.read(buf),
            FilterReader::Dummy => Err(error_invalid_data("invalid decoder state")),
        }
    }
}

impl<R: Read + crate::Control> FilterReader<R> {
    fn create_filter_chain(mut inner: R, filters: &[Option<FilterType>], properties: &[u32]) -> Result<Self> {
        inner.poll()?;
        let mut chain_reader = FilterReader::Counting(CountingReader::new(inner));

        for (filter, property) in filters.iter().copied().zip(properties).filter_map(|(filter, property)| filter.map(|filter| (filter, *property))).rev() {
            chain_reader = match filter {
                FilterType::Delta => {
                    let distance = property as usize;
                    FilterReader::Delta(DeltaReader::new(One::new(chain_reader)?, distance))
                }
                FilterType::BcjX86 => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_x86(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjPpc => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_ppc(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjIa64 => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_ia64(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjArm => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_arm(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjArmThumb => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_arm_thumb(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjSparc => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_sparc(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjArm64 => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_arm64(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::BcjRiscv => {
                    let start_offset = property as usize;
                    FilterReader::Bcj(BcjReader::new_riscv(One::new(chain_reader)?, start_offset)?)
                }
                FilterType::Lzma2 => {
                    let dict_size = property;
                    FilterReader::Lzma2(Lzma2Reader::new(One::new(chain_reader)?, dict_size)?)
                }
            };
        }

        Ok(chain_reader)
    }

    fn bytes_read(&self) -> Result<u64> {
        match self {
            FilterReader::Counting(reader) => Ok(reader.bytes_read()),
            FilterReader::Lzma2(reader) => reader.inner().bytes_read(),
            FilterReader::Delta(reader) => reader.inner().bytes_read(),
            FilterReader::Bcj(reader) => reader.inner().bytes_read(),
            FilterReader::Dummy => Err(error_invalid_data("invalid decoder state")),
        }
    }

    fn into_inner(self) -> Result<R> {
        match self {
            FilterReader::Counting(reader) => Ok(reader.inner),
            FilterReader::Lzma2(reader) => {
                let filter_reader = reader.into_inner().into_value();
                filter_reader.into_inner()
            }
            FilterReader::Delta(reader) => {
                let filter_reader = reader.into_inner().into_value();
                filter_reader.into_inner()
            }
            FilterReader::Bcj(reader) => {
                let filter_reader = reader.into_inner().into_value();
                filter_reader.into_inner()
            }
            FilterReader::Dummy => Err(error_invalid_data("invalid decoder state")),
        }
    }

    fn inner_mut(&mut self) -> Result<&mut R> {
        match self {
            FilterReader::Counting(reader) => Ok(&mut reader.inner),
            FilterReader::Lzma2(reader) => {
                let filter_reader = reader.inner_mut();
                filter_reader.inner_mut()
            }
            FilterReader::Delta(reader) => {
                let filter_reader = reader.inner_mut();
                filter_reader.inner_mut()
            }
            FilterReader::Bcj(reader) => {
                let filter_reader = reader.inner_mut();
                filter_reader.inner_mut()
            }
            FilterReader::Dummy => Err(error_invalid_data("invalid decoder state")),
        }
    }
}

/// Checked reservation for all chain state, including literal replacement overlap.
fn check_memory_limit<R: Read + crate::Control>(block_header: &BlockHeader, mem_limit_kb: u32) -> Result<usize> {
    let mut bytes = core::mem::size_of::<XzReader<R>>().checked_add(65536 + 2 * 24576 + 65536).ok_or_else(|| error_out_of_memory("state overflow"))?;
    for (filter, property) in block_header.filters.iter().zip(&block_header.properties) {
        if let Some(filter) = filter {
            bytes = bytes.checked_add(core::mem::size_of::<FilterReader<R>>()).ok_or_else(|| error_out_of_memory("holder overflow"))?;
            let extra = if *filter == FilterType::Lzma2 {
                *property as usize
            } else if *filter == FilterType::Delta {
                0
            } else {
                4096
            };
            bytes = bytes.checked_add(extra).ok_or_else(|| error_out_of_memory("state overflow"))?;
        }
    }
    if bytes as u64 > u64::from(mem_limit_kb.min(131072)) * 1024 {
        return Err(error_out_of_memory("combined decoder state limit"));
    }
    Ok(bytes)
}

/// A single-threaded XZ decompressor.
pub(crate) struct XzReader<R: Read + crate::Control> {
    reader: FilterReader<R>,
    stream_header: Option<StreamHeader>,
    checksum_calculator: Option<ChecksumCalculator>,
    finished: bool,
    blocks_processed: u64,
    mem_limit_kb: u32,
}

impl<R: Read + crate::Control> XzReader<R> {
    /// Creates an XZ reader with a per-block LZMA2 decoder memory limit.
    ///
    /// `mem_limit_kb` is in KiB. the limit is always capped at 128 MiB.
    ///
    /// Reading a block returns an out of memory error before dictionary allocation if its
    /// dictionary and estimated LZMA2 decoder overhead exceed the limit.
    pub fn new_mem_limit(inner: R, mem_limit_kb: u32) -> Self {
        let reader = FilterReader::Counting(CountingReader::new(inner));

        Self { reader, stream_header: None, checksum_calculator: None, finished: false, blocks_processed: 0, mem_limit_kb }
    }
}

impl<R: Read + crate::Control> XzReader<R> {
    fn ensure_stream_header(&mut self) -> Result<()> {
        if self.stream_header.is_none() {
            let header = StreamHeader::parse(&mut self.reader)?;
            self.stream_header = Some(header);
        }
        Ok(())
    }

    fn prepare_next_block(&mut self) -> Result<bool> {
        match BlockHeader::parse(&mut self.reader)? {
            Some(block_header) => {
                // Check before replacing the reader to preserve access to the input on error.
                let reservation = check_memory_limit::<R>(&block_header, self.mem_limit_kb)?;
                self.reader.admit_state(reservation)?;
                self.reader.poll()?;
                let base_reader: FilterReader<R> = core::mem::replace(&mut self.reader, FilterReader::Dummy);

                self.reader = FilterReader::create_filter_chain(base_reader.into_inner()?, &block_header.filters, &block_header.properties)?;

                match self.stream_header.as_ref() {
                    Some(header) => {
                        self.checksum_calculator = Some(ChecksumCalculator::new(header.check_type));
                    }
                    None => {
                        return Err(error_invalid_data("stream_header not set"));
                    }
                }

                self.blocks_processed += 1;

                Ok(true)
            }
            None => {
                // End of blocks reached, index follows.
                self.parse_index_and_footer()?;

                self.finished = true;
                Ok(false)
            }
        }
    }

    fn consume_padding(&mut self, compressed_bytes: u64) -> Result<()> {
        let padding_needed = match (4 - (compressed_bytes % 4)) % 4 {
            0 => return Ok(()),
            n => n as usize,
        };

        let mut padding_buf = [0u8; 3];

        let bytes_read = self.reader.read(&mut padding_buf[..padding_needed])?;

        if bytes_read != padding_needed {
            return Err(error_invalid_data("incomplete XZ block padding"));
        }

        if !padding_buf[..bytes_read].iter().all(|&byte| byte == 0) {
            return Err(error_invalid_data("invalid XZ block padding"));
        }

        Ok(())
    }

    fn verify_block_checksum(&mut self) -> Result<()> {
        let checksum_calculator = self.checksum_calculator.take().ok_or_else(|| error_invalid_data("checksum_calculator not set"))?;

        match checksum_calculator {
            ChecksumCalculator::None => { /* Nothing to check */ }
            ChecksumCalculator::Crc32(_) => {
                let mut checksum = [0u8; 4];
                self.reader.read_exact(&mut checksum)?;

                if !checksum_calculator.verify(&checksum) {
                    return Err(error_invalid_data("invalid block checksum"));
                }
            }
            ChecksumCalculator::Crc64(_) => {
                let mut checksum = [0u8; 8];
                self.reader.read_exact(&mut checksum)?;

                if !checksum_calculator.verify(&checksum) {
                    return Err(error_invalid_data("invalid block checksum"));
                }
            }
            ChecksumCalculator::Sha256(_) => {
                let mut checksum = [0u8; 32];
                self.reader.read_exact(&mut checksum)?;

                if !checksum_calculator.verify(&checksum) {
                    return Err(error_invalid_data("invalid block checksum"));
                }
            }
        }

        Ok(())
    }

    /// Look for the start of the next stream by reading bytes one at a time
    /// and checking for the XZ magic sequence, allowing for stream padding.
    fn parse_index_and_footer(&mut self) -> Result<()> {
        let index = Index::parse(&mut self.reader)?;

        if index.number_of_records != self.blocks_processed {
            return Err(error_invalid_data("number of blocks processed doesn't match index records"));
        }

        let stream_footer = StreamFooter::parse(&mut self.reader)?;

        let header = self.stream_header.as_ref().ok_or_else(|| error_invalid_data("stream_header not set"))?;

        let header_flags = [0, header.check_type as u8];
        if stream_footer.stream_flags != header_flags {
            return Err(error_invalid_data("stream header and footer flags mismatch"));
        }

        Ok(())
    }
}

impl<R: Read + crate::Control> Read for XzReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.reader.poll()?;
        if self.mem_limit_kb == 0 {
            return Err(error_out_of_memory("zero decoder state limit"));
        }
        let cap = buf.len().min(4096);
        let buf = &mut buf[..cap];
        if self.finished {
            return Ok(0);
        }

        self.ensure_stream_header()?;

        loop {
            self.reader.poll()?;
            if self.checksum_calculator.is_some() {
                let bytes_read = self.reader.read(buf)?;

                if bytes_read > 0 {
                    if let Some(ref mut calc) = self.checksum_calculator {
                        calc.update(&buf[..bytes_read]);
                    }

                    return Ok(bytes_read);
                } else {
                    let reader = core::mem::replace(&mut self.reader, FilterReader::Dummy);
                    let compressed_bytes = reader.bytes_read()?;
                    self.reader = FilterReader::Counting(CountingReader::with_count(reader.into_inner()?, compressed_bytes));

                    self.consume_padding(compressed_bytes)?;
                    self.verify_block_checksum()?;
                }
            } else {
                // No current block, prepare the next one.
                if !self.prepare_next_block()? {
                    // No more blocks, we're done.
                    return Ok(0);
                }
            }
        }
    }
}

impl<T: crate::Control> crate::Control for One<T> {
    fn poll(&mut self) -> Result<()> {
        self.0[0].poll()
    }
    fn admit_state(&mut self, n: usize) -> Result<()> {
        self.0[0].admit_state(n)
    }
    fn admit_index(&mut self, n: usize) -> Result<()> {
        self.0[0].admit_index(n)
    }
}
impl<R: Read + crate::Control> crate::Control for FilterReader<R> {
    fn poll(&mut self) -> Result<()> {
        self.inner_mut()?.poll()
    }
    fn admit_state(&mut self, n: usize) -> Result<()> {
        self.inner_mut()?.admit_state(n)
    }
    fn admit_index(&mut self, n: usize) -> Result<()> {
        self.inner_mut()?.admit_index(n)
    }
}
