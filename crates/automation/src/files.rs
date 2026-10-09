//! File I/O shared by the MCP server and the CLI: open/save any supported
//! format, with `.pcraft` (ZIP or directory bundle) handled natively.

use std::path::Path;

use photocraft_doc::Document;
use photocraft_format::{PcraftWriter, SaveOptions};
use photocraft_io::ExportOptions;

use crate::AutomationError;

/// Result of opening a file.
pub struct Opened {
    pub document: Document,
    pub warnings: Vec<String>,
    pub origin: photocraft_io::ImportOrigin,
}

/// Decode a document that was read through a filesystem capability.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Opened, AutomationError> {
    if !bytes.starts_with(b"gimp xcf ") && Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
        return Ok(Opened {
            document: photocraft_format::load_from_bytes(bytes)?,
            warnings: Vec::new(),
            origin: photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft),
        });
    }
    let r = photocraft_io::import(name, bytes)?;
    Ok(Opened { document: r.document, warnings: r.warnings, origin: r.origin })
}

/// Open a document from disk. Directory bundles and `.pcraft` ZIPs load
/// natively; everything else goes through `photocraft-io` (PSD, PNG, …).
pub fn open(path: &Path) -> Result<Opened, AutomationError> {
    if path.is_dir() {
        return Ok(Opened {
            document: photocraft_format::load_path(path)?,
            warnings: Vec::new(),
            origin: photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft),
        });
    }
    let bytes = photocraft_format::read_file(path).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    open_bytes(&name, &bytes)
}

/// Previews for a `.pcraft` bundle.
pub fn previews(doc: &Document) -> SaveOptions {
    SaveOptions { thumbnail: Some(photocraft_compose::thumbnail(doc, 256)), composite: Some(photocraft_compose::thumbnail(doc, 1024)) }
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
        let mut local = PcraftWriter::new();
        let w = writer.unwrap_or(&mut local);
        w.save_path(doc, path, &previews(doc))?;
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
        return Ok((photocraft_format::save_to_bytes(doc, &previews(doc))?, Vec::new()));
    }
    let result = photocraft_io::export(doc, &ext, opts)?;
    Ok((result.bytes, result.warnings))
}

/// Reject import-only names independently of any format override, before encoding.
pub fn validate_write_name(path: &Path, format_override: Option<&str>) -> Result<(), AutomationError> {
    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("xcf"))
        || format_override.is_some_and(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case("xcf"))
    {
        return Err(AutomationError::BadRequest("XCF is import-only; choose a writable format and destination".into()));
    }
    Ok(())
}

/// Native callers already holding ambient file authority validate the destination before
/// encoding or creating a temporary output. Capability callers use `guard_write` instead.
pub fn validate_destination(path: &Path, source: Option<&photocraft_engine::SourceIdentity>, format_override: Option<&str>) -> Result<(), AutomationError> {
    use std::io::Read;
    validate_write_name(path, format_override)?;
    if let Some(source) = source.filter(|s| s.origin.is_read_only()).and_then(|s| s.path.as_deref()) {
        let source = Path::new(source);
        if source == path || matches!((std::fs::canonicalize(source), std::fs::canonicalize(path)), (Ok(a),Ok(b)) if a == b) {
            return Err(AutomationError::BadRequest("the original XCF source is protected; choose a different destination".into()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if path.exists() && same_file::is_same_file(source, path).map_err(|e| AutomationError::Io(format!("cannot establish XCF source identity: {e}")))? {
            return Err(AutomationError::BadRequest("an alias of the original XCF source is protected; choose a different destination".into()));
        }
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut prefix = [0; 9];
            let full = match file.read_exact(&mut prefix) {
                Ok(()) => true,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => false,
                Err(e) => return Err(AutomationError::Io(format!("validate destination {}: {e}", path.display()))),
            };
            if full && &prefix == b"gimp xcf " {
                return Err(AutomationError::BadRequest("an existing XCF destination is protected; choose a different destination".into()));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) if path.is_dir() => {
            let _ = e;
        }
        Err(e) => return Err(AutomationError::Io(format!("cannot validate destination {}: {e}", path.display()))),
    }
    Ok(())
}

/// Flattened document as PNG, scaled to fit `max_side` (0 = full size).
pub fn render_png(doc: &Document, max_side: u32) -> Result<Vec<u8>, AutomationError> {
    let side = if max_side == 0 { doc.size.width.max(doc.size.height) } else { max_side };
    let img = photocraft_compose::thumbnail(doc, side.max(1));
    let image = photocraft_codecs::Image::from_u8(img.width, img.height, photocraft_codecs::ChannelLayout::Rgba, img.pixels)
        .map_err(|e| AutomationError::Other(e.to_string()))?;
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(|e| AutomationError::Other(e.to_string()))
}
