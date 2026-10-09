//! Runtime source provenance, separate from editable pixels and writable save destinations.
use photocraft_io::ImportOrigin;

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
        if crate::file_cmds::extension(path).as_deref() == Some("xcf") {
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
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Size};
    #[test]
    fn renamed_xcf_source_is_retained_after_save_as() {
        let doc = Document::new("source.psd", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let mut state = crate::DocState::new(doc, Some("source.psd".into()));
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
        let mut state = crate::DocState::new(doc, None);
        state.set_import_origin(ImportOrigin::writable(photocraft_io::ImportFormat::Psd), "input.psd", Some("input.psd".into()));
        assert_eq!(state.path.as_deref(), Some("input.psd"));
        state.set_import_origin(ImportOrigin::xcf(), "bytes", None);
        assert!(state.path.is_none());
        assert!(state.source.as_ref().expect("source").path.is_none());
    }
}
