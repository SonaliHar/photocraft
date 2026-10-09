//! Standalone, bounded XCF reader. No filesystem, network, or GIMP runtime.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
mod error;
mod pixels;
mod properties;
mod reader;
mod types;
pub use error::{ErrorKind, XcfError};
pub use types::*;

/// Recognizes plain GIMP XCF, irrespective of filename.
pub fn is_xcf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"gimp xcf ")
}
/// Decode plain XCF within caller budgets and cancellation policy.
pub fn decode(bytes: &[u8], options: &DecodeOptions<'_>) -> Result<XcfFile, XcfError> {
    reader::decode(bytes, options)
}
