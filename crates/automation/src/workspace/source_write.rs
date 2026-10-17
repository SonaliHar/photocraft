//! Held-parent destination protection and atomic replacement through capabilities.
use super::{AuthorizedWorkspace, DENIED, file_error, request_path};
use crate::AutomationError;
use cap_std::fs::{Dir, OpenOptions};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Borrowed runtime identities; these never expand directory authority.
#[derive(Clone, Copy)]
pub(super) struct Sources<'a> {
    one: Option<&'a str>,
    many: &'a [String],
}
impl<'a> Sources<'a> {
    pub(super) fn one(source: Option<&'a str>) -> Self {
        Self { one: source, many: &[] }
    }
    pub(super) fn many(sources: &'a [String]) -> Self {
        Self { one: None, many: sources }
    }
    fn paths(self) -> impl Iterator<Item = &'a str> {
        self.one.into_iter().chain(self.many.iter().map(String::as_str))
    }
    fn validate(self, workspace: &AuthorizedWorkspace, control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
        poll(control)?;
        if self.many.len() > 4096 {
            return Err(AutomationError::BadRequest("automation protected source count exceeds limit".into()));
        }
        for path in self.paths() {
            poll(control)?;
            if path.is_empty() || path.len() > 4096 {
                return Err(AutomationError::BadRequest("automation protected source path is empty or too long".into()));
            }
            request_path(path, workspace.read.as_ref())?;
        }
        Ok(())
    }
}

fn poll(control: &photocraft_raster::Interrupt<'_>) -> Result<(), AutomationError> {
    control.check().map_err(|_| protection_error(photocraft_io::source_protection::SourceProtectionError::Cancelled))
}

fn protection_error(error: photocraft_io::source_protection::SourceProtectionError) -> AutomationError {
    if matches!(error, photocraft_io::source_protection::SourceProtectionError::Cancelled) {
        return AutomationError::Engine(photocraft_engine::EngineError::Cancelled);
    }
    AutomationError::BadRequest(format!("automation source protection: {error}"))
}
struct Target<'a> {
    workspace: &'a AuthorizedWorkspace,
    parent: Dir,
    leaf: PathBuf,
    relative: PathBuf,
    request: &'a str,
}
impl<'a> Target<'a> {
    fn open(workspace: &'a AuthorizedWorkspace, request: &'a str) -> Result<Self, AutomationError> {
        if request.is_empty() || request.len() > 4096 {
            return Err(AutomationError::BadRequest("automation destination path is empty or too long".into()));
        }
        crate::files::validate_write_name(Path::new(request), None)?;
        if photocraft_io::is_xcf_filename(request) {
            return Err(AutomationError::BadRequest("XCF is import-only; choose a writable format and destination".into()));
        }
        let relative = request_path(request, workspace.write.as_ref())?;
        let root = workspace.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        let leaf = relative.file_name().ok_or_else(|| AutomationError::BadRequest("automation destination has no filename".into()))?.into();
        let parent_path = relative.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let parent = root.dir.open_dir(parent_path).map_err(|e| file_error("open write parent", request, e))?;
        Ok(Self { workspace, parent, leaf, relative, request })
    }
    fn check(
        &self,
        sources: Sources<'_>,
        fingerprints: &[String],
        control: &photocraft_raster::Interrupt<'_>,
        resident_limit: Option<u64>,
    ) -> Result<(), AutomationError> {
        poll(control)?;
        let root = self.workspace.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let parent_path = self.relative.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
            let current = root.dir.open_dir(parent_path).map_err(|error| file_error("recheck write parent", self.request, error))?;
            let held = same_file::Handle::from_file(
                self.parent.try_clone().map_err(|error| file_error("identify held write parent", self.request, error))?.into_std_file(),
            )
            .map_err(|error| file_error("identify held write parent", self.request, error))?;
            let current =
                same_file::Handle::from_file(current.into_std_file()).map_err(|error| file_error("identify current write parent", self.request, error))?;
            if held != current {
                return Err(AutomationError::BadRequest("automation destination parent changed during write".into()));
            }
        }
        for source in sources.paths() {
            poll(control)?;
            if let Some(read) = &self.workspace.read
                && read.identity == root.identity
                && request_path(source, self.workspace.read.as_ref())? == self.relative
            {
                return Err(AutomationError::BadRequest("the original XCF source is protected; choose a different destination".into()));
            }
        }
        poll(control)?;
        let opened = self.parent.open(&self.leaf);
        poll(control)?;
        match opened {
            Ok(mut file) => {
                let metadata = file.metadata().map_err(|e| file_error("inspect write", self.request, e))?;
                if !metadata.is_file() {
                    return Err(AutomationError::BadRequest("automation destination is not a regular file".into()));
                }
                #[cfg(not(target_arch = "wasm32"))]
                for source in sources.paths() {
                    poll(control)?;
                    let read = self
                        .workspace
                        .read
                        .as_ref()
                        .ok_or_else(|| AutomationError::BadRequest("cannot establish source identity without read authority".into()))?;
                    let original = read.dir.open(request_path(source, self.workspace.read.as_ref())?).map_err(|e| file_error("identify source", source, e))?;
                    let destination = file.try_clone().map_err(|e| file_error("identify destination", self.request, e))?;
                    let a = same_file::Handle::from_file(original.into_std()).map_err(|e| file_error("identify source", source, e))?;
                    let b = same_file::Handle::from_file(destination.into_std()).map_err(|e| file_error("identify destination", self.request, e))?;
                    if a == b {
                        return Err(AutomationError::BadRequest("an alias of the original XCF source is protected; choose a different destination".into()));
                    }
                }
                match resident_limit {
                    Some(limit) => photocraft_io::source_protection::check_reader_with_resident_limit(&mut file, metadata.len(), fingerprints, limit, control),
                    None => photocraft_io::source_protection::check_reader_with_interrupt(&mut file, metadata.len(), fingerprints, control),
                }
                .map_err(protection_error)?;
                if file.metadata().map_err(|e| file_error("inspect write", self.request, e))?.len() != metadata.len() {
                    return Err(AutomationError::BadRequest("automation destination changed during protection check".into()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(file_error("validate write", self.request, e)),
        }
        poll(control)?;
        Ok(())
    }
}

pub(super) fn check(workspace: &AuthorizedWorkspace, path: &str, sources: Sources<'_>, fingerprints: &[String]) -> Result<(), AutomationError> {
    check_with_interrupt(workspace, path, sources, fingerprints, &photocraft_raster::Interrupt::NONE)
}
pub(super) fn check_with_interrupt(
    workspace: &AuthorizedWorkspace,
    path: &str,
    sources: Sources<'_>,
    fingerprints: &[String],
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), AutomationError> {
    sources.validate(workspace, control)?;
    photocraft_io::source_protection::validate_fingerprints_with_interrupt(fingerprints, control).map_err(protection_error)?;
    poll(control)?;
    let target = Target::open(workspace, path)?;
    target.check(sources, fingerprints, control, None)?;
    poll(control)
}

pub(super) fn write(workspace: &AuthorizedWorkspace, path: &str, bytes: &[u8], sources: Sources<'_>, fingerprints: &[String]) -> Result<(), AutomationError> {
    write_with_interrupt(workspace, path, bytes, sources, fingerprints, &photocraft_raster::Interrupt::NONE)
}
pub(super) fn write_with_interrupt(
    workspace: &AuthorizedWorkspace,
    path: &str,
    bytes: &[u8],
    sources: Sources<'_>,
    fingerprints: &[String],
    control: &photocraft_raster::Interrupt<'_>,
) -> Result<(), AutomationError> {
    write_stream_with_interrupt(workspace, path, sources, fingerprints, control, |file| {
        for chunk in bytes.chunks(64 << 10) {
            poll(control)?;
            let written = file.write_all(chunk);
            poll(control)?;
            written.map_err(|error| file_error("write temporary", path, error))?;
        }
        Ok(())
    })
}

/// Caller prepares all bounded encoding state before entering this sink seam.
/// The callback can seek the owned temp; it cannot publish or select another path.
pub(super) fn write_stream_with_interrupt<R>(
    workspace: &AuthorizedWorkspace,
    path: &str,
    sources: Sources<'_>,
    fingerprints: &[String],
    control: &photocraft_raster::Interrupt<'_>,
    write: impl FnOnce(&mut cap_std::fs::File) -> Result<R, AutomationError>,
) -> Result<R, AutomationError> {
    write_stream_inner(workspace, path, sources, fingerprints, control, None, write)
}

/// Same held-parent transaction with a caller-paid, lower inspection ceiling.
pub(super) fn write_stream_with_resident_limit<R>(
    workspace: &AuthorizedWorkspace,
    path: &str,
    sources: Sources<'_>,
    fingerprints: &[String],
    control: &photocraft_raster::Interrupt<'_>,
    resident_limit: u64,
    write: impl FnOnce(&mut cap_std::fs::File) -> Result<R, AutomationError>,
) -> Result<R, AutomationError> {
    write_stream_inner(workspace, path, sources, fingerprints, control, Some(resident_limit), write)
}

fn write_stream_inner<R>(
    workspace: &AuthorizedWorkspace,
    path: &str,
    sources: Sources<'_>,
    fingerprints: &[String],
    control: &photocraft_raster::Interrupt<'_>,
    resident_limit: Option<u64>,
    write: impl FnOnce(&mut cap_std::fs::File) -> Result<R, AutomationError>,
) -> Result<R, AutomationError> {
    sources.validate(workspace, control)?;
    photocraft_io::source_protection::validate_fingerprints_with_interrupt(fingerprints, control).map_err(protection_error)?;
    poll(control)?;
    let target = Target::open(workspace, path)?;
    target.check(sources, fingerprints, control, resident_limit)?;
    let leaf = target.leaf.to_string_lossy();
    let tmp = PathBuf::from(photocraft_format::atomic::temp_name(&leaf));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    poll(control)?;
    // Only remove a temporary file after this operation actually created it.
    let mut file = target.parent.open_with(&tmp, &options).map_err(|error| file_error("create write temporary", path, error))?;
    let written = (|| {
        poll(control)?;
        let output = write(&mut file)?;
        poll(control)?;
        let synced = file.sync_all();
        poll(control)?;
        synced.map_err(|error| file_error("sync write temporary", path, error))?;
        Ok(output)
    })();
    // Close before Windows rename. Cancellation/errors clean only the owned temp.
    drop(file);
    let output = match written {
        Ok(output) => output,
        Err(error) => {
            let _ = target.parent.remove_file(&tmp);
            return Err(error);
        }
    };
    let mut protection_failure = None;
    let result = photocraft_format::atomic::retry_rename(photocraft_format::atomic::RenameRetry::platform(), || {
        // Cancellation and protection are rechecked beneath the same held parent
        // immediately before every attempt. A successful rename is the commit point:
        // later cancellation does not report failure for already-published bytes.
        if let Err(error) = target.check(sources, fingerprints, control, resident_limit).and_then(|()| poll(control)) {
            let message = error.to_string();
            protection_failure = Some(error);
            return Err(std::io::Error::other(message));
        }
        target.parent.rename(&tmp, &target.parent, &target.leaf)
    });
    if let Err(error) = result {
        let _ = target.parent.remove_file(&tmp);
        if let Some(failure) = protection_failure {
            return Err(failure);
        }
        return Err(file_error("write", path, error));
    }
    #[cfg(unix)]
    if let Ok(directory) = target.parent.try_clone() {
        let _ = directory.into_std_file().sync_all();
    }
    Ok(output)
}
