use photocraft_io::xcf_assessment::{DiagnosticCode, FailureReport};
use photocraft_xcf::{ErrorKind, XcfError};

#[test]
fn future_version_returns_details_without_a_document_or_raw_payload() {
    let error = photocraft_io::xcf_import::import_with_diagnostics("misleading.psd", b"gimp xcf v099\0", &photocraft_raster::Interrupt::NONE).unwrap_err();
    assert_eq!(error.details.diagnostics[0].code, DiagnosticCode::UnsupportedXcfVersion);
    let json = serde_json::to_value(&error.details).unwrap();
    assert_eq!(json["sourceFormat"], "xcf");
    assert_eq!(json["outcome"], "failure");
    assert!(json.get("document").is_none());
    assert!(json.get("fidelity").is_none());
    assert!(!json.to_string().contains("misleading.psd"));
    assert!(matches!(error.error, photocraft_io::IoError::Xcf(ref e) if e.kind == ErrorKind::Version));
}

#[test]
fn typed_failures_keep_lossless_offsets_and_do_not_echo_embedded_strings() {
    for (kind, expected) in [
        (ErrorKind::Header, DiagnosticCode::InvalidXcf),
        (ErrorKind::Structure, DiagnosticCode::InvalidStructure),
        (ErrorKind::Truncated, DiagnosticCode::TruncatedXcf),
        (ErrorKind::Compression, DiagnosticCode::InvalidContainer),
        (ErrorKind::Unsupported, DiagnosticCode::UnsupportedFeature),
        (ErrorKind::Limit, DiagnosticCode::ResourceLimit),
        (ErrorKind::Allocation, DiagnosticCode::ResourceLimit),
        (ErrorKind::Cancelled, DiagnosticCode::Cancelled),
    ] {
        let error =
            photocraft_io::IoError::Xcf(XcfError { kind, offset: u64::MAX, item: "private embedded locator".into(), message: "secret source payload".into() });
        let details = FailureReport::from_xcf_attempt(&error);
        assert_eq!(details.diagnostics[0].code, expected);
        let json = serde_json::to_value(&details).unwrap();
        assert_eq!(json["diagnostics"][0]["sourceOffset"], u64::MAX.to_string());
        assert!(!json.to_string().contains("private embedded locator"));
        assert!(!json.to_string().contains("secret source payload"));
    }
}

#[test]
fn cancelled_attempt_keeps_legacy_error_and_has_no_invented_recovery() {
    let cancelled = || true;
    let control = photocraft_raster::Interrupt::cancel_only(&cancelled);
    let failure = photocraft_io::xcf_import::import_with_diagnostics("cancelled.xcf", b"gimp xcf v012\0", &control).unwrap_err();
    assert_eq!(failure.details.diagnostics[0].code, DiagnosticCode::Cancelled);
    assert!(failure.details.diagnostics[0].recovery_actions.is_empty());
    assert!(matches!(failure.error, photocraft_io::IoError::Cancelled));
}

#[test]
fn accounted_preparation_failures_keep_typed_codes_without_source_offsets_or_payloads() {
    use photocraft_format::FormatError;
    use photocraft_io::{IoError, xcf_assessment::ImportFailure};
    for (native_error, expected) in [
        (FormatError::Cancelled, DiagnosticCode::Cancelled),
        (FormatError::LimitExceeded("secret embedded source path".into()), DiagnosticCode::ResourceLimit),
        (FormatError::Unsupported("secret embedded source path".into()), DiagnosticCode::UnsupportedFeature),
        (FormatError::Corrupt("secret embedded source path".into()), DiagnosticCode::ImportFailed),
    ] {
        let failure = ImportFailure::from(IoError::Pcraft(native_error));
        let diagnostic = &failure.details.diagnostics[0];
        assert_eq!(diagnostic.code, expected);
        assert_eq!(diagnostic.source_offset, None);
        assert!(diagnostic.recovery_actions.is_empty());
        let encoded = serde_json::to_string(&failure.details).unwrap();
        assert!(!encoded.contains("secret embedded source path"));
        assert!(matches!(failure.error, IoError::Pcraft(_)));
    }
}
