//! File I/O shared by the MCP server and the CLI: open/save any supported
//! format, with `.pcraft` (ZIP or directory bundle) handled natively.

use std::path::Path;

use photocraft_doc::Document;
use photocraft_format::{PcraftWriter, SaveOptions};
use photocraft_io::ExportOptions;

use crate::AutomationError;
#[path = "files/owned_native_open.rs"]
mod owned_native_open;
pub use owned_native_open::open_accounted_native_selected_path;
#[path = "files/owned_xcf_open.rs"]
pub(crate) mod owned_xcf_open;
pub use owned_xcf_open::open_owned_plain_xcf_selected_path;

#[path = "files/protected_save.rs"]
mod protected_save;
#[cfg(not(target_arch = "wasm32"))]
#[path = "files/protected_directory.rs"]
mod protected_directory;
pub use protected_save::{save_protected, save_protected_with_interrupt};

/// Result of opening a file.
pub struct Opened {
    pub document: Document,
    pub warnings: Vec<String>,
    pub origin: photocraft_io::ImportOrigin,
    pub assessment: Option<photocraft_io::xcf_assessment::ImportAssessment>,
    pub source_read_only: bool,
    pub preview_only: bool,
}

/// Decode a document that was read through a filesystem capability.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Opened, AutomationError> {
    open_bytes_with_interrupt(name, bytes, &photocraft_raster::Interrupt::NONE)
}
/// Controlled byte fallback shared with capability-scoped opened handles.
pub fn open_bytes_with_interrupt(name: &str, bytes: &[u8], control: &photocraft_raster::Interrupt<'_>) -> Result<Opened, AutomationError> {
    poll(control)?;
    if photocraft_format::is_pcraft(bytes) {
        return Ok(Opened {
            document: photocraft_format::load_from_bytes_with_interrupt(bytes, &Default::default(), control).map_err(|error| {
                if matches!(error, photocraft_format::FormatError::Cancelled) {
                    AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
                } else {
                    AutomationError::Format(error)
                }
            })?,
            warnings: Vec::new(),
            origin: photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft),
            assessment: None,
            source_read_only: false,
            preview_only: false,
        });
    }
    let r = photocraft_io::import_with(name, bytes, control).map_err(|error| {
        if matches!(error, photocraft_io::IoError::Cancelled) {
            return AutomationError::Engine(photocraft_engine::EngineError::Cancelled);
        }
        let known_xcf = photocraft_io::ImportOrigin::detected(bytes).format == photocraft_io::ImportFormat::Xcf
            || matches!(&error, photocraft_io::IoError::Xcf(_))
            || matches!(&error, photocraft_io::IoError::Unsupported(message) if message.starts_with("XCF "));
        if known_xcf { AutomationError::XcfImport(Box::new(error.into())) } else { AutomationError::Import(error) }
    })?;
    Ok(Opened {
        document: r.document,
        warnings: r.warnings,
        origin: r.origin,
        assessment: r.assessment,
        source_read_only: r.source_read_only,
        preview_only: r.preview_only,
    })
}

/// Open a document from disk. Directory bundles and `.pcraft` ZIPs load
/// natively; everything else goes through `photocraft-io` (PSD, PNG, …).
pub fn open(path: &Path) -> Result<Opened, AutomationError> {
    open_with_interrupt(path, &photocraft_raster::Interrupt::NONE)
}
/// Native files use a single held stream and standard load limits. Other content
/// retains the authorized-size byte fallback on that same handle; directory
/// bundles propagate cancellation through their bounded object loader.
pub fn open_with_interrupt(path: &Path, control: &photocraft_raster::Interrupt<'_>) -> Result<Opened, AutomationError> {
    use std::io::{Seek, SeekFrom};
    poll(control)?;
    let directory = path.is_dir();
    poll(control)?;
    if directory {
        let document = photocraft_format::load_path_with_interrupt(path, &Default::default(), control).map_err(|error| {
            if matches!(error, photocraft_format::FormatError::Cancelled) {
                AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
            } else {
                AutomationError::Format(error)
            }
        })?;
        poll(control)?;
        return Ok(Opened {
            document,
            warnings: Vec::new(),
            origin: photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft),
            assessment: None,
            source_read_only: false,
            preview_only: false,
        });
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut file = std::fs::File::open(path).map_err(|error| AutomationError::Io(format!("{}: {error}", path.display())))?;
    poll(control)?;
    let metadata = file.metadata().map_err(|error| AutomationError::Io(format!("{}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(AutomationError::BadRequest("document path must be a regular file".into()));
    }
    if let Some(result) = photocraft_io::import_native_stream_candidate(&name, &mut file, control).map_err(|error| {
        if matches!(error, photocraft_io::IoError::Cancelled) {
            AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
        } else {
            AutomationError::Import(error)
        }
    })? {
        poll(control)?;
        return Ok(Opened {
            document: result.document,
            warnings: result.warnings,
            origin: result.origin,
            assessment: result.assessment,
            source_read_only: result.source_read_only,
            preview_only: result.preview_only,
        });
    }
    poll(control)?;
    file.seek(SeekFrom::Start(0)).map_err(|error| AutomationError::Io(format!("{}: {error}", path.display())))?;
    poll(control)?;
    let bytes = photocraft_format::read::read_all_with_interrupt(&mut file, metadata.len(), control).map_err(|error| {
        if control.cancelled() {
            AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
        } else {
            AutomationError::Io(format!("{}: {error}", path.display()))
        }
    })?;
    open_bytes_with_interrupt(&name, &bytes, control)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod controlled_native_open_tests {
    use super::*;
    fn path(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!("photocraft-stream-open-{}-{}-{label}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)))
    }
    #[test]
    fn native_central_manifest_content_beats_misnamed_png_suffix() {
        let document = Document::with_background(
            "Own native open",
            photocraft_doc::Size::new(2, 1),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U16,
            photocraft_doc::Color::WHITE,
        );
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let archive = photocraft_format::zip::ZipReader::new(&bytes).unwrap();
        let mut writer = photocraft_format::zip::ZipWriter::new();
        writer.add("owned-first-extra", b"own").unwrap();
        for entry in &archive.entries {
            writer.add(&entry.name, &archive.read(entry, 1 << 20).unwrap()).unwrap();
        }
        let path = path("misnamed.png");
        std::fs::write(&path, writer.finish().unwrap()).unwrap();
        let opened = open_with_interrupt(&path, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(opened.document, document);
        assert_eq!(opened.origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert!(opened.warnings.is_empty());
        assert!(opened.assessment.is_none());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn byte_fallback_preserves_flat_content_and_named_xcf_failures() {
        let image =
            photocraft_codecs::Image::from_raw(1, 1, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, vec![1, 2, 3, 255]).unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let path = path("own.png");
        std::fs::write(&path, &bytes).unwrap();
        let opened = open_with_interrupt(&path, &photocraft_raster::Interrupt::NONE).unwrap();
        let byte_open = open_bytes(path.file_name().unwrap().to_str().unwrap(), &bytes).unwrap();
        assert_eq!(opened.document.name, byte_open.document.name);
        assert_eq!(opened.origin, byte_open.origin);
        assert_eq!(opened.document.layers.first().unwrap().surface(), byte_open.document.layers.first().unwrap().surface());
        std::fs::remove_file(path).unwrap();
        let path = self::path("misnamed.pcraft");
        std::fs::write(&path, b"gimp xcf v013\0").unwrap();
        assert!(matches!(open_with_interrupt(&path, &photocraft_raster::Interrupt::NONE), Err(AutomationError::XcfImport(_))));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn early_cancel_and_truncated_native_return_errors_without_loading() {
        let path = path("own.pcraft");
        std::fs::write(&path, b"PK\x03\x04").unwrap();
        assert!(matches!(
            open_with_interrupt(&path, &photocraft_raster::Interrupt::cancel_only(&|| true)),
            Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
        ));
        assert!(open_with_interrupt(&path, &photocraft_raster::Interrupt::NONE).is_err());
        std::fs::remove_file(path).unwrap();
    }
}

/// Previews for a `.pcraft` bundle.
pub fn previews(doc: &Document) -> Result<SaveOptions, AutomationError> {
    Ok(SaveOptions {
        thumbnail: Some(photocraft_compose::thumbnail(doc, 256).map_err(photocraft_io::IoError::from)?),
        composite: Some(photocraft_compose::thumbnail(doc, 1024).map_err(photocraft_io::IoError::from)?),
    })
}

/// Encode the name the saved native bundle will report on reopening. Save As updates
/// the session identity only *after* a successful write; without a separate snapshot
/// the manifest serializes the previous name instead.
fn named_for_output(doc: &Document, path: &str) -> Document {
    let mut named = doc.clone();
    if let Some(name) = path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()) {
        named.name = name.to_owned();
    }
    named
}

/// Save or export `doc` to `path`, choosing the format from the extension
/// (or `format_override`, an extension such as `"png"`). `writer` makes
/// repeated `.pcraft` saves incremental. Returns warnings about lost data.
pub fn save(
    doc: &Document,
    path: &Path,
    format_override: Option<&str>,
    opts: &ExportOptions,
    writer: Option<&mut PcraftWriter>,
) -> Result<Vec<String>, AutomationError> {
    validate_destination(path, None, format_override)?;
    let ext = format_override
        .map(|f| f.trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()))
        .ok_or_else(|| AutomationError::BadRequest(format!("cannot tell the format of `{}`; pass a format", path.display())))?;
    if ext == photocraft_format::EXTENSION {
        let named = named_for_output(doc, &path.to_string_lossy());
        let mut local = PcraftWriter::new();
        let w = writer.unwrap_or(&mut local);
        w.save_path(&named, path, &previews(&named)?)?;
        return Ok(Vec::new());
    }
    let r = photocraft_io::export(doc, &ext, opts)?;
    // Crash-safe: a failed write never destroys the previous file.
    photocraft_format::atomic_write(path, &r.bytes).map_err(|e| AutomationError::Io(e.to_string()))?;
    Ok(r.warnings)
}

/// Encode a document for a capability-scoped write. Unlike [`save`], this
/// never obtains ambient filesystem authority and emits `.pcraft` as a ZIP.
pub fn save_bytes(doc: &Document, name: &str, format_override: Option<&str>, opts: &ExportOptions) -> Result<(Vec<u8>, Vec<String>), AutomationError> {
    validate_write_name(Path::new(name), format_override)?;
    let ext = format_override
        .map(|format| format.trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| Path::new(name).extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()))
        .ok_or_else(|| AutomationError::BadRequest(format!("cannot tell the format of `{name}`; pass a format")))?;
    if ext == photocraft_format::EXTENSION {
        let named = named_for_output(doc, name);
        return Ok((photocraft_format::save_to_bytes(&named, &previews(&named)?)?, Vec::new()));
    }
    let result = photocraft_io::export(doc, &ext, opts)?;
    Ok((result.bytes, result.warnings))
}

/// Reject import-only names independently of any format override, before encoding.
pub fn validate_write_name(path: &Path, format_override: Option<&str>) -> Result<(), AutomationError> {
    if photocraft_io::is_xcf_filename(&path.to_string_lossy()) || format_override.is_some_and(|ext| photocraft_io::is_xcf_filename(ext.trim_start_matches('.')))
    {
        return Err(AutomationError::BadRequest("XCF is import-only; choose a writable format and destination".into()));
    }
    Ok(())
}

/// Native callers already holding ambient file authority validate the destination before
/// encoding or creating a temporary output. Capability callers use `guard_write` instead.
pub fn validate_destination(path: &Path, source: Option<&photocraft_engine::SourceIdentity>, format_override: Option<&str>) -> Result<(), AutomationError> {
    let mut protection = photocraft_engine::WriteProtection::default();
    if let Some(source) = source.filter(|source| source.origin.is_read_only()).and_then(|source| source.path.as_deref()) {
        if source.is_empty() || source.len() > 4096 {
            return Err(AutomationError::BadRequest("protected native source path is empty or too long".into()));
        }
        let mut owned = String::new();
        owned.try_reserve_exact(source.len()).map_err(|_| AutomationError::BadRequest("protected source allocation denied".into()))?;
        owned.push_str(source);
        protection.sources.try_reserve_exact(1).map_err(|_| AutomationError::BadRequest("protected source allocation denied".into()))?;
        protection.sources.push(owned);
    }
    validate_destination_protected(path, &protection, format_override)
}

fn protection_error(error: photocraft_io::source_protection::SourceProtectionError) -> AutomationError {
    if matches!(error, photocraft_io::source_protection::SourceProtectionError::Cancelled) {
        return AutomationError::Engine(photocraft_engine::EngineError::Cancelled);
    }
    AutomationError::BadRequest(format!("native source protection: {error}"))
}

fn poll(control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
    control.check().map_err(|_| AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
}
fn validate_protection(protection: &photocraft_engine::WriteProtection, control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
    poll(control)?;
    if protection.sources.len() > 4096 {
        return Err(AutomationError::BadRequest("native protected source identities exceed limits".into()));
    }
    for source in &protection.sources {
        poll(control)?;
        if source.is_empty() || source.len() > 4096 {
            return Err(AutomationError::BadRequest("native protected source identities exceed limits".into()));
        }
    }
    photocraft_io::source_protection::validate_fingerprints_with_interrupt(&protection.fingerprints, control).map_err(protection_error)
}

fn reject_named_source(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), AutomationError> {
    poll(control)?;
    for source in &protection.sources {
        poll(control)?;
        let source = Path::new(source);
        if source == path || matches!((std::fs::canonicalize(source), std::fs::canonicalize(path)), (Ok(a), Ok(b)) if a == b) {
            return Err(AutomationError::BadRequest("the original XCF source is protected; choose a different destination".into()));
        }
    }
    poll(control)?;
    Ok(())
}

fn check_native_file_with_limit(
    file: &mut std::fs::File,
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    control: &photocraft_raster::Interrupt<'_>,
    resident_limit: Option<u64>,
) -> Result<(), AutomationError> {
    poll(control)?;
    let metadata = file.metadata().map_err(|error| AutomationError::Io(format!("inspect destination {}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(AutomationError::BadRequest("native byte destination is not a regular file".into()));
    }
    #[cfg(not(target_arch = "wasm32"))]
    for source in &protection.sources {
        poll(control)?;
        // These paths were captured by the authorized open service. Inert native
        // fingerprints and original locators never populate this list.
        let original = same_file::Handle::from_path(source).map_err(|error| AutomationError::Io(format!("cannot identify granted source: {error}")))?;
        let destination =
            file.try_clone().and_then(same_file::Handle::from_file).map_err(|error| AutomationError::Io(format!("cannot identify destination: {error}")))?;
        if original == destination {
            return Err(AutomationError::BadRequest("an alias of the original XCF source is protected; choose a different destination".into()));
        }
    }
    if let Some(limit) = resident_limit {
        photocraft_io::source_protection::check_reader_with_resident_limit(file, metadata.len(), &protection.fingerprints, limit, control).map_err(protection_error)?;
    } else {
        photocraft_io::source_protection::check_reader_with_interrupt(file, metadata.len(), &protection.fingerprints, control).map_err(protection_error)?;
    }
    if file.metadata().map_err(|error| AutomationError::Io(error.to_string()))?.len() != metadata.len() {
        return Err(AutomationError::BadRequest("native destination changed during protection check".into()));
    }
    poll(control)?;
    Ok(())
}

/// Validate only through caller-owned native filesystem authority. Persisted
/// fingerprints exclude destinations; they do not authorize opening locators.
pub fn validate_destination_protected(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    format_override: Option<&str>,
) -> Result<(), AutomationError> {
    validate_destination_protected_with_interrupt(path, protection, format_override, &photocraft_raster::Interrupt::NONE)
}

/// Cooperative destination protection under previously granted native authority.
/// A blocked filesystem syscall itself cannot be interrupted.
pub fn validate_destination_protected_with_interrupt(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    format_override: Option<&str>,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), AutomationError> {
    validate_destination_protected_with_limit(path, protection, format_override, control, None)
}

/// Validate within a caller's remaining save envelope. The limit admits only
/// destination inspection; it supplies no new path or source authority.
pub fn validate_destination_protected_with_resident_limit(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    format_override: Option<&str>,
    control: &photocraft_raster::Interrupt<'_>,
    resident_limit: u64,
) -> Result<(), AutomationError> {
    poll(control)?;
    if resident_limit > photocraft_io::source_protection::resident_ceiling() {
        return Err(AutomationError::BadRequest("source-protection.resident-limit may only be lowered".into()));
    }
    validate_destination_protected_with_limit(path, protection, format_override, control, Some(resident_limit))
}

fn validate_destination_protected_with_limit(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    format_override: Option<&str>,
    control: &photocraft_raster::Interrupt<'_>,
    resident_limit: Option<u64>,
) -> Result<(), AutomationError> {
    poll(control)?;
    validate_write_name(path, format_override)?;
    validate_protection(protection, control)?;
    reject_named_source(path, protection, control)?;
    // Preserve directory-bundle preflight compatibility. The byte writer below
    // intentionally accepts regular-file destinations only.
    if path.is_dir() {
        poll(control)?;
        return Ok(());
    }
    poll(control)?;
    let opened = std::fs::File::open(path);
    poll(control)?;
    let result = match opened {
        Ok(mut file) => check_native_file_with_limit(&mut file, path, protection, control, resident_limit),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AutomationError::Io(format!("cannot validate destination {}: {error}", path.display()))),
    };
    poll(control)?;
    result
}

#[cfg(not(target_arch = "wasm32"))]
fn native_target_path(path: &Path) -> Result<std::path::PathBuf, AutomationError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::canonicalize(path).map_err(|error| AutomationError::Io(format!("cannot resolve selected destination {}: {error}", path.display())))
        }
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(AutomationError::Io(format!("cannot inspect selected destination {}: {error}", path.display()))),
    }
}

/// Atomic byte replacement for an already authorized native/desktop save.
/// Protection is retained through each rename retry in the same held parent.
pub fn write_protected_native(path: &Path, bytes: &[u8], protection: &photocraft_engine::WriteProtection) -> Result<(), AutomationError> {
    write_protected_native_with_interrupt(path, bytes, protection, &photocraft_raster::Interrupt::NONE)
}
/// Cancellation is observed until the successful held-parent rename commit point.
/// Later cancellation never reports failure for an already published replacement.
pub fn write_protected_native_with_interrupt(
    path: &Path,
    bytes: &[u8],
    protection: &photocraft_engine::WriteProtection,
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), AutomationError> {
    write_protected_native_progress_with_interrupt(path, bytes, protection, &|_| {}, control)
}
/// Byte-write progress under previously granted native authority. Completion is
/// reported only after the same held-parent guarded rename publishes the result.
pub fn write_protected_native_progress(
    path: &Path,
    bytes: &[u8],
    protection: &photocraft_engine::WriteProtection,
    progress: &dyn Fn(f32),
) -> Result<(), AutomationError> {
    write_protected_native_progress_with_interrupt(path, bytes, protection, progress, &photocraft_raster::Interrupt::NONE)
}
/// Cooperative cancellation and progress share the original source/alias/fingerprint
/// guards. Blocking filesystem calls cannot themselves be interrupted.
pub fn write_protected_native_progress_with_interrupt(
    path: &Path,
    bytes: &[u8],
    protection: &photocraft_engine::WriteProtection,
    progress: &dyn Fn(f32),
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), AutomationError> {
    poll(control)?;
    progress(0.0);
    write_protected_stream_with_interrupt(path, protection, control, |file| {
        use std::io::Write;
        let mut written_bytes = 0usize;
        for chunk in bytes.chunks(64 << 10) {
            poll(control)?;
            let result = file.write_all(chunk);
            poll(control)?;
            result.map_err(|error| AutomationError::Io(format!("cannot write save temporary: {error}")))?;
            written_bytes = written_bytes.checked_add(chunk.len()).ok_or_else(|| AutomationError::Io("save progress byte count overflow".into()))?;
            if !bytes.is_empty() {
                progress(((written_bytes as f64 / bytes.len() as f64) * 0.99) as f32);
            }
            poll(control)?;
        }
        Ok(())
    })?;
    // Successful rename is the commit point; do not turn it into cancellation.
    progress(1.0);
    Ok(())
}
/// Prepared bounded native serializer; publication remains in this held-parent guard.
pub(crate) fn write_protected_stream_with_interrupt<R>(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    control: &photocraft_raster::Interrupt<'_>,
    write: impl FnOnce(&mut cap_std::fs::File) -> Result<R, AutomationError>,
) -> Result<R, AutomationError> {
    write_protected_stream_with_guard(path, protection, control, &|| Ok(()), None, None, write)
}
pub(crate) fn write_protected_stream_with_guard<R>(
    path: &Path,
    protection: &photocraft_engine::WriteProtection,
    control: &photocraft_raster::Interrupt<'_>,
    extra_check: &dyn Fn() -> Result<(), AutomationError>,
    held_parent: Option<&cap_std::fs::Dir>,
    source_resident_limit: Option<u64>,
    write: impl FnOnce(&mut cap_std::fs::File) -> Result<R, AutomationError>,
) -> Result<R, AutomationError> {
    poll(control)?;
    extra_check()?;
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, write, protection, extra_check, held_parent, source_resident_limit);
        Err(AutomationError::BadRequest("native protected writes are unavailable in the browser".into()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        validate_destination_protected_with_limit(path, protection, None, control, source_resident_limit)?;
        let target = native_target_path(path)?;
        validate_destination_protected_with_limit(&target, protection, None, control, source_resident_limit)?;
        let leaf = target.file_name().ok_or_else(|| AutomationError::BadRequest("native destination has no filename".into()))?;
        let parent_path = target.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        poll(control)?;
        let parent=if let Some(parent)=held_parent {
            parent.try_clone().map_err(|error|AutomationError::Io(format!("cannot retain native entry parent: {error}")))?
        } else {
            std::fs::create_dir_all(parent_path).map_err(|error| AutomationError::Io(format!("cannot create destination parent: {error}")))?;
            cap_std::fs::Dir::open_ambient_dir(parent_path, cap_std::ambient_authority())
                .map_err(|error| AutomationError::Io(format!("cannot hold destination parent: {error}")))?
        };
        let check = || -> Result<(), AutomationError> {
            poll(control)?;
            extra_check()?;
            reject_named_source(path, protection, control)?;
            reject_named_source(&target, protection, control)?;
            if native_target_path(path)? != target {
                return Err(AutomationError::BadRequest("selected native destination changed during save".into()));
            }
            let current_parent = cap_std::fs::Dir::open_ambient_dir(parent_path, cap_std::ambient_authority())
                .map_err(|error| AutomationError::Io(format!("cannot recheck destination parent: {error}")))?;
            let held = parent
                .try_clone()
                .and_then(|dir| same_file::Handle::from_file(dir.into_std_file()))
                .map_err(|error| AutomationError::Io(format!("cannot identify held destination parent: {error}")))?;
            let current = same_file::Handle::from_file(current_parent.into_std_file())
                .map_err(|error| AutomationError::Io(format!("cannot identify destination parent: {error}")))?;
            if held != current {
                return Err(AutomationError::BadRequest("native destination parent changed during save".into()));
            }
            poll(control)?;
            let opened = parent.open(leaf);
            poll(control)?;
            let result = match opened {
                Ok(file) => check_native_file_with_limit(&mut file.into_std(), &target, protection, control, source_resident_limit),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(AutomationError::Io(format!("cannot recheck native destination: {error}"))),
            };
            poll(control)?;
            result
        };
        check()?;
        let tmp = photocraft_format::atomic::temp_name(&leaf.to_string_lossy());
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        poll(control)?;
        let mut file = parent.open_with(&tmp, &options).map_err(|error| AutomationError::Io(format!("cannot create save temporary: {error}")))?;
        let written = (|| {
            poll(control)?;
            let output = write(&mut file)?;
            poll(control)?;
            let synced = file.sync_all();
            poll(control)?;
            synced.map_err(|error| AutomationError::Io(format!("cannot sync save temporary: {error}")))?;
            // Preserve the ordinary existing-file permissions before commit.
            if let Ok(metadata) = parent.metadata(leaf) {
                let _ = file.set_permissions(metadata.permissions());
            }
            poll(control)?;
            Ok(output)
        })();
        drop(file);
        let output = match written {
            Ok(output) => output,
            Err(error) => {
                let _ = parent.remove_file(&tmp);
                return Err(error);
            }
        };
        let mut protection_failure = None;
        let result = photocraft_format::atomic::retry_rename(photocraft_format::atomic::RenameRetry::platform(), || {
            let checked = check().and_then(|()| poll(control));
            if let Err(error) = checked {
                let message = error.to_string();
                protection_failure = Some(error);
                return Err(std::io::Error::other(message));
            }
            parent.rename(&tmp, &parent, leaf)
        });
        if let Err(error) = result {
            let _ = parent.remove_file(&tmp);
            if let Some(failure) = protection_failure {
                return Err(failure);
            }
            return Err(AutomationError::Io(format!("native protected save failed; original file unchanged: {error}")));
        }
        #[cfg(unix)]
        if let Ok(directory) = parent.try_clone() {
            let _ = directory.into_std_file().sync_all();
        }
        Ok(output)
    }
}
/// Flattened document as PNG, scaled to fit `max_side` (0 = full size).
pub fn render_png(doc: &Document, max_side: u32) -> Result<Vec<u8>, AutomationError> {
    let side = if max_side == 0 { doc.size.width.max(doc.size.height) } else { max_side };
    let img = photocraft_compose::thumbnail(doc, side.max(1)).map_err(photocraft_io::IoError::from)?;
    let image = photocraft_codecs::Image::from_u8(img.width, img.height, photocraft_codecs::ChannelLayout::Rgba, img.pixels)
        .map_err(|e| AutomationError::Other(e.to_string()))?;
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(|e| AutomationError::Other(e.to_string()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod controlled_native_write_tests {
    use super::*;
    #[test]
    fn source_inspection_raise_and_cancel_refuse_before_destination_io() {
        let root = root("inspection-admission");
        let path = root.join("absent.pcraft");
        let protection = photocraft_engine::WriteProtection::default();
        let result = validate_destination_protected_with_resident_limit(
            &path, &protection, Some("pcraft"), &photocraft_raster::Interrupt::NONE,
            photocraft_io::source_protection::resident_ceiling().checked_add(1).unwrap(),
        );
        assert!(matches!(result, Err(AutomationError::BadRequest(_))));
        let cancelled = || true;
        assert!(validate_destination_protected_with_resident_limit(
            &path, &protection, Some("pcraft"), &photocraft_raster::Interrupt::cancel_only(&cancelled), 0,
        ).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }
    fn root(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "photocraft-controlled-native-{}-{label}-{stamp}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn late_native_cancel_retains_destination_and_cleans_owned_temporary() {
        let root = root("late-cancel");
        let path = root.join("destination.pcraft");
        std::fs::write(&path, b"original destination").unwrap();
        let cancel = || {
            std::fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name() != std::ffi::OsStr::new("destination.pcraft") && entry.metadata().is_ok_and(|metadata| metadata.len() > 0))
        };
        let control = photocraft_raster::Interrupt::cancel_only(&cancel);
        let result = write_protected_native_with_interrupt(&path, &vec![b'n'; 192 << 10], &photocraft_engine::WriteProtection::default(), &control);
        assert!(matches!(result, Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))));
        assert_eq!(std::fs::read(&path).unwrap(), b"original destination");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn controlled_fingerprint_refusal_preserves_target_bytes() {
        let root = root("fingerprint");
        let path = root.join("renamed.pcraft");
        let original = b"preserved original source";
        std::fs::write(&path, original).unwrap();
        let protection = photocraft_engine::WriteProtection { fingerprints: vec![blake3::hash(original).to_hex().to_string()], ..Default::default() };
        let result = write_protected_native_with_interrupt(&path, b"replacement", &protection, &photocraft_raster::Interrupt::NONE);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn controlled_success_publishes_full_bytes_without_leftover_temp() {
        let root = root("success");
        let path = root.join("destination.pcraft");
        let bytes = vec![b'k'; 192 << 10];
        write_protected_native_with_interrupt(&path, &bytes, &photocraft_engine::WriteProtection::default(), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
