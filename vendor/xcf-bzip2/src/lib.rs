//! Bounded safe decoder prototype, adapted from bzip2-rs 0.1.2.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#[cfg(any(feature = "nightly", feature = "rustc_1_37", feature = "rustc_1_40", feature = "rustc_1_51"))]
compile_error!("legacy alternative decoder features are not admitted");
mod bitreader;
mod block;
mod bounded;
mod crc;
mod header;
mod huffman;
mod move_to_front;
pub use bounded::{Reservation, decode_prefix_with_reservation, decode_with_reservation};
const LEN_258: usize = 512;
mod randtable;
