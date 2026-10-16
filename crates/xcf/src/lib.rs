//! Standalone, bounded XCF reader. No path resolution, network, or GIMP runtime.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
mod arguments;
mod configuration;
mod container;
mod error;
mod graph;
mod markup;
mod paths;
mod pixels;
mod properties;
mod reader;
mod sink;
mod source;
mod text;
mod text_layout;
mod types;
mod owned_backing_census;
mod vector_layer;
pub use container::{read_plain_source_metadata_with_budget,PlainSourceMetadata};
pub use container::{
    BudgetDecodedImport, DecodeReservations, DecodedImport, SourceAccounting, SourceContainer, classify_container, decode_candidate,
    decode_candidate_with_budget, decode_with_resources,
};
pub use error::{ErrorKind, XcfError};
pub use sink::*;
pub use source::*;
pub use types::*;
pub use owned_backing_census::{OwnedBackingLimits, OwnedBackingRequirements, census_owned_file};

/// Accounting assertion for an immutable caller-owned plain source, not an IO grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlainSourceAccounting {
    BorrowedMemory,
    /// Trusted caller keeps Encoded and actual Resident storage leases live through mapping.
    CallerReservedMemory,
    /// Trusted caller keeps Encoded and Scratch leases plus actual resident metadata live.
    /// This assertion does not prove storage kind or grant filesystem authority.
    CallerReservedScratch,
}

pub use container::{decode_plain_source_into_sink_with_budget, decode_plain_source_with_budget};

/// Recognizes plain GIMP XCF, irrespective of filename.
pub fn is_xcf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"gimp xcf ")
}
/// Decode plain or singly wrapped XCF within caller budgets and cancellation policy.
pub fn decode(bytes: &[u8], options: &DecodeOptions<'_>) -> Result<XcfFile, XcfError> {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default())?;
    Ok(decode_with_resources(bytes, options, &limits)?.file)
}
