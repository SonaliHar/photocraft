//! Runtime source provenance, separate from editable pixels and writable save destinations.
use photocraft_io::ImportOrigin;

/// Inert destination exclusions; these values confer no file-read authority.
#[derive(Debug, Clone, Default)]
pub struct WriteProtection {
    pub sources: Vec<String>,
    pub fingerprints: Vec<String>,
}
impl WriteProtection {
    /// Known retained exclusion metadata for a shared save envelope. Paths remain
    /// inert strings until an already-authorized destination guard consumes them.
    pub fn resident_bytes_with_interrupt(&self, control: &photocraft_raster::Interrupt<'_>) -> crate::Result<usize> {
        let poll = || control.check().map_err(|_| crate::EngineError::Cancelled);
        let denied = || crate::EngineError::Other("source-protection.resident-limit: exclusion metadata".into());
        poll()?;
        if self.sources.len() > 4096 { return Err(denied()); }
        let hashes = photocraft_io::source_protection::fingerprint_resident_bytes(&self.fingerprints, self.fingerprints.capacity(), control)
            .map_err(|error| if control.cancelled() { crate::EngineError::Cancelled } else { crate::EngineError::Other(error.to_string()) })?;
        let mut bytes = self.sources.capacity().checked_mul(std::mem::size_of::<String>())
            .and_then(|n| n.checked_add(hashes)).and_then(|n| n.checked_add(std::mem::size_of::<Self>())).ok_or_else(denied)?;
        for path in &self.sources {
            poll()?;
            if path.len() > 4096 { return Err(denied()); }
            bytes = bytes.checked_add(path.capacity()).ok_or_else(denied)?;
        }
        poll()?;
        Ok(bytes)
    }
    pub fn add_document(&mut self, document: &photocraft_doc::Document, source: Option<&SourceIdentity>) -> Result<(), String> {
        self.add_document_with_interrupt(document, source, &photocraft_raster::Interrupt::NONE).map_err(|error| error.to_string())
    }

    /// Collect inert exclusions with the caller's token before publishing either list.
    pub fn add_document_with_interrupt(&mut self, document: &photocraft_doc::Document, source: Option<&SourceIdentity>, control: &photocraft_raster::Interrupt<'_>) -> crate::Result<()> {
        let poll = || control.check().map_err(|_| crate::EngineError::Cancelled);
        let denied = |message: &str| crate::EngineError::Other(message.into());
        poll()?;
        let fingerprints = photocraft_io::source_protection::document_fingerprints_with_interrupt(document, control)
            .map_err(|error| if matches!(error, photocraft_io::source_protection::SourceProtectionError::Cancelled) {
                crate::EngineError::Cancelled
            } else { crate::EngineError::Other(error.to_string()) })?;
        let path = source.filter(|s| s.origin.is_read_only()).and_then(|s| s.path.as_ref());
        if path.is_some_and(|p| p.len() > 4096)
            || self.sources.len().saturating_add(usize::from(path.is_some())) > 4096
            || self.fingerprints.len().saturating_add(fingerprints.len()) > photocraft_io::source_protection::MAX_FINGERPRINTS
        {
            return Err(denied("xcf.source-protection.limit: too many protected source identities"));
        }
        self.sources.try_reserve(usize::from(path.is_some())).map_err(|_| denied("xcf.source-protection.limit: source allocation failed"))?;
        self.fingerprints.try_reserve(fingerprints.len()).map_err(|_| denied("xcf.source-protection.limit: fingerprint allocation failed"))?;
        poll()?;
        let owned = if let Some(path) = path {
            let mut owned = String::new();
            owned.try_reserve_exact(path.len()).map_err(|_| denied("xcf.source-protection.limit: path allocation failed"))?;
            owned.push_str(path);
            Some(owned)
        } else { None };
        poll()?;
        if let Some(owned) = owned { self.sources.push(owned); }
        self.fingerprints.extend(fingerprints);
        Ok(())
    }
}

/// A source label and a path supplied by an authorized open service, never persisted authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    pub label: String,
    pub path: Option<String>,
    pub origin: ImportOrigin,
}
impl SourceIdentity {
    pub fn new(label: impl Into<String>, path: Option<String>, origin: ImportOrigin) -> Self {
        Self { label: label.into(), path, origin }
    }
    /// Pre-encoding checks without filesystem access. Services additionally check aliases
    /// and destination content within their existing filesystem authority.
    pub fn check_destination(&self, path: &str) -> Result<(), String> {
        if photocraft_io::is_xcf_filename(path) {
            return Err("XCF is an import-only format; choose a writable destination such as .pcraft".into());
        }
        if self.origin.is_read_only() && self.path.as_ref().is_some_and(|source| same_name(source, path)) {
            return Err("the imported XCF source is protected; choose a different writable destination".into());
        }
        Ok(())
    }
}
fn same_name(a: &str, b: &str) -> bool {
    let a = a.replace('\\', "/");
    let b = b.replace('\\', "/");
    if cfg!(windows) { a.eq_ignore_ascii_case(&b) } else { a == b }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusion_residency_keeps_actual_string_capacity_and_no_authority() {
        let mut path = String::with_capacity(8192);
        path.push_str("granted/source.xcf");
        let mut hash = String::with_capacity(256);
        hash.push_str(&blake3::hash(b"own source").to_hex().to_string());
        let protection = WriteProtection { sources: vec![path], fingerprints: vec![hash] };
        let before = protection.sources.clone();
        let resident = protection.resident_bytes_with_interrupt(&photocraft_raster::Interrupt::NONE).unwrap();
        assert!(resident >= 8192 + 256);
        assert_eq!(protection.sources, before);
        assert!(matches!(protection.resident_bytes_with_interrupt(&photocraft_raster::Interrupt::cancel_only(&|| true)), Err(crate::EngineError::Cancelled)));
        let mut altered = protection;
        altered.fingerprints[0] = "invalid".into();
        assert!(altered.resident_bytes_with_interrupt(&photocraft_raster::Interrupt::NONE).is_err());
    }
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Size};
    #[test]
    fn renamed_xcf_source_is_retained_after_save_as() {
        let doc = Document::new("source.psd", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let mut state = crate::DocState::new(doc, Some("source.psd".into())).unwrap();
        state.set_import_origin(ImportOrigin::xcf(), "source.psd", Some("source.psd".into()));
        assert!(state.path.is_none());
        state.path = Some("edited.pcraft".into());
        let source = state.source.as_ref().expect("runtime source retained");
        assert!(source.check_destination("source.psd").is_err());
        assert!(source.check_destination("edited.pcraft").is_ok());
        assert!(source.check_destination("other.XCF").is_err());
    }
    #[test]
    fn writable_import_keeps_path_and_byte_only_import_grants_none() {
        let doc = Document::new("input.psd", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let mut state = crate::DocState::new(doc, None).unwrap();
        state.set_import_origin(ImportOrigin::writable(photocraft_io::ImportFormat::Psd), "input.psd", Some("input.psd".into()));
        assert_eq!(state.path.as_deref(), Some("input.psd"));
        state.set_import_origin(ImportOrigin::xcf(), "bytes", None);
        assert!(state.path.is_none());
        assert!(state.source.as_ref().expect("source").path.is_none());
    }
}
