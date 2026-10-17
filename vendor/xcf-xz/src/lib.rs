//! Checked, bounded single-threaded XZ decoding for the standalone XCF reader.
//!
//! Public decode requires encoded/logical/state/index caps and caller cancellation/deadline
//! and peak-reservation hooks. Structural preflight precedes allocation/decode; payload
//! CRC32/CRC64/SHA256 checks finish before returning bytes. Raw decoder APIs are private.
//!
//! Adapted from lzma-rust2 0.21.0 (Apache-2.0), with adjacent original LICENSE and ADAPTATION.md.
//! This package supports std + XZ only, using safe Rust. Encoding, LZIP, MT, no_std and
//! unsafe optimization features are excluded. Wasm uses Rust std compiled to WebAssembly.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(any(feature = "encoder", feature = "optimization", feature = "lzip"))]
compile_error!("unreviewed decoder features are not admitted");
#[cfg(not(all(feature = "std", feature = "xz")))]
compile_error!("reviewed prototype requires std + xz");

/// Caller-owned cancellation/deadline and peak-reservation hooks, with no persisted authority.
pub(crate) trait Control {
    /// Poll the visible token and monotonic deadline. An error aborts the decoder.
    fn poll(&mut self) -> Result<()>;
    /// Admit the complete retained/overlap decoder-state reservation before allocation.
    fn admit_state(&mut self, bytes: usize) -> Result<()>;
    /// Admit the complete index reservation before allocation.
    fn admit_index(&mut self, bytes: usize) -> Result<()>;
}

mod decoder;
mod lz;

mod lzma2_reader;

mod range_dec;
mod state;

#[cfg(feature = "xz")]
mod xz;

mod filter;

#[cfg(any(feature = "lzip", feature = "xz"))]
mod crc;

#[cfg(feature = "std")]
pub(crate) use std::io::Error;
#[cfg(feature = "std")]
pub(crate) use std::io::Read;
#[cfg(feature = "encoder")]
pub(crate) use std::io::Write;

#[cfg(feature = "encoder")]
pub use enc::*;

#[cfg(all(feature = "lzip", feature = "std"))]
pub use lzip::LzipReaderMt;
#[cfg(all(feature = "lzip", feature = "encoder", feature = "std"))]
pub use lzip::LzipWriterMt;
#[cfg(all(feature = "lzip", feature = "encoder"))]
pub use lzip::{LzipOptions, LzipWriter};
#[cfg(feature = "lzip")]
pub use lzip::{LzipReader, LzipStream};
pub(crate) use lzma2_reader::Lzma2Reader;
#[cfg(not(feature = "std"))]
pub use no_std::Error;
#[cfg(not(feature = "std"))]
pub use no_std::Read;
#[cfg(not(feature = "std"))]
pub use no_std::Write;
use state::*;

#[cfg(feature = "xz")]
pub(crate) use xz::XzReader;
#[cfg(all(feature = "xz", feature = "encoder", feature = "std"))]
pub use xz::XzWriterMt;
#[cfg(all(feature = "xz", feature = "encoder"))]
pub use xz::{XzOptions, XzWriter};

/// Result type of the crate.
#[cfg(feature = "std")]
pub(crate) type Result<T> = core::result::Result<T, Error>;

/// Result type of the crate.
#[cfg(not(feature = "std"))]
pub(crate) type Result<T> = core::result::Result<T, Error>;

const LOW_SYMBOLS: usize = 1 << 3;
const MID_SYMBOLS: usize = 1 << 3;
const HIGH_SYMBOLS: usize = 1 << 8;

const POS_STATES_MAX: usize = 1 << 4;
const MATCH_LEN_MIN: usize = 2;

const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 1 << 6;
const DIST_MODEL_START: usize = 4;
const DIST_MODEL_END: usize = 14;

const ALIGN_BITS: usize = 4;
const ALIGN_SIZE: usize = 1 << ALIGN_BITS;

const REPS: usize = 4;

const SHIFT_BITS: u32 = 8;
const BIT_MODEL_TOTAL_BITS: u32 = 11;
const BIT_MODEL_TOTAL: u32 = 1 << BIT_MODEL_TOTAL_BITS;
const PROB_INIT: u16 = (BIT_MODEL_TOTAL / 2) as u16;
const MOVE_BITS: u32 = 5;
const DIST_SPECIAL_INDEX: [usize; 10] = [0, 2, 4, 8, 12, 20, 28, 44, 60, 92];
const DIST_SPECIAL_END: [usize; 10] = [2, 4, 8, 12, 20, 28, 44, 60, 92, 124];
const RC_BIT_MODEL_OFFSET: u32 = (1u32 << MOVE_BITS).wrapping_sub(1).wrapping_sub(BIT_MODEL_TOTAL);

#[derive(Clone)]
pub(crate) struct LzmaCoder {
    pub(crate) pos_mask: u32,
    pub(crate) reps: [i32; REPS],
    pub(crate) state: State,
    pub(crate) is_match: [[u16; POS_STATES_MAX]; STATES],
    pub(crate) is_rep: [u16; STATES],
    pub(crate) is_rep0: [u16; STATES],
    pub(crate) is_rep1: [u16; STATES],
    pub(crate) is_rep2: [u16; STATES],
    pub(crate) is_rep0_long: [[u16; POS_STATES_MAX]; STATES],
    pub(crate) dist_slots: [[u16; DIST_SLOTS]; DIST_STATES],
    dist_special: [u16; 124],
    dist_align: [u16; ALIGN_SIZE],
}

pub(crate) fn coder_get_dict_size(len: usize) -> usize {
    if len < DIST_STATES + MATCH_LEN_MIN { len - MATCH_LEN_MIN } else { DIST_STATES - 1 }
}

impl LzmaCoder {
    pub(crate) fn new(pb: usize) -> Self {
        let mut c = Self {
            pos_mask: (1 << pb) - 1,
            reps: Default::default(),
            state: Default::default(),
            is_match: Default::default(),
            is_rep: Default::default(),
            is_rep0: Default::default(),
            is_rep1: Default::default(),
            is_rep2: Default::default(),
            is_rep0_long: Default::default(),
            dist_slots: [[Default::default(); DIST_SLOTS]; DIST_STATES],
            dist_special: [Default::default(); 124],
            dist_align: Default::default(),
        };
        c.reset();
        c
    }

    pub(crate) fn reset(&mut self) {
        self.reps = [0; REPS];
        self.state.reset();
        for ele in self.is_match.iter_mut() {
            init_probs(ele);
        }
        init_probs(&mut self.is_rep);
        init_probs(&mut self.is_rep0);
        init_probs(&mut self.is_rep1);
        init_probs(&mut self.is_rep2);

        for ele in self.is_rep0_long.iter_mut() {
            init_probs(ele);
        }
        for ele in self.dist_slots.iter_mut() {
            init_probs(ele);
        }
        init_probs(&mut self.dist_special);
        init_probs(&mut self.dist_align);
    }

    #[inline(always)]
    pub(crate) fn get_dist_special(&mut self, i: usize) -> &mut [u16] {
        &mut self.dist_special[DIST_SPECIAL_INDEX[i]..DIST_SPECIAL_END[i]]
    }
}

#[inline(always)]
pub(crate) fn init_probs(probs: &mut [u16]) {
    probs.fill(PROB_INIT);
}

#[derive(Clone)]
pub(crate) struct LiteralCoder {
    lc: u32,
    literal_pos_mask: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LiteralSubCoder {
    probs: [u16; 0x300],
}

impl LiteralSubCoder {
    pub fn new() -> Self {
        let probs = [PROB_INIT; 0x300];
        Self { probs }
    }

    pub fn reset(&mut self) {
        self.probs = [PROB_INIT; 0x300];
    }
}

impl LiteralCoder {
    pub fn new(lc: u32, lp: u32) -> Self {
        Self { lc, literal_pos_mask: (1 << lp) - 1 }
    }

    pub(crate) fn get_sub_coder_index(&self, prev_byte: u32, pos: u32) -> u32 {
        let low = prev_byte >> (8 - self.lc);
        let high = (pos & self.literal_pos_mask) << self.lc;
        low + high
    }
}

#[derive(Clone)]
pub(crate) struct LengthCoder {
    choice: [u16; 2],
    low: [[u16; LOW_SYMBOLS]; POS_STATES_MAX],
    mid: [[u16; MID_SYMBOLS]; POS_STATES_MAX],
    high: [u16; HIGH_SYMBOLS],
}

impl LengthCoder {
    pub fn new() -> Self {
        Self { choice: Default::default(), low: Default::default(), mid: Default::default(), high: [0; HIGH_SYMBOLS] }
    }

    pub fn reset(&mut self) {
        init_probs(&mut self.choice);
        for ele in self.low.iter_mut() {
            init_probs(ele);
        }
        for ele in self.mid.iter_mut() {
            init_probs(ele);
        }
        init_probs(&mut self.high);
    }
}

trait ByteReader {
    fn read_u8(&mut self) -> Result<u8>;

    fn read_u16_be(&mut self) -> Result<u16>;

    fn read_u32(&mut self) -> Result<u32>;

    fn read_u32_be(&mut self) -> Result<u32>;
}

impl<T: Read> ByteReader for T {
    #[inline(always)]
    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    #[inline(always)]
    fn read_u16_be(&mut self) -> Result<u16> {
        let mut buf = [0; 2];
        self.read_exact(buf.as_mut())?;
        Ok(u16::from_be_bytes(buf))
    }

    #[inline(always)]
    fn read_u32(&mut self) -> Result<u32> {
        let mut buf = [0; 4];
        self.read_exact(buf.as_mut())?;
        Ok(u32::from_le_bytes(buf))
    }

    #[inline(always)]
    fn read_u32_be(&mut self) -> Result<u32> {
        let mut buf = [0; 4];
        self.read_exact(buf.as_mut())?;
        Ok(u32::from_be_bytes(buf))
    }
}

#[cfg(feature = "std")]
#[cfg(feature = "std")]
#[inline(always)]
fn error_other(msg: &'static str) -> Error {
    Error::other(msg)
}

#[cfg(feature = "std")]
#[inline(always)]
fn error_invalid_input(msg: &'static str) -> Error {
    Error::new(std::io::ErrorKind::InvalidInput, msg)
}

#[cfg(feature = "std")]
#[inline(always)]
fn error_invalid_data(msg: &'static str) -> Error {
    Error::new(std::io::ErrorKind::InvalidData, msg)
}

#[cfg(feature = "std")]
#[inline(always)]
fn error_out_of_memory(msg: &'static str) -> Error {
    Error::new(std::io::ErrorKind::OutOfMemory, msg)
}

#[cfg(feature = "std")]
#[inline(always)]
fn error_unsupported(msg: &'static str) -> Error {
    Error::new(std::io::ErrorKind::Unsupported, msg)
}

#[cfg(feature = "std")]
#[cfg(not(feature = "std"))]
#[cfg(not(feature = "std"))]
#[inline(always)]
fn error_other(msg: &'static str) -> Error {
    Error::Other(msg)
}

#[cfg(not(feature = "std"))]
#[inline(always)]
fn error_invalid_input(msg: &'static str) -> Error {
    Error::InvalidInput(msg)
}

#[cfg(not(feature = "std"))]
#[inline(always)]
fn error_invalid_data(msg: &'static str) -> Error {
    Error::InvalidData(msg)
}

#[cfg(not(feature = "std"))]
#[inline(always)]
fn error_out_of_memory(msg: &'static str) -> Error {
    Error::OutOfMemory(msg)
}

#[cfg(not(feature = "std"))]
#[inline(always)]
fn error_unsupported(msg: &'static str) -> Error {
    Error::Unsupported(msg)
}

/// Counts consumed compressed bytes for block padding.
struct CountingReader<R> {
    inner: R,
    bytes_read: u64,
}

impl<R> CountingReader<R> {
    fn new(inner: R) -> Self {
        Self { inner, bytes_read: 0 }
    }

    fn with_count(inner: R, bytes_read: u64) -> Self {
        Self { inner, bytes_read }
    }

    fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let read_size = self.inner.read(buf)?;
        self.bytes_read += read_size as u64;
        Ok(read_size)
    }
}

#[cfg(feature = "encoder")]
struct CountingWriter<W> {
    inner: W,
    bytes_written: u64,
}

#[cfg(feature = "encoder")]
impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, bytes_written: 0 }
    }

    fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

#[cfg(feature = "encoder")]
impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let bytes_written = self.inner.write(buf)?;
        self.bytes_written += bytes_written as u64;
        Ok(bytes_written)
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}

mod bounded;
mod strict_xz;
pub use bounded::{Reservation, decode_prefix_with_reservation, decode_with_reservation};
