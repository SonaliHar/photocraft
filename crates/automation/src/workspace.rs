//! Capability-based filesystem policy for untrusted automation paths.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde_json::Value;

use crate::AutomationError;

mod accounted_native_open;
mod accounted_native_save;
mod linked_read;
mod source_write;
mod stored_channel_read;

const DENIED: &str = "automation filesystem access is not granted";

#[derive(Clone)]
struct RootCapability {
    dir: Arc<Dir>,
    identity: PathBuf,
    path: PathBuf,
}

/// Separate directory capabilities for automation reads and writes.
///
/// Root paths are trusted launch-time configuration. Request paths are always
/// untrusted, forward-slash relative paths resolved by `cap-std` beneath the
/// held directory handle, or absolute paths whose text lies beneath the root
/// (translated to relative ones, see [`request_path`]). Parent traversal,
/// alternate separators, Windows prefixes and empty components are rejected
/// before I/O.
#[derive(Clone, Default)]
pub struct AuthorizedWorkspace {
    read: Option<RootCapability>,
    write: Option<RootCapability>,
}

impl AuthorizedWorkspace {
    /// Presence of an already-open read capability; no new authority is made.
    pub fn has_read_root(&self) -> bool {
        self.read.is_some()
    }
    /// Explicit original input beneath the existing held read root. No document
    /// locator, source identity or serialized grant is consulted. The host must
    /// capture its target and admit reader storage before calling this method.
    pub fn read_xcf_stored_channel_source(&self, path: &str, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>, AutomationError> {
        control.check().map_err(|_| AutomationError::Engine(photocraft_engine::EngineError::Cancelled))?;
        if path.is_empty() || path.len() > 4096 || path.contains('\0') {
            return Err(AutomationError::BadRequest("select.loadSelection.originalPath: bounded explicit path required".into()));
        }
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let relative = request_path(path, Some(root))?;
        stored_channel_read::read(&root.dir, &relative, control)
    }
    /// Read explicitly requested linked material below the held read-root capability.
    /// The caller captures the engine target before I/O, then constructs and
    /// immediately executes the registered substitution request after reading.
    /// No stored locator, resource kind, document or grant is consulted here.
    pub fn read_xcf_linked_replacement(&self, path: &str, control: &photocraft_raster::Interrupt<'_>) -> Result<Vec<u8>, AutomationError> {
        control.check().map_err(|_| AutomationError::Engine(photocraft_engine::EngineError::Cancelled))?;
        if path.len() > 4096 {
            return Err(AutomationError::BadRequest("xcf.link.substituteFromRoot.path-limit".into()));
        }
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        linked_read::read(&root.dir, &relative, control)
    }

    /// Explicit request path below the held read root, never a source-stored locator.
    /// Bytes and grants are separate: only ordinary resolved document data is saved.
    pub fn recover_xcf_resource(
        &self,
        session: &mut photocraft_engine::Session,
        resource: photocraft_doc::item_identity::ResourceId,
        path: &str,
        policy: photocraft_engine::xcf_resources::RecoveryPolicy,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<photocraft_doc::xcf_resources::ResourceFidelity, AutomationError> {
        use std::io::Read;
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let kind = session
            .active()
            .and_then(|active| active.doc.xcf_state.as_ref())
            .and_then(|state| state.resources.iter().find(|entry| entry.id == resource))
            .map(|entry| entry.kind.clone())
            .ok_or_else(|| AutomationError::BadRequest("unknown XCF resource identity".into()))?;
        if kind == photocraft_doc::xcf_resources::ResourceKind::Font {
            return Err(AutomationError::BadRequest("font recovery requires an installed-font identity service".into()));
        }
        control.check().map_err(|_| AutomationError::BadRequest("XCF resource recovery cancelled".into()))?;
        let mut file = root.dir.open(&relative).map_err(|e| file_error("resource read", path, e))?;
        let metadata = file.metadata().map_err(|e| file_error("resource inspect", path, e))?;
        if !metadata.is_file() || metadata.len() > 64 << 20 {
            return Err(AutomationError::BadRequest("XCF resource is not a bounded regular file".into()));
        }
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 32 << 10];
        loop {
            control.check().map_err(|_| AutomationError::BadRequest("XCF resource recovery cancelled".into()))?;
            let count = file.read(&mut chunk).map_err(|e| file_error("resource read", path, e))?;
            if count == 0 {
                break;
            }
            if bytes.len().checked_add(count).is_none_or(|size| size > 64 << 20) {
                return Err(AutomationError::BadRequest("XCF resource read limit exceeded".into()));
            }
            bytes.try_reserve(count).map_err(|_| AutomationError::BadRequest("XCF resource allocation denied".into()))?;
            bytes.extend_from_slice(chunk.get(..count).ok_or_else(|| AutomationError::BadRequest("invalid resource read count".into()))?);
        }
        let input = photocraft_engine::xcf_resources::ServiceResourceBytes::from_authorized_service(kind, bytes, true)
            .map_err(|e| AutomationError::BadRequest(e.to_string()))?;
        photocraft_engine::xcf_resources::recover_authorized_bytes(session, resource, input, policy, control)
            .map_err(|e| AutomationError::BadRequest(e.to_string()))
    }
    /// Check an output through the write capability without obtaining ambient authority.
    /// Existing XCF contents remain protected even when renamed or hard-linked.
    pub fn guard_write(&self, path: &str, protected_source: Option<&str>) -> Result<(), AutomationError> {
        source_write::check(self, path, source_write::Sources::one(protected_source), &[])
    }
    /// Check inert native fingerprint evidence without reading any metadata locator.
    pub fn guard_write_document(&self, path: &str, source: Option<&str>, document: &photocraft_doc::Document) -> Result<(), AutomationError> {
        let fingerprints = photocraft_io::source_protection::document_fingerprints(document)
            .map_err(|error| AutomationError::BadRequest(format!("automation source protection: {error}")))?;
        source_write::check(self, path, source_write::Sources::one(source), &fingerprints)
    }

    /// Protect all caller-supplied runtime identities and inert source fingerprints.
    pub fn guard_write_protected(&self, path: &str, sources: &[String], fingerprints: &[String]) -> Result<(), AutomationError> {
        source_write::check(self, path, source_write::Sources::many(sources), fingerprints)
    }
    /// Controlled protection check using only the existing held read/write roots.
    pub fn guard_write_with_interrupt(&self, path: &str, source: Option<&str>, control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
        source_write::check_with_interrupt(self, path, source_write::Sources::one(source), &[], control)
    }
    pub fn guard_write_document_with_interrupt(
        &self,
        path: &str,
        source: Option<&str>,
        document: &photocraft_doc::Document,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(), AutomationError> {
        let fingerprints = photocraft_io::source_protection::document_fingerprints_with_interrupt(document, control).map_err(|error| {
            if matches!(error, photocraft_io::source_protection::SourceProtectionError::Cancelled) {
                AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
            } else {
                AutomationError::BadRequest(format!("automation source protection: {error}"))
            }
        })?;
        source_write::check_with_interrupt(self, path, source_write::Sources::one(source), &fingerprints, control)
    }
    pub fn guard_write_protected_with_interrupt(
        &self,
        path: &str,
        sources: &[String],
        fingerprints: &[String],
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(), AutomationError> {
        source_write::check_with_interrupt(self, path, source_write::Sources::many(sources), fingerprints, control)
    }

    /// Open the configured roots as capabilities. Either authority may be
    /// omitted; omitted authority fails closed.
    pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, AutomationError> {
        Ok(Self { read: open_root(read_root, "read")?, write: open_root(write_root, "write")? })
    }

    /// Read one regular file below the configured read root.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, AutomationError> {
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let mut file = root.dir.open(&relative).map_err(|e| file_error("read", path, e))?;
        let metadata = file.metadata().map_err(|e| file_error("read", path, e))?;
        if !metadata.is_file() {
            return Err(AutomationError::BadRequest(format!("automation read path is not a regular file: `{path}`")));
        }
        read_to_observed_length(&mut file, metadata.len())
    }

    /// Load one `.pcraft` document in directory form â€” the native on-disk layout â€” below the
    /// read root. `read` serves regular files only (the zip form), so a regular file at the
    /// path is reported as `Ok(None)` for the caller to fall back to.
    pub fn open_pcraft(&self, path: &str) -> Result<Option<photocraft_doc::Document>, AutomationError> {
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let dir = match root.dir.open_dir(&relative) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return Ok(None),
            Err(e) => return Err(file_error("read", path, e)),
        };
        let source = DirHandle { dir };
        Ok(Some(photocraft_format::load_bytes_source(&source, &photocraft_format::LoadOptions::default())?))
    }

    /// Open a document through the existing held read root. Native archives
    /// decode from this file handle without retaining the complete archive;
    /// other formats use the same handle and the legacy authorized-size policy.
    /// This grants no source locator access or serialized filesystem authority.
    pub fn open_document_with_interrupt(&self, path: &str, control: &photocraft_raster::Interrupt<'_>) -> Result<crate::files::Opened, AutomationError> {
        use std::io::{Seek, SeekFrom};
        let relative = request_path(path, self.read.as_ref())?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        let cancelled = || AutomationError::Engine(photocraft_engine::EngineError::Cancelled);
        control.check().map_err(|_| cancelled())?;
        let mut file = root.dir.open(&relative).map_err(|error| file_error("read", path, error))?;
        let metadata = file.metadata().map_err(|error| file_error("read", path, error))?;
        if metadata.is_dir() {
            let document = self.open_pcraft(path)?.ok_or_else(|| AutomationError::BadRequest("native directory did not load".into()))?;
            control.check().map_err(|_| cancelled())?;
            return Ok(crate::files::Opened {
                document,
                warnings: Vec::new(),
                origin: photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft),
                assessment: None,
                source_read_only: false,
                preview_only: false,
            });
        }
        if !metadata.is_file() {
            return Err(AutomationError::BadRequest(format!("automation read path is not a regular file: `{path}`")));
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let candidate = photocraft_io::import_native_stream_candidate(name, &mut file, control)
            .map_err(|error| if matches!(error, photocraft_io::IoError::Cancelled) { cancelled() } else { AutomationError::Import(error) })?;
        control.check().map_err(|_| cancelled())?;
        if let Some(imported) = candidate {
            return Ok(crate::files::Opened {
                document: imported.document,
                warnings: imported.warnings,
                origin: imported.origin,
                assessment: imported.assessment,
                source_read_only: imported.source_read_only,
                preview_only: imported.preview_only,
            });
        }
        // Probe fallback must rewind this held handle, never reopen an ambient path.
        file.seek(SeekFrom::Start(0)).map_err(|error| file_error("read rewind", path, error))?;
        control.check().map_err(|_| cancelled())?;
        let bytes = photocraft_format::read::read_all_with_interrupt(&mut file, metadata.len(), control)
            .map_err(|error| if control.cancelled() { cancelled() } else { file_error("read", path, error) })?;
        let opened = crate::files::open_bytes_with_interrupt(name, &bytes, control)?;
        control.check().map_err(|_| cancelled())?;
        Ok(opened)
    }

    /// Create or replace one file below the configured write root, crash-safely: the bytes go to
    /// a temporary file beside the target, which is synced and renamed over it (the same steps
    /// as [`photocraft_format::atomic_write`], through the directory capability). On failure
    /// the previous file is untouched and the temporary file is removed.
    ///
    /// The parent directory must already exist. `cap-std` performs path
    /// resolution and file creation relative to the held directory handle, so
    /// a non-existent final target is supported without ambient path access.
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<(), AutomationError> {
        self.write_with_source(path, bytes, None)
    }

    /// Retain an authorized runtime source identity through final destination checks.
    /// The source path is resolved only beneath the separately held read root.
    pub fn write_with_source(&self, path: &str, bytes: &[u8], protected_source: Option<&str>) -> Result<(), AutomationError> {
        source_write::write(self, path, bytes, source_write::Sources::one(protected_source), &[])
    }
    /// Carry the same document fingerprints through initial and final held-parent checks.
    pub fn write_document(&self, path: &str, bytes: &[u8], source: Option<&str>, document: &photocraft_doc::Document) -> Result<(), AutomationError> {
        let fingerprints = photocraft_io::source_protection::document_fingerprints(document)
            .map_err(|error| AutomationError::BadRequest(format!("automation source protection: {error}")))?;
        source_write::write(self, path, bytes, source_write::Sources::one(source), &fingerprints)
    }

    /// Retain all open-document protection evidence through every final rename attempt.
    pub fn write_protected(&self, path: &str, bytes: &[u8], sources: &[String], fingerprints: &[String]) -> Result<(), AutomationError> {
        source_write::write(self, path, bytes, source_write::Sources::many(sources), fingerprints)
    }

    /// Poll during protection/read/write/sync and before each held-parent rename.
    /// Blocking syscalls cannot be interrupted. Successful rename commits the write;
    /// later cancellation cannot turn an already-published replacement into failure.
    pub fn write_with_interrupt(&self, path: &str, bytes: &[u8], control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
        self.write_with_source_and_interrupt(path, bytes, None, control)
    }
    pub fn write_with_source_and_interrupt(
        &self,
        path: &str,
        bytes: &[u8],
        source: Option<&str>,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(), AutomationError> {
        source_write::write_with_interrupt(self, path, bytes, source_write::Sources::one(source), &[], control)
    }
    pub fn write_document_with_interrupt(
        &self,
        path: &str,
        bytes: &[u8],
        source: Option<&str>,
        document: &photocraft_doc::Document,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(), AutomationError> {
        let fingerprints = photocraft_io::source_protection::document_fingerprints_with_interrupt(document, control).map_err(|error| {
            if matches!(error, photocraft_io::source_protection::SourceProtectionError::Cancelled) {
                AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
            } else {
                AutomationError::BadRequest(format!("automation source protection: {error}"))
            }
        })?;
        source_write::write_with_interrupt(self, path, bytes, source_write::Sources::one(source), &fingerprints, control)
    }
    pub fn write_protected_with_interrupt(
        &self,
        path: &str,
        bytes: &[u8],
        sources: &[String],
        fingerprints: &[String],
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<(), AutomationError> {
        source_write::write_with_interrupt(self, path, bytes, source_write::Sources::many(sources), fingerprints, control)
    }
    /// Internal prepared serializer writes through the existing held-root capability.
    /// Preparation/caps belong to the caller and precede any temp-file creation.
    pub(crate) fn write_protected_stream_with_interrupt<R>(
        &self,
        path: &str,
        sources: &[String],
        fingerprints: &[String],
        control: &photocraft_raster::Interrupt<'_>,
        write: impl FnOnce(&mut cap_std::fs::File) -> Result<R, AutomationError>,
    ) -> Result<R, AutomationError> {
        source_write::write_stream_with_interrupt(self, path, source_write::Sources::many(sources), fingerprints, control, write)
    }
    /// Prepared native save under this held write root, with caller-captured
    /// all-open source protection. No ambient paths or complete archive Vec.
    pub fn save_native_with_interrupt(
        &self,
        document: &photocraft_doc::Document,
        path: &str,
        protection: &photocraft_engine::WriteProtection,
        limits: photocraft_io::NativeSaveLimits,
        control: &photocraft_raster::Interrupt<'_>,
    ) -> Result<photocraft_io::NativeSaveResult, AutomationError> {
        let error = |error: photocraft_io::IoError| {
            if matches!(error, photocraft_io::IoError::Cancelled) {
                AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
            } else {
                AutomationError::Import(error)
            }
        };
        self.guard_write_protected_with_interrupt(path, &protection.sources, &protection.fingerprints, control)?;
        let prepared = photocraft_io::prepare_native_stream(document, limits, control).map_err(error)?;
        self.write_protected_stream_with_interrupt(path, &protection.sources, &protection.fingerprints, control, move |file| {
            let (_, result) = prepared.write(file).map_err(error)?;
            Ok(result)
        })
    }
}

fn read_to_observed_length(reader: &mut impl Read, observed_length: u64) -> Result<Vec<u8>, AutomationError> {
    // Do not reserve from a potentially stale or attacker-controlled metadata length. Read in
    // chunks, and reject growth beyond the length observed on the opened file before appending.
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 << 10];
    loop {
        let remaining = observed_length.saturating_sub(bytes.len() as u64);
        let ask = chunk.len().min(usize::try_from(remaining.saturating_add(1)).unwrap_or(chunk.len()));
        let n = match reader.read(&mut chunk[..ask]) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(AutomationError::Io(format!("automation read failed: {e}"))),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if n as u64 > remaining {
            return Err(AutomationError::BadRequest(format!("automation file grew beyond its observed length of {observed_length} bytes")));
        }
        let needed = bytes.len().checked_add(n).ok_or_else(|| AutomationError::BadRequest("automation file is too large to read".into()))?;
        if needed > bytes.capacity() {
            let target = bytes.capacity().saturating_add(bytes.capacity() / 2).max(needed).min(usize::try_from(observed_length).unwrap_or(usize::MAX));
            bytes.try_reserve_exact(target - bytes.len()).map_err(|e| AutomationError::Io(format!("automation read allocation failed: {e}")))?;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

fn open_root(path: Option<&Path>, authority: &str) -> Result<Option<RootCapability>, AutomationError> {
    let Some(path) = path else { return Ok(None) };
    if path.as_os_str().is_empty() {
        return Err(AutomationError::BadRequest(format!("automation {authority} root is empty")));
    }
    let dir = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|e| AutomationError::Io(format!("cannot open automation {authority} root `{}`: {e}", path.display())))?;
    // Textual (no link resolution), like the request paths it is compared with.
    let absolute =
        std::path::absolute(path).map_err(|e| AutomationError::Io(format!("cannot resolve automation {authority} root `{}`: {e}", path.display())))?;
    let identity = std::fs::canonicalize(path).map_err(|e| AutomationError::Io(format!("cannot identify automation {authority} root: {e}")))?;
    Ok(Some(RootCapability { dir: Arc::new(dir), identity, path: absolute }))
}

/// A request path as the relative path the root's capability resolves. An absolute path is
/// accepted only when its text names a file beneath `root`: the root's components are
/// stripped (the drive letter compared without case) and the rest must pass
/// [`relative_path`], so `..`, prefixes and device names are refused as before. Nothing is
/// resolved on the filesystem; the held directory handle still does all the opening, so
/// links cannot escape the root.
fn request_path(raw: &str, root: Option<&RootCapability>) -> Result<PathBuf, AutomationError> {
    let Some(root) = root.filter(|_| Path::new(raw).is_absolute()) else { return relative_path(raw) };
    let mut request = Path::new(raw).components();
    for want in root.path.components() {
        let same = match (request.next(), want) {
            (Some(Component::Prefix(a)), Component::Prefix(b)) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
            (Some(got), want) => got == want,
            (None, _) => false,
        };
        if !same {
            return Err(path_error(raw, "absolute paths must be inside the automation root"));
        }
    }
    let rest: Vec<&str> = request.map(|c| c.as_os_str().to_str().unwrap_or("\u{FFFD}")).collect();
    relative_path(&rest.join("/"))
}

fn relative_path(raw: &str) -> Result<PathBuf, AutomationError> {
    if raw.is_empty() {
        return Err(path_error(raw, "path is empty"));
    }
    if raw.contains('\\') {
        return Err(path_error(raw, "alternate separators are not allowed; use `/`"));
    }
    if raw.starts_with('/') {
        return Err(path_error(raw, "absolute paths are not allowed"));
    }
    if raw.contains(':') {
        return Err(path_error(raw, "drive, device and stream prefixes are not allowed"));
    }
    for component in raw.split('/') {
        if component.is_empty() {
            return Err(path_error(raw, "empty path components are not allowed"));
        }
        if component == "." {
            return Err(path_error(raw, "`.` path components are not allowed"));
        }
        if component == ".." {
            return Err(path_error(raw, "parent traversal is not allowed"));
        }
        if component.ends_with(['.', ' ']) {
            return Err(path_error(raw, "path components ending in a dot or space are not allowed"));
        }
        if is_windows_device_name(component) {
            return Err(path_error(raw, "reserved device names are not allowed"));
        }
    }
    Ok(PathBuf::from(raw))
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
        || stem.strip_prefix("COM").is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        || stem.strip_prefix("LPT").is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}

fn path_error(path: &str, reason: &str) -> AutomationError {
    AutomationError::BadRequest(format!("automation path rejected: {reason}: `{path}`"))
}

/// A `.pcraft` directory behind the read-root capability: bounded reads, no ambient paths.
struct DirHandle {
    dir: Dir,
}

impl photocraft_format::ByteSource for DirHandle {
    fn get(&self, path: &str, max: usize) -> photocraft_format::Result<Vec<u8>> {
        let missing = || photocraft_format::FormatError::Corrupt(format!("missing `{path}`"));
        let mut file = self.dir.open(path).map_err(|_| missing())?;
        let metadata = file.metadata().map_err(|_| missing())?;
        if !metadata.is_file() {
            // A FIFO or device would block the open and stream without end.
            return Err(photocraft_format::FormatError::Corrupt(format!("`{path}` is not a regular file")));
        }
        photocraft_format::read_entry(&mut file, metadata.len(), max, path)
    }
}

fn file_error(operation: &str, path: &str, error: std::io::Error) -> AutomationError {
    let kind = error.kind();
    AutomationError::Io(format!("automation {operation} `{path}` failed ({kind:?}): {error}"))
}

/// Filesystem-bearing engine commands have not yet been converted to consume
/// directory capabilities. Automation must use `doc.open`, `doc.save` and
/// `doc.render` for file effects until those commands are migrated.
pub fn authorize_engine_command(id: &str, params: &Value) -> Result<(), AutomationError> {
    let safe_file_command = matches!(
        id,
        "file.new"
            | "file.open.discardPending"
            | "file.newFromClipboard"
            | "file.close"
            | "file.closeAll"
            | "file.closeOthers"
            | "file.fileInfo"
            | "file.automate.fitImage"
            | "file.automate.conditionalModeChange"
            | "file.automate.cropAndStraightenPhotos"
            | "file.scripts.flattenAllLayerEffects"
            | "file.scripts.flattenAllMasks"
            | "file.scripts.deleteAllEmptyLayers"
            | "file.export.exportPreferences"
    );
    if id.starts_with("file.") && !safe_file_command {
        return Err(command_error(id));
    }
    // Contents commands re-check the resolved source in the engine before any disk read.
    // Photoshop's embedded linked-layer blocks count as in-memory sources too.
    let in_memory_smart_command = matches!(
        id,
        "layer.smartObjects.convertToSmartObject"
            | "layer.smartObjects.editContents"
            | "layer.smartObjects.convertToLayers"
            | "layer.smartObjects.saveContents"
    );
    if (id.starts_with("layer.smartObjects.") && !in_memory_smart_command)
        || matches!(
            id,
            "pattern.import"
                | "pattern.export"
                | "edit.presets.migratePresets"
                | "measurementLog.export"
                | "layer.videoLayers.newVideoLayerFromFile"
                | "layer.videoLayers.replaceFootage"
                | "layer.videoLayers.reloadFrame"
        )
        || command_uses_ambient_path(id, params)
        || profile_command_may_read_ambient(id, params)
        || preferences_may_grant_ambient_paths(id, params)
        || params_contain_ambient_path(id, params)
    {
        return Err(command_error(id));
    }
    Ok(())
}

/// Desktop sessions may already contain user-configured ambient colour-profile
/// paths. Commands which implicitly resolve those settings are denied even
/// when their request parameters contain no path. Fresh headless sessions
/// cannot acquire such settings through the automation interface.
pub fn authorize_desktop_engine_command(id: &str, params: &Value) -> Result<(), AutomationError> {
    authorize_engine_command(id, params)?;
    if id.starts_with("image.mode.") {
        return Err(command_error(id));
    }
    Ok(())
}

/// [`authorize_engine_command`] as the function pointer [`photocraft_engine::Session::authorize`]
/// stores. `actions.play` calls it for every nested step.
pub fn authorize_engine_step(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    authorize_engine_command(id, params).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
}

/// [`authorize_desktop_engine_command`] as a [`photocraft_engine::Session::authorize`] hook.
pub fn authorize_desktop_engine_step(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    authorize_desktop_engine_command(id, params).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
}

fn params_contain_ambient_path(id: &str, params: &Value) -> bool {
    let keys: &[&str] = match id {
        "image.adjustments.colorLookup" | "layer.newAdjustmentLayer.colorLookup" | "layer.setAdjustment" => &["file"],
        "filter.distort.displace" => &["mapPath"],
        "layer.quickExportAsPng" | "layer.exportAs" | "image.applyDataSet" | "layer.smartObjects.editContents" | "layer.smartObjects.convertToLayers" => {
            &["path"]
        }
        "image.mode.rgb" | "image.mode.grayscale" | "image.mode.cmyk" | "image.mode.lab" => &["profile"],
        "edit.assignProfile" | "edit.convertToProfile" | "edit.profileInfo" | "view.proofSetup" | "view.gamutWarning" => &["profile"],
        "edit.colorSettings" => &["workingRgb", "workingCmyk", "workingGray"],
        _ => &[],
    };
    keys.iter().any(|key| {
        params
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| if matches!(*key, "file" | "mapPath" | "path") { !value.is_empty() } else { looks_like_path(value) })
    })
}

fn profile_command_may_read_ambient(id: &str, params: &Value) -> bool {
    // Installed profiles scan OS and user directories even without a request path.
    if matches!(id, "color.profileMismatch" | "edit.colorSettings" | "edit.installedProfiles" | "view.proofSetup") {
        return true;
    }
    matches!(id, "edit.assignProfile" | "edit.convertToProfile" | "edit.profileInfo")
        && params.get("profile").and_then(Value::as_str).is_some_and(|profile| {
            matches!(profile, "working" | "default" | "working-cmyk" | "workingCmyk" | "working-rgb" | "workingRgb" | "working-gray" | "workingGray")
        })
}

fn preferences_may_grant_ambient_paths(id: &str, params: &Value) -> bool {
    if id != "prefs.set" {
        return false;
    }
    let direct = params.get("path").and_then(Value::as_str).is_some_and(preference_uses_ambient_filesystem);
    let batch = params.get("values").and_then(Value::as_object).is_some_and(|values| values.keys().any(|path| preference_uses_ambient_filesystem(path)));
    direct || batch
}

fn preference_uses_ambient_filesystem(path: &str) -> bool {
    // The engine skips empty segments (`.scriptEvents.enabled` sets `scriptEvents.enabled`), so
    // judge the first non-empty one; no segment at all is the whole-preferences update.
    let Some(section) = path.split('.').find(|segment| !segment.is_empty()) else { return true };
    matches!(section, "colorSettings" | "scriptEvents" | "historyLog" | "plugIns" | "scratchDisks")
}

fn command_uses_ambient_path(id: &str, params: &Value) -> bool {
    match id {
        "actions.load" | "brush.presets.importAbr" | "gradient.presets.importGrd" | "plugin.install" | "swatches.import" => {
            // `data` wins over `path` in these commands; any `path` without it reads the filesystem.
            params.get("data").is_none() && params.get("path").is_some()
        }
        // With a `path` the file is written there; without one the bytes come back as `data`.
        "actions.save" | "swatches.export" => params.get("path").is_some(),
        "plugin.reload" => true,
        _ => false,
    }
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.to_ascii_lowercase().ends_with(".icc")
        || value.to_ascii_lowercase().ends_with(".icm")
}

fn command_error(id: &str) -> AutomationError {
    AutomationError::BadRequest(format!("automation command `{id}` uses ambient filesystem paths and is disabled; use capability-scoped document methods"))
}

/// An export owns only files it created through this held directory capability. Refuse
/// collisions (including symlinks) so cancellation cannot delete or replace an earlier export.
pub(crate) struct ExportSequence {
    dir: Dir,
    created: Vec<String>,
}

impl AuthorizedWorkspace {
    pub(crate) fn export_sequence(&self, path: &str) -> Result<ExportSequence, AutomationError> {
        let relative = request_path(path, self.write.as_ref())?;
        let root = self.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        root.dir.create_dir_all(&relative).map_err(|e| file_error("mkdir", path, e))?;
        let dir = root.dir.open_dir(&relative).map_err(|e| file_error("open export directory", path, e))?;
        Ok(ExportSequence { dir, created: Vec::new() })
    }
}

impl ExportSequence {
    pub(crate) fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), AutomationError> {
        let relative = relative_path(name)?;
        if relative.components().count() != 1 {
            return Err(path_error(name, "export filename must be a single component"));
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self.dir.open_with(&relative, &options).map_err(|e| file_error("create export", name, e))?;
        // Record immediately after creation, before any fallible write, including partial writes.
        self.created.push(name.to_owned());
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| file_error("write export", name, e))
    }

    pub(crate) fn commit(&mut self) {
        self.created.clear();
    }

    pub(crate) fn cleanup(&mut self) -> Result<(), AutomationError> {
        let mut failed = Vec::new();
        self.created.retain(|name| match self.dir.remove_file(name) {
            Ok(()) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                true
            }
        });
        if failed.is_empty() { Ok(()) } else { Err(AutomationError::Io(format!("export cleanup failed: {}", failed.join("; ")))) }
    }
}

impl Drop for ExportSequence {
    fn drop(&mut self) {
        // Also cover an escaped rendering panic; normal errors report cleanup failures above.
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(name: &str) -> (PathBuf, PathBuf, AuthorizedWorkspace) {
        let base = std::env::temp_dir().join(format!("photocraft-workspace-{}-{name}", std::process::id()));
        let inside = base.join("inside");
        let outside = base.join("outside");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let workspace = AuthorizedWorkspace::new(Some(&inside), Some(&inside)).unwrap();
        (inside, outside, workspace)
    }

    #[test]
    fn controlled_write_late_cancel_preserves_destination_and_cleans_only_owned_temp() {
        let (inside, _, workspace) = roots("controlled-late-cancel");
        let output = inside.join("output.pcraft");
        std::fs::write(&output, b"original destination").unwrap();
        // Cancel only after the actual private temp has received a write chunk.
        let cancel = || {
            std::fs::read_dir(&inside).ok().is_some_and(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.file_name() != std::ffi::OsStr::new("output.pcraft") && entry.metadata().is_ok_and(|meta| meta.len() > 0))
            })
        };
        let control = photocraft_raster::Interrupt::cancel_only(&cancel);
        let error = workspace.write_with_interrupt("output.pcraft", &vec![b'n'; 192 << 10], &control).unwrap_err();
        assert!(matches!(error, AutomationError::Engine(photocraft_engine::EngineError::Cancelled)));
        assert_eq!(std::fs::read(&output).unwrap(), b"original destination");
        assert_eq!(std::fs::read_dir(&inside).unwrap().count(), 1);
    }
    #[test]
    fn controlled_guard_cancel_preserves_existing_destination() {
        let (inside, _, workspace) = roots("controlled-guard-cancel");
        let output = inside.join("output.pcraft");
        std::fs::write(&output, b"existing content").unwrap();
        let cancel = || true;
        let control = photocraft_raster::Interrupt::cancel_only(&cancel);
        let error = workspace.guard_write_protected_with_interrupt("output.pcraft", &[], &[], &control).unwrap_err();
        assert!(matches!(error, AutomationError::Engine(photocraft_engine::EngineError::Cancelled)));
        assert_eq!(std::fs::read(&output).unwrap(), b"existing content");
    }

    #[test]
    fn valid_read_and_nonexistent_output_write_stay_in_root() {
        let (inside, _, workspace) = roots("valid");
        std::fs::write(inside.join("read.txt"), b"canary").unwrap();
        assert_eq!(workspace.read("read.txt").unwrap(), b"canary");
        workspace.write("new-output.txt", b"created").unwrap();
        assert_eq!(std::fs::read(inside.join("new-output.txt")).unwrap(), b"created");
    }

    // Unrun held-root streaming-open drafts. They do not install a Session
    // document or acquire a runtime source grant.
    #[test]
    fn controlled_open_native_and_flat_fallback_use_held_root() {
        let (inside, _, workspace) = roots("stream-open");
        let document =
            photocraft_doc::Document::new("retained native", photocraft_geom::Size::new(8, 6), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        std::fs::write(inside.join("document.pcraft"), &native).unwrap();
        let opened = workspace.open_document_with_interrupt("document.pcraft", &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(opened.document, document);
        assert_eq!(opened.origin.format, photocraft_io::ImportFormat::Pcraft);
        assert_eq!(std::fs::read(inside.join("document.pcraft")).unwrap(), native);
        let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgba, vec![13, 29, 47, 255]).unwrap();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        std::fs::write(inside.join("image.png"), &png).unwrap();
        let expected = crate::files::open_bytes("image.png", &png).unwrap();
        let actual = workspace.open_document_with_interrupt("image.png", &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(actual.document.size, expected.document.size);
        assert_eq!(actual.origin, expected.origin);
        assert!(workspace.open_document_with_interrupt("../outside/image.png", &photocraft_raster::Interrupt::NONE).is_err());
        assert!(AuthorizedWorkspace::default().open_document_with_interrupt("image.png", &photocraft_raster::Interrupt::NONE).is_err());
    }
    #[test]
    fn controlled_open_cancel_and_truncated_native_never_return_document() {
        let (inside, _, workspace) = roots("stream-open-refusal");
        let document =
            photocraft_doc::Document::new("native refusal", photocraft_geom::Size::new(8, 6), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        std::fs::write(inside.join("document.pcraft"), &native).unwrap();
        let cancel = || true;
        assert!(matches!(
            workspace.open_document_with_interrupt("document.pcraft", &photocraft_raster::Interrupt::cancel_only(&cancel)),
            Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))
        ));
        let truncated = native.get(..native.len().saturating_sub(7)).unwrap();
        std::fs::write(inside.join("truncated.pcraft"), truncated).unwrap();
        assert!(workspace.open_document_with_interrupt("truncated.pcraft", &photocraft_raster::Interrupt::NONE).is_err());
        assert_eq!(std::fs::read(inside.join("document.pcraft")).unwrap(), native);
    }

    #[cfg(unix)]
    #[test]
    fn controlled_open_keeps_original_held_root_after_launch_path_replacement() {
        let (inside, _, workspace) = roots("stream-open-held-root");
        let document =
            photocraft_doc::Document::new("held identity", photocraft_geom::Size::new(8, 6), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        std::fs::write(inside.join("document.pcraft"), &native).unwrap();
        let renamed = inside.with_file_name("renamed-inside");
        std::fs::rename(&inside, &renamed).unwrap();
        std::fs::create_dir(&inside).unwrap();
        std::fs::write(inside.join("document.pcraft"), b"ambient impostor").unwrap();
        let opened = workspace.open_document_with_interrupt("document.pcraft", &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(opened.document, document);
        assert_eq!(std::fs::read(inside.join("document.pcraft")).unwrap(), b"ambient impostor");
    }

    fn temp_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect()
    }

    #[test]
    fn write_replaces_atomically_and_leaves_no_temp_file() {
        let (inside, _, workspace) = roots("atomic");
        std::fs::create_dir_all(inside.join("sub")).unwrap();
        workspace.write("sub/out.psd", b"original").unwrap();
        workspace.write("sub/out.psd", b"replacement").unwrap();
        assert_eq!(std::fs::read(inside.join("sub/out.psd")).unwrap(), b"replacement");
        assert!(temp_files(&inside.join("sub")).is_empty());
        // Renaming over a directory fails: nothing is left behind and the directory survives.
        std::fs::create_dir_all(inside.join("adir")).unwrap();
        assert!(workspace.write("adir", b"x").is_err());
        assert!(inside.join("adir").is_dir());
        assert!(temp_files(&inside).is_empty());
    }
    // Unrun prepared-sink regression drafts; no full archive buffer is supplied.
    #[test]
    fn prepared_sink_can_seek_and_returns_only_after_guarded_publication() {
        use std::io::{Seek, SeekFrom, Write};
        let (inside, _, workspace) = roots("stream-seek");
        std::fs::write(inside.join("out.pcraft"), b"old").unwrap();
        let returned = workspace
            .write_protected_stream_with_interrupt("out.pcraft", &[], &[], &photocraft_raster::Interrupt::NONE, |file| {
                file.write_all(b"0000body").unwrap();
                file.seek(SeekFrom::Start(0)).unwrap();
                file.write_all(b"head").unwrap();
                Ok(7)
            })
            .unwrap();
        assert_eq!(returned, 7);
        assert_eq!(std::fs::read(inside.join("out.pcraft")).unwrap(), b"headbody");
        assert!(temp_files(&inside).is_empty());
    }
    #[test]
    fn prepared_sink_failure_or_late_cancel_preserves_destination_and_cleans_only_owned_temp() {
        use std::io::Write;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (inside, _, workspace) = roots("stream-cancel");
        std::fs::write(inside.join("out.pcraft"), b"original").unwrap();
        let cancelled = AtomicBool::new(false);
        let stop = || cancelled.load(Ordering::Relaxed);
        let control = photocraft_raster::Interrupt::cancel_only(&stop);
        let result = workspace.write_protected_stream_with_interrupt("out.pcraft", &[], &[], &control, |file| {
            file.write_all(b"private").unwrap();
            cancelled.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(matches!(result, Err(AutomationError::Engine(photocraft_engine::EngineError::Cancelled))));
        cancelled.store(false, Ordering::Relaxed);
        let result: Result<(), AutomationError> = workspace.write_protected_stream_with_interrupt("out.pcraft", &[], &[], &control, |file| {
            file.write_all(b"partial bounded serializer output").unwrap();
            Err(AutomationError::BadRequest("synthetic serializer cap refusal".into()))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(inside.join("out.pcraft")).unwrap(), b"original");
        assert!(temp_files(&inside).is_empty());
    }
    #[test]
    fn alias_introduced_during_streaming_is_refused_at_final_guard() {
        use std::io::Write;
        let (inside, _, workspace) = roots("stream-source-alias");
        std::fs::write(inside.join("source.bin"), b"protected original").unwrap();
        std::fs::write(inside.join("out.pcraft"), b"unrelated old destination").unwrap();
        let result = workspace.write_protected_stream_with_interrupt("out.pcraft", &["source.bin".into()], &[], &photocraft_raster::Interrupt::NONE, |file| {
            file.write_all(b"private archive").unwrap();
            std::fs::remove_file(inside.join("out.pcraft")).unwrap();
            std::fs::hard_link(inside.join("source.bin"), inside.join("out.pcraft")).unwrap();
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(inside.join("source.bin")).unwrap(), b"protected original");
        assert_eq!(std::fs::read(inside.join("out.pcraft")).unwrap(), b"protected original");
        assert!(temp_files(&inside).is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn retargeted_parent_during_streaming_is_refused_using_held_root_identity() {
        use std::io::Write;
        let (inside, _, workspace) = roots("stream-parent-change");
        std::fs::create_dir(inside.join("sub")).unwrap();
        std::fs::write(inside.join("sub/out.pcraft"), b"original").unwrap();
        let result = workspace.write_protected_stream_with_interrupt("sub/out.pcraft", &[], &[], &photocraft_raster::Interrupt::NONE, |file| {
            file.write_all(b"private archive").unwrap();
            std::fs::rename(inside.join("sub"), inside.join("old-sub")).unwrap();
            std::fs::create_dir(inside.join("sub")).unwrap();
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(inside.join("old-sub/out.pcraft")).unwrap(), b"original");
        assert!(!inside.join("sub/out.pcraft").exists());
        assert!(temp_files(&inside.join("old-sub")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn write_into_read_only_folder_keeps_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let (inside, _, workspace) = roots("readonly");
        let ro = inside.join("ro");
        std::fs::create_dir_all(&ro).unwrap();
        std::fs::write(ro.join("doc.psd"), b"original").unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let root_user = std::fs::File::create(ro.join("probe")).is_ok();
        if !root_user {
            assert!(workspace.write("ro/doc.psd", b"new").is_err());
            assert_eq!(std::fs::read(ro.join("doc.psd")).unwrap(), b"original");
            assert!(temp_files(&ro).is_empty());
        }
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn rejects_absolute_traversal_mixed_prefix_and_malformed_paths_without_panicking() {
        let (_, _, workspace) = roots("reject");
        for path in [
            "",
            "/absolute",
            "../escape",
            "a/../escape",
            "a\\..\\escape",
            "a\\b",
            "C:/escape",
            "a//b",
            "./a",
            "name:stream",
            "NUL",
            "con.txt",
            "folder/COM1.log",
        ] {
            assert!(workspace.read(path).is_err(), "read accepted {path:?}");
            assert!(workspace.write(path, b"x").is_err(), "write accepted {path:?}");
        }
    }

    #[test]
    fn read_and_write_authority_are_separate() {
        let (inside, _, _) = roots("separate");
        std::fs::write(inside.join("read.txt"), b"canary").unwrap();
        let read_only = AuthorizedWorkspace::new(Some(&inside), None).unwrap();
        assert_eq!(read_only.read("read.txt").unwrap(), b"canary");
        assert!(read_only.write("blocked.txt", b"x").is_err());
        let write_only = AuthorizedWorkspace::new(None, Some(&inside)).unwrap();
        assert!(write_only.read("read.txt").is_err());
        write_only.write("written.txt", b"ok").unwrap();
    }

    #[test]
    fn missing_parent_is_a_stable_error_before_any_file_is_created() {
        let (inside, _, workspace) = roots("missing-parent");
        let error = workspace.write("missing/output.txt", b"x").unwrap_err().to_string();
        assert!(error.contains("automation write"), "{error}");
        assert!(!inside.join("missing").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;

        let (inside, outside, workspace) = roots("symlink");
        std::fs::write(outside.join("secret.txt"), b"outside").unwrap();
        symlink(&outside, inside.join("link")).unwrap();
        assert!(workspace.read("link/secret.txt").is_err());
        assert!(workspace.write("link/new.txt", b"blocked").is_err());
        assert!(!outside.join("new.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_symlink_escape_is_rejected_when_supported() {
        use std::os::windows::fs::symlink_dir;

        let (inside, outside, workspace) = roots("windows-link");
        std::fs::write(outside.join("secret.txt"), b"outside").unwrap();
        if symlink_dir(&outside, inside.join("link")).is_err() {
            return;
        }
        assert!(workspace.read("link/secret.txt").is_err());
        assert!(workspace.write("link/new.txt", b"blocked").is_err());
        assert!(!outside.join("new.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_junction_escape_is_rejected_when_supported() {
        use std::process::Command;

        let (inside, outside, workspace) = roots("windows-junction");
        std::fs::write(outside.join("secret.txt"), b"outside").unwrap();
        let link = inside.join("junction");
        let status = Command::new("cmd.exe").args(["/D", "/C", "mklink", "/J"]).arg(&link).arg(&outside).status();
        if !status.is_ok_and(|status| status.success()) {
            return;
        }
        assert!(workspace.read("junction/secret.txt").is_err());
        assert!(workspace.write("junction/new.txt", b"blocked").is_err());
        assert!(!outside.join("new.txt").exists());
        let _ = std::fs::remove_dir(link);
    }

    #[test]
    fn filesystem_commands_fail_closed() {
        for id in [
            "file.open",
            "file.openAs",
            "file.save",
            "file.saveAs",
            "file.saveACopy",
            "file.export.saveForWebLegacy",
            "pattern.import",
            "layer.smartObjects.exportContents",
            "layer.smartObjects.replaceContents",
            "layer.smartObjects.relinkToFile",
            "layer.smartObjects.convertToLinked",
            "measurementLog.export",
            "layer.videoLayers.reloadFrame",
            "edit.colorSettings",
        ] {
            assert!(authorize_engine_command(id, &serde_json::json!({})).is_err());
        }
        assert!(authorize_engine_command("file.new", &serde_json::json!({})).is_ok());
        assert!(authorize_engine_command("file.automate.cropAndStraightenPhotos", &serde_json::json!({})).is_ok());
        assert!(authorize_desktop_engine_command("file.automate.cropAndStraightenPhotos", &serde_json::json!({})).is_ok());
        for id in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers", "layer.smartObjects.saveContents"] {
            assert!(authorize_desktop_engine_command(id, &serde_json::json!({"layer": 19})).is_ok(), "{id}");
        }
        for id in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers"] {
            assert!(authorize_engine_command(id, &serde_json::json!({"path": "/outside/source.psb"})).is_err(), "{id}");
        }
        // The UI-level examples in docs/control-protocol.md.
        for id in ["view.zoomIn", "window.theme.pro", "edit.search"] {
            assert!(authorize_desktop_engine_command(id, &serde_json::json!({})).is_ok(), "{id}");
        }
        assert!(authorize_engine_command("image.mode.cmyk", &serde_json::json!({})).is_ok());
        assert!(authorize_desktop_engine_command("image.mode.cmyk", &serde_json::json!({})).is_err());
        assert!(authorize_engine_command("image.mode.rgb", &serde_json::json!({"profile": "/outside/profile.icc"})).is_err());
        assert!(authorize_engine_command("image.mode.rgb", &serde_json::json!({"profile": "srgb"})).is_ok());
        assert!(authorize_engine_command("filter.distort.displace", &serde_json::json!({"mapPath": "outside.png"})).is_err());
        assert!(authorize_engine_command("layer.setAdjustment", &serde_json::json!({"file": "outside.cube"})).is_err());
        assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": "colorSettings.workingRgb", "value": "outside.icc"})).is_err());
        assert!(authorize_engine_step("file.open", &serde_json::json!({})).is_err());
        assert!(authorize_engine_step("actions.play", &serde_json::json!({})).is_ok());
        // An allowed command can't reach a denied one by running it on its own behalf.
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width": 4, "height": 4})).unwrap();
        session.authorize = Some(authorize_desktop_engine_step);
        let params = serde_json::json!({"to": "grayscale"});
        assert!(authorize_desktop_engine_command("file.automate.conditionalModeChange", &params).is_ok());
        assert!(session.execute("file.automate.conditionalModeChange", params).is_err());
        assert_eq!(session.active().unwrap().doc.mode, photocraft_engine::doc::ColorMode::Rgb);
        assert!(authorize_desktop_engine_step("file.saveACopy", &serde_json::json!({})).is_err());
    }

    #[test]
    fn apply_data_set_is_judged_by_the_values_it_applies() {
        let mut headless = crate::Headless::new();
        headless.command_run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        let layer = headless.command_run("layer.new.layer", serde_json::json!({})).unwrap()["layer"].clone();
        let defs = [
            serde_json::json!({"name": "shown", "layer": layer, "type": "visibility"}),
            serde_json::json!({"name": "photo", "layer": layer, "type": "pixelReplacement"}),
        ];
        headless.command_run("image.variables.define", serde_json::json!({"defs": defs})).unwrap();
        let sets = serde_json::json!({"dataSets": [
            {"name": "hidden", "values": [{"variable": "shown", "kind": "visibility", "value": false}]},
            {"name": "swap", "values": [
                {"variable": "shown", "kind": "visibility", "value": false},
                {"variable": "photo", "kind": "pixels", "value": "/outside/photo.png"},
            ]},
        ]});
        headless.command_run("image.variables.dataSets", sets).unwrap();
        let visible = |headless: &crate::Headless| headless.session.active().unwrap().doc.layers.iter().all(|l| l.visible);
        let refused = headless.command_run("image.applyDataSet", serde_json::json!({"name": "swap"})).unwrap_err();
        assert!(refused.to_string().contains("ambient filesystem paths"), "{refused}");
        assert!(visible(&headless), "a refused data set applies none of its values");
        assert!(headless.command_run("image.applyDataSet", serde_json::json!({"name": "hidden"})).is_ok());
        assert!(!visible(&headless));
    }

    #[test]
    fn automation_rejects_ambient_path_commands_and_preferences() {
        for (id, params) in [
            ("brush.presets.importAbr", serde_json::json!({"path": "/outside/set.abr"})),
            ("gradient.presets.importGrd", serde_json::json!({"path": "/outside/set.grd"})),
            ("plugin.install", serde_json::json!({"path": "/outside/plugin.wasm"})),
            ("plugin.reload", serde_json::json!({"path": "/outside/plugins"})),
            ("plugin.reload", serde_json::json!({})),
            ("plugin.install", serde_json::json!({"path": " "})),
            ("swatches.import", serde_json::json!({"path": "/outside/set.aco"})),
            ("swatches.export", serde_json::json!({"path": "/outside/set.ase"})),
            ("actions.load", serde_json::json!({"path": "/outside/set.atn"})),
            ("actions.save", serde_json::json!({"path": "/outside/set.atn"})),
        ] {
            assert!(authorize_engine_command(id, &params).is_err(), "{id}: {params}");
        }
        for (id, params) in [
            ("brush.presets.importAbr", serde_json::json!({"data": "QUJD"})),
            ("gradient.presets.importGrd", serde_json::json!({"data": "QUJD"})),
            ("plugin.install", serde_json::json!({"data": "QUJD"})),
            ("swatches.import", serde_json::json!({"data": "QUJD"})),
            ("swatches.export", serde_json::json!({"format": "ase"})),
            ("actions.load", serde_json::json!({"data": "QUJD"})),
            ("actions.save", serde_json::json!({"set": "S"})),
        ] {
            assert!(authorize_engine_command(id, &params).is_ok(), "{id}: {params}");
        }
        for path in [
            "",
            "colorSettings",
            "colorSettings.workingRgb",
            "scriptEvents",
            "scriptEvents.enabled",
            ".scriptEvents",
            ".scriptEvents.enabled",
            "historyLog.filePath",
            ".historyLog.filePath",
            "plugIns.additionalPluginsFolder",
            "scratchDisks.disks",
            ".",
            "..historyLog.filePath",
        ] {
            assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": path, "value": {}})).is_err(), "{path}");
        }
        assert!(
            authorize_engine_command("prefs.set", &serde_json::json!({"values": {"interface.language": "fr", "historyLog.filePath": "/outside/log"}})).is_err()
        );
        assert!(authorize_engine_command("prefs.set", &serde_json::json!({"path": "interface.language", "value": "fr"})).is_ok());
    }

    #[test]
    fn registry_filesystem_path_params_are_classified() {
        let probe = serde_json::json!({
            "path": "/outside/photocraft-probe",
            "file": "/outside/photocraft-probe.icc",
            "mapPath": "/outside/photocraft-probe.png",
            "profile": "/outside/photocraft-probe.icc",
            "workingRgb": "/outside/photocraft-probe.icc",
            "workingCmyk": "/outside/photocraft-probe.icc",
            "workingGray": "/outside/photocraft-probe.icc",
            "input": "/outside/photocraft-input",
            "output": "/outside/photocraft-output",
            "paths": ["/outside/photocraft-probe.psd"],
        });
        let preference_probe = serde_json::json!({"path": ".scriptEvents", "value": {}});
        let unclassified: Vec<_> = photocraft_engine::command_specs()
            .iter()
            .filter(|spec| documents_filesystem_path_params(spec.id, spec.params))
            .filter(|spec| {
                let params = if spec.id == "prefs.set" { &preference_probe } else { &probe };
                authorize_engine_command(spec.id, params).is_ok()
            })
            .map(|spec| spec.id)
            .collect();
        assert!(unclassified.is_empty(), "filesystem path parameters in the command registry need an automation policy: {unclassified:?}");
    }

    #[test]
    fn pending_import_cancel_is_available_without_granting_file_access() {
        let mut headless=crate::Headless::new();
        assert_eq!(headless.command_run("file.open.discardPending",serde_json::json!({})).unwrap()["discarded"],0);
        assert!(headless.command_run("file.open.discardPending",serde_json::json!({"path":"/outside/source.xcf"})).is_err());
        assert!(headless.command_run("file.open.discardPending",serde_json::json!({"heldId":"0"})).is_err());
        assert!(headless.command_run("file.open",serde_json::json!({"path":"/outside/source.xcf"})).is_err());
        assert!(headless.session.documents().is_empty());
    }

    fn documents_filesystem_path_params(id: &str, params: &str) -> bool {
        if matches!(id, "prefs.get" | "prefs.reset") {
            return false;
        }
        // These registry descriptions refer to document vector paths, not host files.
        if matches!(
            id,
            "filter.blurGallery.pathBlur"
                | "filter.render.flame"
                | "shape.create"
                | "shape.edit"
                | "shape.info"
                | "shape.presets.list"
                | "shape.presets.new"
                | "path.list"
                | "path.info"
                | "path.set"
                | "path.transform"
                | "path.moveAnchors"
                | "path.moveHandle"
                | "path.bendSegment"
                | "path.convertPoint"
                | "path.addAnchor"
                | "path.deleteAnchor"
                | "path.clippingPath.set"
                | "path.rename"
                | "path.delete"
                | "path.toSelection"
                | "path.fill"
                | "path.stroke"
                | "type.attachToPath"
                | "type.detachFromPath"
                | "xcf.itemSet.setMembers"
                | "xcf.vector.stroke.width.set"
                | "select.toWorkPath"
                | "layer.vectorMask.add"
                | "layer.vectorMask.edit"
                | "layer.vectorMask.info"
                | "paint.symmetryFromPath"
                | "edit.defineCustomShape"
                | "layer.combineShapes.unite"
                | "layer.combineShapes.subtractFrontShape"
                | "layer.combineShapes.intersectShapeAreas"
                | "layer.combineShapes.excludeOverlappingShapes"
                | "layer.combineShapes.mergeShapeComponents"
        ) || id.starts_with("layer.newFillLayer.")
            || id.starts_with("layer.newAdjustmentLayer.")
        {
            // New fill and adjustment layers take a vector path as their vector mask (#1419).
            return false;
        }
        params.to_ascii_lowercase().contains("path")
    }

    #[test]
    fn whole_preferences_update_cannot_enable_automation_script_events() {
        let mut headless = crate::headless::Headless::new();
        let result = headless.command_run(
            "prefs.set",
            serde_json::json!({
                "path": "",
                "value": {
                    "scriptEvents": {
                        "enabled": true,
                        "bindings": [{
                            "event": "newDocument",
                            "steps": [["file.saveACopy", {"path": "/outside/canary.psd"}]]
                        }]
                    }
                }
            }),
        );
        assert!(result.is_err());

        for (id, params) in [
            ("brush.presets.importAbr", serde_json::json!({"path": "/outside/set.abr"})),
            ("gradient.presets.importGrd", serde_json::json!({"path": "/outside/set.grd"})),
            ("plugin.install", serde_json::json!({"path": "/outside/plugin.wasm"})),
            ("plugin.reload", serde_json::json!({"path": "/outside/plugins"})),
            ("prefs.set", serde_json::json!({"path": "historyLog.filePath", "value": "/outside/log"})),
            ("prefs.set", serde_json::json!({"values": {"interface.language": "fr", "historyLog.filePath": "/outside/log"}})),
        ] {
            assert!(headless.command_run(id, params).is_err(), "{id}");
        }
        headless.command_run("file.new", serde_json::json!({"width": 5, "height": 5})).unwrap();
        assert!(headless.session.file_menu.event_log.is_empty());
    }

    /// #2176: an absolute path beneath the root names the same file as its relative path; every
    /// other absolute path is refused before any file effect.
    #[test]
    fn absolute_paths_beneath_the_root_are_translated_and_others_refused() {
        let (inside, outside, workspace) = roots("absolute-paths");
        let abs = |p: &Path| p.to_string_lossy().into_owned();

        workspace.write(&abs(&inside.join("a.bin")), b"one").unwrap();
        assert_eq!(std::fs::read(inside.join("a.bin")).unwrap(), b"one");
        assert_eq!(workspace.read(&abs(&inside.join("a.bin"))).unwrap(), b"one");
        // Forward slashes too (on Windows the native spelling uses `\`).
        assert_eq!(workspace.read(&abs(&inside.join("a.bin")).replace('\\', "/")).unwrap(), b"one");
        std::fs::create_dir(inside.join("sub")).unwrap();
        workspace.write(&abs(&inside.join("sub").join("b.bin")), b"two").unwrap();
        assert_eq!(std::fs::read(inside.join("sub/b.bin")).unwrap(), b"two");
        assert_eq!(workspace.read("sub/b.bin").unwrap(), b"two", "relative paths still work");

        let refused = |path: String| {
            let error = workspace.write(&path, b"x").unwrap_err().to_string();
            assert!(error.contains("automation path rejected"), "{path}: {error}");
        };
        refused(abs(&outside.join("escape.bin")));
        refused(abs(&inside.join("..").join("outside").join("traversal.bin")));
        refused(abs(&inside));
        refused(abs(&inside.join("CON")));
        refused(format!("{}:hidden", abs(&inside.join("a.bin"))));
        // A sibling whose name merely starts with the root's name is not beneath it.
        let sibling = inside.with_file_name("inside-sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        refused(abs(&sibling.join("lookalike.bin")));
        refused(format!("{}-sibling/lookalike.bin", abs(&inside)));
        // Climbing back out through a subdirectory is refused too.
        refused(abs(&inside.join("sub").join("..").join("..").join("outside").join("climb.bin")));
        assert!(!sibling.join("lookalike.bin").exists());
        assert!(!outside.join("climb.bin").exists());
        assert!(!outside.join("escape.bin").exists());
        assert!(!outside.join("traversal.bin").exists());
        assert_eq!(std::fs::read(inside.join("a.bin")).unwrap(), b"one");
    }

    /// Each authority is checked against its own root, and none granted refuses absolute paths.
    #[test]
    fn absolute_paths_are_checked_against_the_authority_s_own_root() {
        let (inside, outside, _) = roots("absolute-authority");
        let workspace = AuthorizedWorkspace::new(Some(&outside), Some(&inside)).unwrap();
        let target = inside.join("written.bin");
        workspace.write(&target.to_string_lossy(), b"w").unwrap();
        assert!(workspace.read(&target.to_string_lossy()).is_err(), "not under the read root");
        let none = AuthorizedWorkspace::default();
        assert!(none.read(&target.to_string_lossy()).is_err());
    }
}
