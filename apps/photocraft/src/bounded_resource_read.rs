//! Dedicated selected resource reads. Generic document Open/Save is unchanged.
use std::io::{self, Read};
use std::path::Path;

pub(crate) const MAX_BYTES: usize = 8 << 20;
const SCRATCH_BYTES: usize = 64 << 10;
const MAX_NAME_BYTES: usize = 2048;

#[derive(Debug)]
pub(crate) struct SelectedBytes {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum Error {
    Io(io::Error),
    Root(photocraft_automation::AutomationError),
    InvalidPath,
    NotRegular,
    TooLarge,
    Allocation,
    Changed,
    InvalidReadCount,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(_) => f.write_str("xcf.resource.read-failed"),
            Self::Root(error) => f.write_str(root_error_code(error)),
            Self::InvalidPath => f.write_str("xcf.resource.invalid-path"),
            Self::NotRegular => f.write_str("xcf.resource.not-regular"),
            Self::TooLarge => f.write_str("xcf.resource.too-large"),
            Self::Allocation => f.write_str("xcf.resource.allocation"),
            Self::Changed => f.write_str("xcf.resource.changed"),
            Self::InvalidReadCount => f.write_str("xcf.resource.invalid-read-count"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Root(error) => Some(error),
            _ => None,
        }
    }
}

fn root_error_code(error: &photocraft_automation::AutomationError) -> &'static str {
    use photocraft_automation::AutomationError;
    match error {
        AutomationError::BadRequest(message) => match message.as_str() {
            "xcf.link.substituteFromRoot.regular-file" => "xcf.resource.not-regular",
            "xcf.link.substituteFromRoot.limit" => "xcf.resource.too-large",
            "xcf.link.substituteFromRoot.allocation" => "xcf.resource.allocation",
            "xcf.link.substituteFromRoot.changed-length" => "xcf.resource.changed",
            "xcf.link.substituteFromRoot.read-count" => "xcf.resource.invalid-read-count",
            "xcf.link.substituteFromRoot.path-limit" => "xcf.resource.invalid-path",
            value if value.starts_with("automation filesystem access is not granted") => "xcf.resource.read-root-unavailable",
            value if value.starts_with("automation path rejected:") => "xcf.resource.invalid-path",
            _ => "xcf.resource.read-failed",
        },
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled) => "xcf.resource.read-cancelled",
        _ => "xcf.resource.read-failed",
    }
}

/// Copies only a capped explicit request; it never obtains a stored locator or a new grant.
pub(crate) fn rooted_request_path(path: &str) -> Result<String, Error> {
    if path.is_empty() || path.len() > 4096 {
        return Err(Error::InvalidPath);
    }
    let mut owned = String::new();
    owned.try_reserve_exact(path.len()).map_err(|_| Error::Allocation)?;
    owned.push_str(path);
    Ok(owned)
}

/// Uses only the held launch-time capability and the dedicated same-handle bounded reader.
pub(crate) fn read_rooted(workspace: &photocraft_automation::AuthorizedWorkspace, path: &str) -> Result<SelectedBytes, Error> {
    let bytes = workspace.read_xcf_linked_replacement(path, &photocraft_raster::Interrupt::NONE).map_err(Error::Root)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let name = bounded_name("selected-resource.png")?;
    Ok(SelectedBytes { name, bytes })
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) fn read_selection(path: Option<&Path>) -> Result<Option<SelectedBytes>, Error> {
    let Some(path) = path else { return Ok(None) };
    let mut file = std::fs::File::open(path)?;
    let before = file.metadata()?;
    let bytes = read_opened(&mut file, &before)?;
    // The opened handle scopes the read; unchanged length/timestamp is not a cryptographic snapshot.
    let label = path.file_name().map(|name| name.to_string_lossy());
    let name = bounded_name(label.as_deref().unwrap_or("resource"))?;
    Ok(Some(SelectedBytes { name, bytes }))
}

fn read_opened(file: &mut std::fs::File, before: &std::fs::Metadata) -> Result<Vec<u8>, Error> {
    if !before.is_file() {
        return Err(Error::NotRegular);
    }
    if before.len() > MAX_BYTES as u64 {
        return Err(Error::TooLarge);
    }
    let bytes = read_bounded(file, before.len())?;
    let after = file.metadata()?;
    if !after.is_file() || after.len() != before.len() || bytes.len() as u64 != before.len() {
        return Err(Error::Changed);
    }
    if let (Ok(a), Ok(b)) = (before.modified(), after.modified())
        && a != b
    {
        return Err(Error::Changed);
    }
    Ok(bytes)
}

fn bounded_name(value: &str) -> Result<String, Error> {
    let mut name = String::new();
    name.try_reserve_exact(value.len().min(MAX_NAME_BYTES)).map_err(|_| Error::Allocation)?;
    for character in value.chars() {
        let Some(length) = name.len().checked_add(character.len_utf8()) else { return Err(Error::Allocation) };
        if length > MAX_NAME_BYTES {
            break;
        }
        name.push(character);
    }
    Ok(name)
}

fn read_bounded(reader: &mut impl Read, size_hint: u64) -> Result<Vec<u8>, Error> {
    if size_hint > MAX_BYTES as u64 {
        return Err(Error::TooLarge);
    }
    let reserve = usize::try_from(size_hint).map_err(|_| Error::TooLarge)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(reserve).map_err(|_| Error::Allocation)?;
    let limit = (MAX_BYTES as u64).checked_add(1).ok_or(Error::TooLarge)?;
    let mut bounded = reader.take(limit);
    let mut scratch = [0u8; SCRATCH_BYTES];
    loop {
        let count = match bounded.read(&mut scratch) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(bytes);
        }
        let chunk = scratch.get(..count).ok_or(Error::InvalidReadCount)?;
        let total = bytes.len().checked_add(count).ok_or(Error::TooLarge)?;
        if total > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        bytes.try_reserve(count).map_err(|_| Error::Allocation)?;
        bytes.extend_from_slice(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Generated {
        remaining: usize,
        read: usize,
        interrupted: bool,
        fail_after: Option<usize>,
    }
    impl Read for Generated {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.interrupted {
                self.interrupted = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.fail_after.is_some_and(|n| self.read >= n) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let count = out.len().min(self.remaining);
            out.get_mut(..count).ok_or(io::ErrorKind::InvalidData)?.fill(91);
            self.read += count;
            self.remaining -= count;
            Ok(count)
        }
    }
    fn generated(n: usize) -> Generated {
        Generated { remaining: n, read: 0, interrupted: false, fail_after: None }
    }
    #[test]
    fn cancelled_does_not_open_a_path() {
        assert!(read_selection(None).unwrap().is_none());
    }
    #[test]
    fn exact_cap_is_accepted_with_original_bytes() {
        let mut input = generated(MAX_BYTES);
        let output = read_bounded(&mut input, MAX_BYTES as u64).unwrap();
        assert_eq!(output.len(), MAX_BYTES);
        assert!(output.iter().all(|v| *v == 91));
        assert_eq!(input.read, MAX_BYTES);
    }
    #[test]
    fn growth_after_small_hint_is_total_cap_plus_one_bounded() {
        let mut input = generated(MAX_BYTES + 17);
        assert!(matches!(read_bounded(&mut input, 1), Err(Error::TooLarge)));
        assert_eq!(input.read, MAX_BYTES + 1);
        assert_eq!(input.remaining, 16);
    }
    #[test]
    fn oversized_metadata_starts_no_read() {
        let mut input = generated(1);
        assert!(matches!(read_bounded(&mut input, MAX_BYTES as u64 + 1), Err(Error::TooLarge)));
        assert_eq!(input.read, 0);
    }
    #[test]
    fn empty_and_short_reader_keep_exact_data_without_padding() {
        assert!(read_bounded(&mut generated(0), 0).unwrap().is_empty());
        assert_eq!(read_bounded(&mut generated(3), 8).unwrap(), [91, 91, 91]);
    }
    #[test]
    fn interrupted_is_retried_and_read_error_is_named() {
        let mut input = generated(5);
        input.interrupted = true;
        assert_eq!(read_bounded(&mut input, 5).unwrap(), [91; 5]);
        let mut input = generated(SCRATCH_BYTES * 2);
        input.fail_after = Some(SCRATCH_BYTES);
        assert!(matches!(read_bounded(&mut input, 0), Err(Error::Io(_))));
        assert_eq!(input.read, SCRATCH_BYTES);
    }
    #[test]
    fn regular_file_roundtrip_and_missing_path() {
        let path = std::env::temp_dir().join(format!("photocraft-bounded-resource-{}-roundtrip.bin", std::process::id()));
        let data = [0, 255, 3, 0, 92];
        std::fs::write(&path, data).unwrap();
        let got = read_selection(Some(&path)).unwrap().unwrap();
        assert_eq!(got.bytes, data);
        assert!(!got.name.is_empty());
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(read_selection(Some(&path)), Err(Error::Io(_))));
    }
    #[test]
    fn directory_is_refused_and_unicode_name_is_bounded() {
        // Windows refuses opening a directory as a file before metadata can classify it.
        assert!(matches!(read_selection(Some(&std::env::temp_dir())), Err(Error::NotRegular | Error::Io(_))));
        let name = bounded_name(&"🖌".repeat(1000)).unwrap();
        assert_eq!(name.len(), MAX_NAME_BYTES);
        assert_eq!(name.chars().count(), MAX_NAME_BYTES / 4);
    }
    #[test]
    fn changed_opened_file_length_is_refused_after_read() {
        use std::io::Write;
        for (label, replacement) in [("truncated", &b"x"[..]), ("grown", &b"longer"[..])] {
            let path = std::env::temp_dir().join(format!("photocraft-bounded-resource-{}-{label}.bin", std::process::id()));
            let mut source = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();
            source.write_all(b"old").unwrap();
            drop(source);
            let mut source = std::fs::File::open(&path).unwrap();
            let before = source.metadata().unwrap();
            std::fs::write(&path, replacement).unwrap();
            assert!(matches!(read_opened(&mut source, &before), Err(Error::Changed)));
            drop(source);
            std::fs::remove_file(path).unwrap();
        }
    }
}
