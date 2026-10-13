/// Actionable error category; source offsets are reported separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Header,
    Version,
    Structure,
    Truncated,
    Compression,
    Unsupported,
    Limit,
    Allocation,
    Cancelled,
}
#[derive(Clone, Debug, thiserror::Error)]
#[error("XCF {kind:?} at byte {offset}{item}: {message}")]
pub struct XcfError {
    pub kind: ErrorKind,
    pub offset: u64,
    pub item: String,
    pub message: String,
}
impl XcfError {
    pub(crate) fn at(kind: ErrorKind, offset: u64, message: impl Into<String>) -> Self {
        Self { kind, offset, item: String::new(), message: message.into() }
    }
    pub(crate) fn new(kind: ErrorKind, offset: usize, message: impl Into<String>) -> Self {
        Self { kind, offset: offset as u64, item: String::new(), message: message.into() }
    }
}
