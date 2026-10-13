//! Manifest-last native directory writes through a held caller-authorized root.
//! Existing identical native entries are reused; no files are garbage-collected.
use super::{AutomationError, Path, poll};
use photocraft_engine::WriteProtection;
use photocraft_format::{DirectoryEntry, DirectoryEntryDisposition, DirectoryEntryKind, DirectoryEntrySink, FormatError};
use photocraft_raster::Interrupt;
use std::{io::Read, path::PathBuf};

fn failed(message: impl Into<String>) -> AutomationError {
    AutomationError::BadRequest(message.into())
}
fn format_error(error: AutomationError) -> FormatError {
    if matches!(error, AutomationError::Engine(photocraft_engine::EngineError::Cancelled)) {
        FormatError::Cancelled
    } else {
        FormatError::Io(std::io::Error::other(error.to_string()))
    }
}
fn automation_error(error: FormatError) -> AutomationError {
    if matches!(error, FormatError::Cancelled) { AutomationError::Engine(photocraft_engine::EngineError::Cancelled) } else { AutomationError::Format(error) }
}
pub(super) struct Sink<'a> {
    selected: PathBuf,
    resolved: PathBuf,
    root: cap_std::fs::Dir,
    protection: &'a WriteProtection,
}
impl<'a> Sink<'a> {
    pub(super) fn new(path: &Path, protection: &'a WriteProtection, control: &Interrupt<'_>) -> Result<Self, AutomationError> {
        poll(control)?;
        if path.as_os_str().len() > 4096 {
            return Err(failed("native directory path limit"));
        }
        super::validate_destination_protected_with_interrupt(path, protection, None, control)?;
        let resolved = std::fs::canonicalize(path).map_err(|e| AutomationError::Io(format!("cannot resolve native directory: {e}")))?;
        if resolved.as_os_str().len() > 4096 {
            return Err(failed("resolved native directory path limit"));
        }
        let root = cap_std::fs::Dir::open_ambient_dir(&resolved, cap_std::ambient_authority())
            .map_err(|e| AutomationError::Io(format!("cannot hold native directory: {e}")))?;
        let sink = Self { selected: path.to_path_buf(), resolved, root, protection };
        sink.check_root(control)?;
        Ok(sink)
    }
    fn check_root(&self, control: &Interrupt<'_>) -> Result<(), AutomationError> {
        poll(control)?;
        if std::fs::canonicalize(&self.selected).map_err(|e| AutomationError::Io(format!("cannot recheck native directory: {e}")))? != self.resolved {
            return Err(failed("selected native directory changed during save"));
        }
        let current = cap_std::fs::Dir::open_ambient_dir(&self.resolved, cap_std::ambient_authority())
            .map_err(|e| AutomationError::Io(format!("cannot recheck native directory: {e}")))?;
        let held = self.root.try_clone().and_then(|dir| same_file::Handle::from_file(dir.into_std_file())).map_err(|e| AutomationError::Io(e.to_string()))?;
        let current = same_file::Handle::from_file(current.into_std_file()).map_err(|e| AutomationError::Io(e.to_string()))?;
        if held != current {
            return Err(failed("native directory identity changed during save"));
        }
        super::reject_named_source(&self.selected, self.protection, control)?;
        super::reject_named_source(&self.resolved, self.protection, control)?;
        poll(control)
    }
    fn name(entry: &DirectoryEntry<'_>) -> Result<(), AutomationError> {
        let hash = match entry.kind {
            DirectoryEntryKind::Tile => entry.name.strip_prefix("tiles/").and_then(|s| s.strip_suffix(".zst")),
            DirectoryEntryKind::Blob => entry.name.strip_prefix("blobs/").and_then(|s| s.strip_suffix(".zst")),
            DirectoryEntryKind::Preview if matches!(entry.name, "thumb.png" | "composite/preview.png") => return Ok(()),
            DirectoryEntryKind::Manifest if entry.name == "manifest.json" => return Ok(()),
            _ => return Err(failed("native directory entry name/kind mismatch")),
        }
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .ok_or_else(|| failed("native directory content name is invalid"))?;
        for (pair, expected) in hash.as_bytes().chunks_exact(2).zip(entry.decoded_hash) {
            let nibble = |byte: u8| if byte.is_ascii_digit() { byte - b'0' } else { byte - b'a' + 10 };
            let first = *pair.first().ok_or_else(|| failed("native directory content hash width"))?;
            let second = *pair.get(1).ok_or_else(|| failed("native directory content hash width"))?;
            if (nibble(first) << 4) | nibble(second) != expected {
                return Err(failed("native directory content identity mismatch"));
            }
        }
        Ok(())
    }
    fn publish(&self, entry: DirectoryEntry<'_>, input: &mut dyn Read, control: &Interrupt<'_>) -> Result<DirectoryEntryDisposition, AutomationError> {
        Self::name(&entry)?;
        self.check_root(control)?;
        match self.root.open(entry.name) {
            Ok(file) => {
                let mut file = file.into_std();
                let before = file.metadata().map_err(|e| AutomationError::Io(e.to_string()))?;
                if !before.is_file() {
                    return Err(failed("native directory entry is not a regular file"));
                }
                let original = same_file::Handle::from_file(file.try_clone().map_err(|e| AutomationError::Io(e.to_string()))?)
                    .map_err(|e| AutomationError::Io(e.to_string()))?;
                let identical = photocraft_format::verify_existing_directory_entry(&entry, &mut file, control).map_err(automation_error)?;
                self.check_root(control)?;
                let current = self.root.open(entry.name).map_err(|e| AutomationError::Io(format!("native entry changed during reuse check: {e}")))?.into_std();
                let after = current.metadata().map_err(|e| AutomationError::Io(e.to_string()))?;
                let current = same_file::Handle::from_file(current).map_err(|e| AutomationError::Io(e.to_string()))?;
                if original != current || before.len() != after.len() || before.modified().ok() != after.modified().ok() {
                    return Err(failed("native directory entry changed during reuse check"));
                }
                poll(control)?;
                if identical {
                    return Ok(DirectoryEntryDisposition::ReusedIdentical);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AutomationError::Io(format!("cannot inspect native directory entry: {error}"))),
        }
        // Only fixed native parent names can be created; cap-std prevents a
        // symlink from turning this relative operation into an outside-root write.
        let parent = if let Some(parent) = Path::new(entry.name).parent().filter(|p| !p.as_os_str().is_empty()) {
            match self.root.create_dir(parent) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(AutomationError::Io(e.to_string())),
            }
            self.root.open_dir(parent).map_err(|e| AutomationError::Io(format!("cannot hold native entry parent: {e}")))?
        } else {
            self.root.try_clone().map_err(|e| AutomationError::Io(e.to_string()))?
        };
        self.check_root(control)?;
        let target = self.resolved.join(entry.name);
        let check = || {
            self.check_root(control)?;
            match self.root.symlink_metadata(entry.name) {
                Ok(metadata) if metadata.file_type().is_symlink() => Err(failed("different native directory entry is a symbolic link")),
                Ok(_) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(AutomationError::Io(error.to_string())),
            }
        };
        super::write_protected_stream_with_guard(&target, self.protection, control, &check, Some(&parent), |file| {
            photocraft_format::copy_directory_entry_stream(&entry, input, file, control).map_err(automation_error)
        })?;
        Ok(DirectoryEntryDisposition::Written)
    }
}
impl DirectoryEntrySink for Sink<'_> {
    fn resident_scratch_bytes(&self) -> u64 {
        (128 << 20) + (512 << 10)
    }
    fn publish_entry(
        &mut self,
        entry: DirectoryEntry<'_>,
        input: &mut dyn Read,
        control: &Interrupt<'_>,
    ) -> photocraft_format::Result<DirectoryEntryDisposition> {
        if entry.kind == DirectoryEntryKind::Manifest {
            return Err(FormatError::Unsupported("manifest requires the native commit callback".into()));
        }
        self.publish(entry, input, control).map_err(format_error)
    }
    fn commit_manifest(
        &mut self,
        entry: DirectoryEntry<'_>,
        input: &mut dyn Read,
        control: &Interrupt<'_>,
    ) -> photocraft_format::Result<DirectoryEntryDisposition> {
        if entry.kind != DirectoryEntryKind::Manifest {
            return Err(FormatError::Unsupported("native commit callback requires manifest".into()));
        }
        self.publish(entry, input, control).map_err(format_error)
    }
}
