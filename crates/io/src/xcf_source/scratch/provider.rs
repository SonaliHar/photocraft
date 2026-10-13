//! Explicit service authority. This capability does not turn a locator into a grant.
use super::{OwnedScratchXcfSnapshot, SLICE, ScratchHandle, error};
use photocraft_xcf::{Budget, BudgetKind, ErrorKind, Reservation, Source, XcfError};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

const MAX_PATH: usize = 4096;
const MAX_CAPACITY: usize = 8192;
struct Locator<'b, 'a> {
    path: PathBuf,
    _lease: Reservation<'b, 'a>,
}

/// An explicitly granted private scratch directory, supplied by a trusted platform service.
///
/// The caller must authorize this directory independently of source metadata/resources or
/// serialized preferences, and keep its ancestor/root identity and private namespace stable.
/// Validation rejects observable links/bad paths, but portable pathname APIs do not establish
/// protection against namespace replacement. No grant is inferred or persisted. Construction
/// of the incoming PathBuf belongs to the service; its retained capacity is charged here.
/// No clone, path getter, extraction, or serialization is provided. Every capture uses the
/// same job Budget and borrows this capability until the snapshot is destroyed.
pub struct ServiceAuthorizedScratchDirectory<'b, 'a> {
    #[cfg(target_os = "linux")]
    directory: Option<File>,
    locator: Locator<'b, 'a>,
    budget: &'b Budget<'a>,
}

fn io_error(e: io::Error) -> XcfError {
    error(ErrorKind::Structure, &format!("scratch provider: {e}"))
}
fn bounded_path<'b, 'a>(size: usize, budget: &'b Budget<'a>) -> Result<Locator<'b, 'a>, XcfError> {
    let guard = size.checked_mul(2).ok_or_else(|| error(ErrorKind::Limit, "scratch path size overflow"))?;
    let lease = budget.reserve(BudgetKind::Resident, guard as u64, "scratch path allocation")?;
    let mut out = Locator { path: PathBuf::new(), _lease: lease };
    let mut text = OsString::new();
    text.try_reserve_exact(size).map_err(|_| error(ErrorKind::Allocation, "scratch path allocation failed"))?;
    out.path = PathBuf::from(text);
    if out.path.capacity() > guard {
        return Err(error(ErrorKind::Limit, "scratch path allocation capacity"));
    }
    out._lease.resize(out.path.capacity() as u64, "scratch actual path capacity")?;
    Ok(out)
}
// Bound pathname conversion staging at the std/OS boundary separately from the
// retained locator. This is not a process-RSS or kernel-allocation census.
fn path_call<'b, 'a>(path: &Path, budget: &'b Budget<'a>) -> Result<Reservation<'b, 'a>, XcfError> {
    let bytes = path
        .as_os_str()
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(512))
        .ok_or_else(|| error(ErrorKind::Limit, "scratch path call staging overflow"))?;
    budget.reserve(BudgetKind::Resident, bytes as u64, "scratch path call staging")
}
fn validate_root(path: &Path, budget: &Budget<'_>) -> Result<(), XcfError> {
    let mut current = bounded_path(path.as_os_str().len().checked_add(32).ok_or_else(|| error(ErrorKind::Limit, "scratch path length"))?, budget)?;
    let mut count = 0usize;
    for component in path.components() {
        count += 1;
        if count > 128 {
            return Err(error(ErrorKind::Limit, "scratch root component limit"));
        }
        budget.consume_work(1, "scratch root validation")?;
        match component {
            Component::ParentDir | Component::CurDir => return Err(error(ErrorKind::Structure, "scratch root must not traverse dot components")),
            #[cfg(windows)]
            Component::Prefix(prefix) => {
                if !matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)) {
                    return Err(error(ErrorKind::Unsupported, "scratch requires a local disk directory"));
                }
                current.path.push(component.as_os_str());
                continue;
            }
            _ => current.path.push(component.as_os_str()),
        }
        let _call = path_call(&current.path, budget)?;
        let metadata = std::fs::symlink_metadata(&current.path).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(error(ErrorKind::Structure, "scratch root contains a link or non-directory"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(error(ErrorKind::Structure, "scratch root contains a reparse point"));
            }
        }
        budget.poll("scratch root metadata completed")?;
    }
    Ok(())
}

impl<'b, 'a> ServiceAuthorizedScratchDirectory<'b, 'a> {
    /// Creates an exclusive generated destination. Writes reserve persistent
    /// Scratch growth; no source-derived path or writable handle escapes.
    #[cfg(any(windows, target_os = "linux"))]
    pub fn create_generated_archive_destination<'p, 'c>(
        &self,
        max_bytes: u64,
        quota: &photocraft_raster::backing::quota::BackingQuota,
        control: &'p photocraft_raster::backing::ReadControl<'c>,
    ) -> Result<crate::native_plane_store::generated::GeneratedArchiveDestination<'p, 'c>, photocraft_raster::backing::BackingError> {
        use crate::native_plane_store::generated::GeneratedArchiveDestination;
        use photocraft_raster::backing::{BackingError, quota::QuotaKind};
        control.check(1)?;
        if max_bytes == 0
            || max_bytes > u64::from(crate::native_plane_store::NATIVE_STORE_MAX_BLOCKS) * crate::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64
        {
            return Err(BackingError::Bounds);
        }
        let lease = quota.reserve(QuotaKind::Resident, GeneratedArchiveDestination::handle_envelope()).map_err(|_| BackingError::Quota)?;
        let scratch = quota.reserve(QuotaKind::Scratch, 0).map_err(|_| BackingError::Quota)?;
        #[cfg(windows)]
        let opened = self.create_windows();
        #[cfg(target_os = "linux")]
        let opened: Result<File, XcfError> = (|| {
            use rustix::fs::{Mode, OFlags};
            self.budget.consume_work(1, "generated archive anonymous create")?;
            let directory = self.directory.as_ref().ok_or_else(|| error(ErrorKind::Structure, "scratch directory handle missing"))?;
            let fd = rustix::fs::openat(directory, ".", OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC, Mode::RUSR | Mode::WUSR)
                .map_err(|_| error(ErrorKind::Unsupported, "anonymous generated storage unavailable"))?;
            Ok(File::from(fd))
        })();
        let file = opened.map_err(|e| match e.kind {
            ErrorKind::Cancelled => BackingError::Cancelled,
            ErrorKind::Limit => BackingError::Quota,
            ErrorKind::Allocation => BackingError::Allocation,
            ErrorKind::Unsupported => BackingError::Unsupported,
            _ => BackingError::Io,
        })?;
        let owned = GeneratedArchiveDestination::from_authorized_handle(file, quota.clone(), lease, scratch, max_bytes, control);
        control.check(1)?;
        Ok(owned)
    }
    /// Creates exact byte storage using this explicit creation authority only.
    /// The returned storage retains no directory/job references. Opening/path
    /// staging uses the job Budget; persistent handle/data use BackingQuota.
    /// These scopes are not unified aggregate native profile admission.
    #[cfg(any(windows, target_os = "linux"))]
    pub fn create_owned_byte_storage(
        &self,
        length: u64,
        quota: &photocraft_raster::backing::quota::BackingQuota,
        control: &photocraft_raster::backing::ReadControl<'_>,
    ) -> Result<crate::native_plane_store::provider::OwnedNativeStorage, photocraft_raster::backing::BackingError> {
        use crate::native_plane_store::provider::OwnedNativeStorage;
        use photocraft_raster::backing::{BackingError, quota::QuotaKind};
        control.check(1)?;
        if length == 0 || length > u64::from(crate::native_plane_store::NATIVE_STORE_MAX_BLOCKS) * crate::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64 {
            return Err(BackingError::Bounds);
        }
        let lease = quota.reserve(QuotaKind::Resident, OwnedNativeStorage::handle_envelope()).map_err(|_| BackingError::Quota)?;
        let scratch = quota.reserve(QuotaKind::Scratch, length).map_err(|_| BackingError::Quota)?;
        #[cfg(windows)]
        let opened = self.create_windows();
        #[cfg(target_os = "linux")]
        let opened: Result<File, XcfError> = (|| {
            use rustix::fs::{Mode, OFlags};
            self.budget.consume_work(1, "owned native anonymous create")?;
            let directory = self.directory.as_ref().ok_or_else(|| error(ErrorKind::Structure, "scratch directory handle missing"))?;
            let fd = rustix::fs::openat(directory, ".", OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC, Mode::RUSR | Mode::WUSR)
                .map_err(|_| error(ErrorKind::Unsupported, "anonymous native storage unavailable"))?;
            Ok(File::from(fd))
        })();
        let file = opened.map_err(|e| match e.kind {
            ErrorKind::Cancelled => BackingError::Cancelled,
            ErrorKind::Limit => BackingError::Quota,
            ErrorKind::Allocation => BackingError::Allocation,
            ErrorKind::Unsupported => BackingError::Unsupported,
            _ => BackingError::Io,
        })?;
        let owned = OwnedNativeStorage::from_authorized_handle(file, quota.clone(), lease, scratch);
        control.check(1)?;
        Ok(owned)
    }
    /// Preserves native-plane creation with its already checked exact layout.
    #[cfg(any(windows, target_os = "linux"))]
    pub fn create_owned_native_storage(
        &self,
        descriptor: photocraft_raster::backing::NativePlaneDescriptor,
        quota: &photocraft_raster::backing::quota::BackingQuota,
        control: &photocraft_raster::backing::ReadControl<'_>,
    ) -> Result<crate::native_plane_store::provider::OwnedNativeStorage, photocraft_raster::backing::BackingError> {
        self.create_owned_byte_storage(descriptor.logical_bytes(), quota, control)
    }
    pub fn from_platform_authorized(directory: PathBuf, budget: &'b Budget<'a>) -> Result<Self, XcfError> {
        budget.poll("scratch directory admission")?;
        if !cfg!(any(windows, target_os = "linux")) {
            return Err(error(ErrorKind::Unsupported, "no qualified kernel-lifetime scratch backend on this OS"));
        }
        if !directory.is_absolute() || directory.as_os_str().len() > MAX_PATH || directory.capacity() > MAX_CAPACITY {
            return Err(error(ErrorKind::Limit, "scratch directory must be bounded and absolute"));
        }
        let retained =
            directory.capacity().checked_add(std::mem::size_of::<Self>()).ok_or_else(|| error(ErrorKind::Limit, "scratch directory metadata overflow"))?;
        let lease = budget.reserve(BudgetKind::Resident, retained as u64, "scratch directory retained metadata")?;
        let out = Self {
            #[cfg(target_os = "linux")]
            directory: None,
            locator: Locator { path: directory, _lease: lease },
            budget,
        };
        validate_root(&out.locator.path, budget)?;
        #[cfg(target_os = "linux")]
        let mut out = out;
        #[cfg(target_os = "linux")]
        {
            use rustix::fs::{Mode, OFlags};
            budget.consume_work(1, "scratch directory open")?;
            let _call = path_call(&out.locator.path, budget)?;
            let fd = rustix::fs::open(&out.locator.path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())
                .map_err(|e| io_error(e.into()))?;
            out.directory = Some(File::from(fd));
        }
        budget.poll("scratch directory admitted")?;
        Ok(out)
    }

    pub fn capture<'d>(&'d self, source: &mut dyn Source) -> Result<OwnedScratchXcfSnapshot<'b, 'a, NativeScratchHandle<'d, 'b, 'a>>, XcfError> {
        self.budget.check(BudgetKind::Encoded, source.len(), "scratch provider source admission")?;
        self.budget.check(BudgetKind::Scratch, source.len(), "scratch provider disk admission")?;
        let handle = self.create()?;
        OwnedScratchXcfSnapshot::capture(source, handle, self.budget)
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn create(&self) -> Result<NativeScratchHandle<'_, 'b, 'a>, XcfError> {
        let lease = self.budget.reserve(BudgetKind::Resident, std::mem::size_of::<NativeScratchHandle<'_, '_, '_>>() as u64, "scratch handle construction")?;
        #[cfg(windows)]
        let file = self.create_windows()?;
        #[cfg(target_os = "linux")]
        let file = {
            use rustix::fs::{Mode, OFlags};
            self.budget.consume_work(1, "scratch anonymous create")?;
            let directory = self.directory.as_ref().ok_or_else(|| error(ErrorKind::Structure, "scratch directory handle missing"))?;
            // No named fallback: a filesystem without O_TMPFILE is not qualified here.
            let fd = rustix::fs::openat(directory, ".", OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC, Mode::RUSR | Mode::WUSR)
                .map_err(|e| error(ErrorKind::Unsupported, &format!("anonymous scratch creation unavailable: {e}")))?;
            File::from(fd)
        };
        let handle = NativeScratchHandle { file, _lease: lease, _directory: self, sealed: false };
        self.budget.poll("scratch handle created")?;
        Ok(handle)
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn create(&self) -> Result<NativeScratchHandle<'_, 'b, 'a>, XcfError> {
        Err(error(ErrorKind::Unsupported, "kernel-lifetime scratch backend unavailable"))
    }

    #[cfg(windows)]
    fn create_windows(&self) -> Result<File, XcfError> {
        use std::{
            fs::OpenOptions,
            os::windows::fs::OpenOptionsExt,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let capacity = self.locator.path.as_os_str().len().checked_add(80).ok_or_else(|| error(ErrorKind::Limit, "scratch filename size"))?;
        let mut target = bounded_path(capacity, self.budget)?;
        for _ in 0..32 {
            self.budget.consume_work(1, "scratch exclusive create")?;
            let id = NEXT
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| error(ErrorKind::Limit, "scratch identifier exhausted"))?;
            let mut name = [0u8; 64];
            let mut length = 0;
            append(&mut name, &mut length, b".photocraft-xcf-")?;
            append_decimal(&mut name, &mut length, u64::from(std::process::id()))?;
            append(&mut name, &mut length, b"-")?;
            append_decimal(&mut name, &mut length, id)?;
            let text = std::str::from_utf8(name.get(..length).ok_or_else(|| error(ErrorKind::Structure, "scratch name bounds"))?)
                .map_err(|_| error(ErrorKind::Structure, "scratch name encoding"))?;
            target.path.clear();
            target.path.push(&self.locator.path);
            target.path.push(text);
            let _call = path_call(&target.path, self.budget)?;
            match OpenOptions::new().create_new(true).read(true).write(true).share_mode(0).custom_flags(0x04000100).open(&target.path) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_error(e)),
                Ok(file) => {
                    self.budget.poll("scratch exclusive create completed")?;
                    return Ok(file);
                }
            }
        }
        Err(error(ErrorKind::Limit, "scratch collision retry limit"))
    }
}

#[cfg(windows)]
fn append(out: &mut [u8], length: &mut usize, text: &[u8]) -> Result<(), XcfError> {
    let end = length.checked_add(text.len()).ok_or_else(|| error(ErrorKind::Structure, "scratch name overflow"))?;
    out.get_mut(*length..end).ok_or_else(|| error(ErrorKind::Structure, "scratch name capacity"))?.copy_from_slice(text);
    *length = end;
    Ok(())
}
#[cfg(windows)]
fn append_decimal(out: &mut [u8], length: &mut usize, mut value: u64) -> Result<(), XcfError> {
    let mut digits = [0u8; 20];
    let mut start = digits.len();
    loop {
        start = start.checked_sub(1).ok_or_else(|| error(ErrorKind::Structure, "scratch decimal capacity"))?;
        *digits.get_mut(start).ok_or_else(|| error(ErrorKind::Structure, "scratch decimal bounds"))? = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    append(out, length, digits.get(start..).ok_or_else(|| error(ErrorKind::Structure, "scratch decimal bounds"))?)
}

/// Opaque kernel-lifetime storage. Constructed only by the directory capability.
pub struct NativeScratchHandle<'d, 'b, 'a> {
    // Actual storage must be gone before its retained handle charge is refunded.
    file: File,
    _lease: Reservation<'b, 'a>,
    _directory: &'d ServiceAuthorizedScratchDirectory<'b, 'a>,
    sealed: bool,
}
impl ScratchHandle for NativeScratchHandle<'_, '_, '_> {
    fn resident_bytes(&self) -> u64 {
        0
    } // Already charged by our live construction lease.
    fn len(&mut self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        if output.len() > SLICE {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "scratch read slice exceeds bound"));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read(output)
    }
    fn write_at(&mut self, offset: u64, input: &[u8]) -> io::Result<usize> {
        if self.sealed || input.len() > SLICE {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "scratch is sealed or write exceeds bound"));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write(input)
    }
    fn seal(&mut self) -> io::Result<()> {
        // Same-handle writes are already coherent; ephemeral scratch needs no
        // crash-durable fsync. Logical sealing plus no aliases is the contract.
        self.file.flush()?;
        self.sealed = true;
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new(label: &str) -> io::Result<Self> {
            let path = std::env::temp_dir().join(format!("photocraft-sol2-provider-unit-{}-{label}", std::process::id()));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir(&self.0) {
                eprintln!("test scratch directory cleanup failed: {error}");
            }
        }
    }
    fn budget() -> Result<Budget<'static>, XcfError> {
        Ok(Budget::new(photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::NativeStandard)?, None))
    }
    #[test]
    fn native_seal_prevents_all_further_writes_and_drop_cleans_storage() {
        let root = Root::new("seal").unwrap();
        let budget = budget().unwrap();
        let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
        let retained = budget.used(BudgetKind::Resident);
        let mut handle = directory.create().unwrap();
        assert_eq!(handle.resident_bytes(), 0);
        assert!(budget.used(BudgetKind::Resident) > retained);
        assert_eq!(handle.write_at(0, b"literal").unwrap(), 7);
        handle.seal().unwrap();
        assert_eq!(handle.write_at(0, b"mutate").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(handle.read_at(0, &mut [0; SLICE + 1]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        let mut out = [0; 7];
        assert_eq!(handle.read_at(0, &mut out).unwrap(), 7);
        assert_eq!(&out, b"literal");
        drop(handle);
        assert_eq!(budget.used(BudgetKind::Resident), retained);
        assert!(std::fs::read_dir(&root.0).unwrap().next().is_none());
        drop(directory);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
    }
    #[test]
    fn exclusive_collision_retry_is_bounded_and_preserves_existing_bytes() {
        let root = Root::new("collision").unwrap();
        for id in 0..128 {
            std::fs::write(root.0.join(format!(".photocraft-xcf-{}-{id}", std::process::id())), b"keep").unwrap();
        }
        let budget = budget().unwrap();
        let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
        let retained = budget.used(BudgetKind::Resident);
        let work = budget.used(BudgetKind::Work);
        assert_eq!(directory.create().err().unwrap().kind, ErrorKind::Limit);
        assert_eq!(budget.used(BudgetKind::Work) - work, 32);
        assert_eq!(budget.used(BudgetKind::Resident), retained);
        drop(directory);
        for id in 0..128 {
            let path = root.0.join(format!(".photocraft-xcf-{}-{id}", std::process::id()));
            assert_eq!(std::fs::read(&path).unwrap(), b"keep");
            std::fs::remove_file(path).unwrap();
        }
        assert_eq!(budget.used(BudgetKind::Resident), 0);
    }
}
