//! Controlled native/flat save under already-authorized desktop file authority.
//! Protected directory bundles need their own guarded writer. Only ordinary
//! exclusion-free directories retain the upstream legacy save behavior.
use super::{AutomationError, Document, ExportOptions, Path, poll};
use photocraft_engine::WriteProtection;
use photocraft_raster::Interrupt;
fn io_error(error: photocraft_io::IoError) -> AutomationError {
    if matches!(error, photocraft_io::IoError::Cancelled) {
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
    } else {
        AutomationError::Import(error)
    }
}
fn denied() -> AutomationError {
    AutomationError::BadRequest("native save snapshot exceeds standard preparation limits".into())
}

fn admit_snapshot(document: &Document, control: &Interrupt<'_>) -> Result<photocraft_io::NativeSaveLimits, AutomationError> {
    let limits = photocraft_io::NativeSaveLimits::default();
    let raster = photocraft_engine::DocumentSnapshot::resident_raster_footprint_of(document, control).map_err(photocraft_engine::EngineError::from)?;
    let input = photocraft_io::retained_native_input_bytes(document, limits.preparation, control).map_err(io_error)? as u64;
    let model = photocraft_io::retained_native_model_bytes(document, limits.preparation, control).map_err(io_error)? as u64;
    let ceiling = photocraft_raster::MAX_CHECKED_RASTER_BYTES.min(limits.preparation.resident_bytes as u64);
    let named_bytes =
        input.max(raster.retained_bytes).checked_add(model).and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(2)?)).ok_or_else(denied)?;
    if named_bytes > ceiling {
        return Err(denied());
    }
    poll(control)?;
    Ok(limits)
}

/// Synchronous caller convenience; source/final guards are identical to the
/// controlled API, with the explicit noncancellable NONE token.
pub fn save_protected(
    document: &Document,
    path: &Path,
    format_override: Option<&str>,
    options: &ExportOptions,
    protection: &WriteProtection,
) -> Result<Vec<String>, AutomationError> {
    save_protected_with_interrupt(document, path, format_override, options, protection, &Interrupt::NONE)
}

/// Caller supplies runtime granted source paths and original document/all-open
/// fingerprints in `protection`. This does not read persisted resource locators
/// or change the color policy. Each final write retains those exclusions.
pub fn save_protected_with_interrupt(
    document: &Document,
    path: &Path,
    format_override: Option<&str>,
    options: &ExportOptions,
    protection: &WriteProtection,
    control: &Interrupt<'_>,
) -> Result<Vec<String>, AutomationError> {
    poll(control)?;
    super::validate_destination_protected_with_interrupt(path, protection, format_override, control)?;
    let directory = path.is_dir();
    poll(control)?;
    if directory {
        let original = photocraft_io::source_protection::document_fingerprints_with_interrupt(document, control).map_err(|error| {
            if matches!(error, photocraft_io::source_protection::SourceProtectionError::Cancelled) {
                AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
            } else {
                AutomationError::BadRequest(error.to_string())
            }
        })?;
        if !protection.sources.is_empty() || !protection.fingerprints.is_empty() || !original.is_empty() {
            let extension = format_override
                .map(|f| f.trim_start_matches('.').to_ascii_lowercase())
                .or_else(|| path.extension().map(|f| f.to_string_lossy().to_ascii_lowercase()));
            if extension.as_deref() != Some(photocraft_format::EXTENSION) {
                return Err(AutomationError::BadRequest("directory bundles require the native .pcraft format".into()));
            }
            let mut known = std::collections::HashSet::new();
            known.try_reserve(protection.fingerprints.len()).map_err(|_| denied())?;
            for fingerprint in &protection.fingerprints {
                poll(control)?;
                known.insert(fingerprint.as_str());
            }
            for fingerprint in &original {
                poll(control)?;
                if !known.contains(fingerprint.as_str()) {
                    return Err(AutomationError::BadRequest("native directory caller must retain document source fingerprints".into()));
                }
            }
            drop(known);
            drop(original);
            #[cfg(not(target_arch = "wasm32"))]
            {
                admit_snapshot(document, control)?;
                let mut named = document.clone();
                if let Some(filename) = path.file_name() {
                    let filename = filename.to_string_lossy();
                    let mut name = String::new();
                    name.try_reserve_exact(filename.len()).map_err(|_| denied())?;
                    name.push_str(&filename);
                    named.name = name;
                }
                poll(control)?;
                let prepared = photocraft_io::prepare_native_stream(&named, Default::default(), control).map_err(io_error)?;
                let mut sink = super::protected_directory::Sink::new(path, protection, control)?;
                return Ok(prepared.write_directory(&mut sink).map_err(io_error)?.warnings);
            }
            #[cfg(target_arch = "wasm32")]
            return Err(AutomationError::BadRequest("protected native directory saves are unavailable in the browser".into()));
        }
        admit_snapshot(document, control)?;
        poll(control)?;
        // save_dir has no token/final-guard callback. Only exclusion-free legacy
        // directories enter it; once begun it cannot respond to cancellation.
        // Preserve a successful directory commit rather than report cancellation
        // after publication. No protected/race-safe directory claim is made.
        return super::save(document, path, format_override, options, None);
    }
    let extension = format_override
        .map(|format| format.trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| path.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()))
        .ok_or_else(|| AutomationError::BadRequest("choose an output format or filename extension".into()))?;
    if extension == photocraft_format::EXTENSION {
        // Preserve the old save helper's output-title behavior without allowing
        // an unchecked large private model clone. This is known model/payload/map
        // admission, not a complete allocator/RSS measurement.
        let limits = admit_snapshot(document, control)?;
        let filename = path.file_name().ok_or_else(|| AutomationError::BadRequest("native destination has no filename".into()))?.to_string_lossy();
        let mut name = String::new();
        name.try_reserve_exact(filename.len()).map_err(|_| denied())?;
        name.push_str(&filename);
        poll(control)?;
        let mut named = document.clone();
        named.name = name;
        poll(control)?;
        let prepared = photocraft_io::prepare_native_stream(&named, limits, control).map_err(io_error)?;
        let result = super::write_protected_stream_with_interrupt(path, protection, control, move |file| {
            let (_, result) = prepared.write(file).map_err(io_error)?;
            Ok(result)
        })?;
        Ok(result.warnings)
    } else {
        let encoded = photocraft_io::export_with(document, &extension, options, control).map_err(io_error)?;
        super::write_protected_native_with_interrupt(path, &encoded.bytes, protection, control)?;
        Ok(encoded.warnings)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
// Test assertions are permitted by repository policy.
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn root() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path =
            std::env::temp_dir().join(format!("photocraft-protected-save-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        path
    }
    fn document() -> Document {
        Document::new("Old title", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8)
    }
    #[test]
    fn runtime_source_alias_is_refused_without_source_or_destination_changes() {
        let root = root();
        let source = root.join("owned-source.bin");
        let alias = root.join("output.png");
        std::fs::write(&source, b"owned source bytes").unwrap();
        std::fs::hard_link(&source, &alias).unwrap();
        let protection = WriteProtection { sources: vec![source.to_string_lossy().into_owned()], fingerprints: vec![] };
        let before = std::fs::read(&source).unwrap();
        assert!(save_protected_with_interrupt(&document(), &alias, None, &Default::default(), &protection, &Interrupt::NONE).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), before);
        assert_eq!(std::fs::read(&alias).unwrap(), before);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_title_and_original_model_preserved_with_protected_directory_reuse() {
        let root = root();
        let path = root.join("Result.pcraft");
        let doc = document();
        let before = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        save_protected_with_interrupt(&doc, &path, None, &Default::default(), &WriteProtection::default(), &Interrupt::NONE).unwrap();
        let saved = photocraft_format::load_path(&path).unwrap();
        assert_eq!(saved.name, "Result.pcraft");
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
        let directory = root.join("bundle.pcraft");
        std::fs::create_dir(&directory).unwrap();
        let protection = WriteProtection { sources: Vec::new(), fingerprints: vec!["11".repeat(32)] };
        for _ in 0..2 {
            save_protected_with_interrupt(&doc, &directory, None, &Default::default(), &protection, &Interrupt::NONE).unwrap();
            assert_eq!(photocraft_format::load_path(&directory).unwrap().name, "bundle.pcraft");
        }
        save_protected(&doc, &directory, None, &Default::default(), &WriteProtection::default()).unwrap();
        let reopened = photocraft_format::load_path(&directory).unwrap();
        assert_eq!(reopened.name, "bundle.pcraft");
        assert_eq!(photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn protected_directory_manifest_alias_preserves_original_source() {
        let root = root();
        let directory = root.join("Alias.pcraft");
        std::fs::create_dir(&directory).unwrap();
        let source = root.join("owned-original.xcf");
        let bytes = b"gimp xcf v001\0own synthetic protected source";
        std::fs::write(&source, bytes).unwrap();
        std::fs::hard_link(&source, directory.join("manifest.json")).unwrap();
        let protection = WriteProtection { sources: vec![source.to_string_lossy().into_owned()], fingerprints: vec![] };
        assert!(save_protected(&document(), &directory, None, &Default::default(), &protection).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(std::fs::read(directory.join("manifest.json")).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }
}
