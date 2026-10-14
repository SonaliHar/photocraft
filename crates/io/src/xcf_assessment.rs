//! Structured XCF failure details. Success fidelity and dependency recovery are separate.
//! Reports never echo untrusted payloads or grant access to source-stored locators.
use crate::IoError;
use photocraft_xcf::ErrorKind;
use serde::Serialize;

#[path = "xcf_success.rs"]
mod success;
pub use success::{DependencyAssessment, DependencyStatus, Fidelity, ImportAssessment, SuccessDiagnostic, assess_prepared_xcf_document, assess_xcf_document};
pub(crate) use success::{after_qualified_import, allocation_bound};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    InvalidXcf,
    UnsupportedXcfVersion,
    InvalidStructure,
    TruncatedXcf,
    InvalidContainer,
    UnsupportedFeature,
    ResourceLimit,
    Cancelled,
    ImportFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureDiagnostic {
    pub code: DiagnosticCode,
    pub severity: &'static str,
    pub message: &'static str,
    /// Canonical decimal avoids loss of offsets in JSON consumers with f64 numbers.
    pub source_offset: Option<String>,
    pub affected_capability: &'static str,
    /// Actions are advertised only when the corresponding recovery is implemented.
    pub recovery_actions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureReport {
    pub source_format: &'static str,
    pub outcome: &'static str,
    pub diagnostics: Vec<FailureDiagnostic>,
}

impl FailureReport {
    /// Call after an XCF import attempt; this does not identify arbitrary input formats.
    /// Generic admission errors stay generic rather than parsing their display strings
    /// into invented operation, resource or item identities.
    pub fn from_xcf_attempt(error: &IoError) -> Self {
        let (code, source_offset) = match error {
            IoError::Xcf(error) => {
                let code = match error.kind {
                    ErrorKind::Header => DiagnosticCode::InvalidXcf,
                    ErrorKind::Version => DiagnosticCode::UnsupportedXcfVersion,
                    ErrorKind::Structure => DiagnosticCode::InvalidStructure,
                    ErrorKind::Truncated => DiagnosticCode::TruncatedXcf,
                    ErrorKind::Compression => DiagnosticCode::InvalidContainer,
                    ErrorKind::Unsupported => DiagnosticCode::UnsupportedFeature,
                    ErrorKind::Limit | ErrorKind::Allocation => DiagnosticCode::ResourceLimit,
                    ErrorKind::Cancelled => DiagnosticCode::Cancelled,
                };
                (code, Some(error.offset.to_string()))
            }
            IoError::Cancelled => (DiagnosticCode::Cancelled, None),
            // Accounted preparation uses the native storage ledger as well as
            // the XCF decoder. Preserve its typed failures without inspecting
            // messages that may contain an untrusted path or payload.
            IoError::Pcraft(photocraft_format::FormatError::Cancelled) => (DiagnosticCode::Cancelled, None),
            IoError::Pcraft(photocraft_format::FormatError::LimitExceeded(_)) => (DiagnosticCode::ResourceLimit, None),
            IoError::Pcraft(photocraft_format::FormatError::Unsupported(_)) => (DiagnosticCode::UnsupportedFeature, None),
            IoError::Unsupported(_) => (DiagnosticCode::UnsupportedFeature, None),
            _ => (DiagnosticCode::ImportFailed, None),
        };
        let message = match code {
            DiagnosticCode::InvalidXcf => "The input is not a valid XCF project.",
            DiagnosticCode::UnsupportedXcfVersion => "This XCF version is unsupported.",
            DiagnosticCode::InvalidStructure => "The XCF project contains invalid structure or references.",
            DiagnosticCode::TruncatedXcf => "The XCF project is incomplete or truncated.",
            DiagnosticCode::InvalidContainer => "The XCF compressed container is invalid.",
            DiagnosticCode::UnsupportedFeature => "The XCF project requires an unsupported feature or setting.",
            DiagnosticCode::ResourceLimit => "The XCF import exceeded a supported resource limit or allocation failed.",
            DiagnosticCode::Cancelled => "The XCF import was cancelled.",
            DiagnosticCode::ImportFailed => "The XCF project could not be imported.",
        };
        Self {
            source_format: "xcf",
            outcome: "failure",
            diagnostics: vec![FailureDiagnostic { code, severity: "error", message, source_offset, affected_capability: "open", recovery_actions: Vec::new() }],
        }
    }
}

/// The ordinary error remains available to existing string/error consumers.
/// A failed attempt carries no document or success-fidelity claim.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct ImportFailure {
    #[source]
    pub error: IoError,
    pub details: FailureReport,
}

impl From<IoError> for ImportFailure {
    fn from(error: IoError) -> Self {
        let details = FailureReport::from_xcf_attempt(&error);
        Self { error, details }
    }
}
