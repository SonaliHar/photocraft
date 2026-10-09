//! Runtime import provenance. Filename suffixes never determine source write authority.

/// Actual format identified from the input content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportFormat {
    #[default]
    Unknown,
    Pcraft,
    Psd,
    Psb,
    Xcf,
    CameraRaw,
    Flat(photocraft_codecs::Format),
}

/// Whether opening this format can establish a writable source destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourcePolicy {
    #[default]
    Writable,
    ReadOnlySource,
}

/// Format and policy carried with a successful import, outside persisted document data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOrigin {
    pub format: ImportFormat,
    pub source_policy: SourcePolicy,
}

impl ImportOrigin {
    pub const fn writable(format: ImportFormat) -> Self {
        Self { format, source_policy: SourcePolicy::Writable }
    }

    pub const fn xcf() -> Self {
        Self { format: ImportFormat::Xcf, source_policy: SourcePolicy::ReadOnlySource }
    }

    pub const fn is_read_only(self) -> bool {
        matches!(self.source_policy, SourcePolicy::ReadOnlySource)
    }

    /// Identification happens before any suffix-specific loader shortcut.
    pub fn detected(bytes: &[u8]) -> Self {
        if photocraft_xcf::is_xcf(bytes) {
            Self::xcf()
        } else if photocraft_format::is_pcraft(bytes) {
            Self::writable(ImportFormat::Pcraft)
        } else if bytes.starts_with(b"8BPS") {
            Self::writable(if bytes.get(4..6) == Some(&[0, 2]) { ImportFormat::Psb } else { ImportFormat::Psd })
        } else if crate::raw::is_raw(bytes) {
            Self::writable(ImportFormat::CameraRaw)
        } else {
            Self::writable(photocraft_codecs::detect(bytes).map_or(ImportFormat::Unknown, ImportFormat::Flat))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xcf_is_a_protected_source_independent_of_its_filename() {
        assert_eq!(ImportOrigin::detected(b"gimp xcf v013\0"), ImportOrigin::xcf());
        assert!(ImportOrigin::xcf().is_read_only());
        assert!(!ImportOrigin::writable(ImportFormat::Pcraft).is_read_only());
    }

    #[test]
    fn short_headers_are_safe_and_psb_retains_actual_format() {
        assert_eq!(ImportOrigin::detected(b"8"), ImportOrigin::default());
        assert_eq!(ImportOrigin::detected(b"8BPS\0\x02").format, ImportFormat::Psb);
    }
}
