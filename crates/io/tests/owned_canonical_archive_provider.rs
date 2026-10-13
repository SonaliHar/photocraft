#![cfg(windows)]
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::BackedLayer;
use photocraft_format::{
    canonical_archive::{CanonicalArchivePlaneStore, CanonicalTileReference, ImmutableCanonicalArchive},
    canonical_object::PreparedCanonicalTile,
    zip::stream::{Limits, Writer},
};
use photocraft_geom::{Rect, TileCoord};
use photocraft_io::{
    native_plane_store::{NativeStoreLimits, OwnedCanonicalArchive, provider::OwnedNativeStorage},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use photocraft_xcf::{Budget, ResourceLimits, ResourcePolicy};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-sol4-canonical-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir(&self.0) {
            eprintln!("owned archive test cleanup:{error}");
        }
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 16 << 20, scratch: 8 << 20, index: 4 << 20 })
}
fn capture(root: &Root, bytes: &[u8], q: &BackingQuota) -> Arc<OwnedCanonicalArchive<OwnedNativeStorage>> {
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let handle = service.create_owned_byte_storage(bytes.len() as u64, q, &control()).unwrap();
    Arc::new(
        OwnedCanonicalArchive::capture(handle, bytes.len() as u64, q.clone(), NativeStoreLimits::default(), &control(), |offset, out, _| {
            let start = offset as usize;
            out.copy_from_slice(&bytes[start..start + out.len()]);
            Ok(())
        })
        .unwrap(),
    )
}
fn raw_archive(raw: &[u8]) -> (Vec<u8>, [u8; 32]) {
    let hash = *blake3::hash(raw).as_bytes();
    let path = format!("tiles/{}.zst", hash.iter().map(|v| format!("{v:02x}")).collect::<String>());
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38];
    frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    let total = raw.len().div_ceil(128 << 10);
    for (index, chunk) in raw.chunks(128 << 10).enumerate() {
        let head = ((chunk.len() as u32) << 3) | u32::from(index + 1 == total);
        frame.extend_from_slice(&head.to_le_bytes()[..3]);
        frame.extend_from_slice(chunk);
    }
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add(&path, frame.len() as u64, &mut Cursor::new(frame), &Interrupt::NONE).unwrap();
    (writer.finish(&Interrupt::NONE).unwrap().into_inner(), hash)
}
#[test]
fn real_owned_archive_outlives_capture_and_round_trips_backed_cow_history() {
    let root = Root::new();
    let q = quota();
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let pixel = [(-0.0f64).to_ne_bytes(), (0.125 + 2f64.powi(-40)).to_ne_bytes(), 0.75f64.to_ne_bytes(), 1f64.to_ne_bytes()].concat();
    let (source, hash) = raw_archive(&pixel.repeat(256 * 256));
    let source_hash = *blake3::hash(&source).as_bytes();
    let archive = capture(&root, &source, &q);
    assert_eq!(root.entries(), 1);
    assert_eq!(q.usage().unwrap().scratch, source.len() as u64);
    let mut check = vec![0; source.len()];
    for (index, out) in check.chunks_mut(65536).enumerate() {
        archive.read_exact_at((index * 65536) as u64, out, &control()).unwrap();
    }
    assert_eq!(*blake3::hash(&check).as_bytes(), source_hash);
    drop(check);
    drop(source); // source reader/provider/job no longer exist
    let desc = NativePlaneDescriptor::new(format, Rect::from_xywh(3, 4, 2, 2), &[0; 32]).unwrap();
    let refs = [CanonicalTileReference { coord: TileCoord::new(0, 0), hash }];
    let other = quota();
    assert!(matches!(CanonicalArchivePlaneStore::new(archive.clone(), desc, &refs, other.clone(), &control()), Err(BackingError::Quota)));
    assert_eq!(other.usage().unwrap(), QuotaUsage::default());
    let store = Arc::new(CanonicalArchivePlaneStore::new(archive.clone(), desc, &refs, q.clone(), &control()).unwrap());
    let mut layer = BackedLayer::new(BackedRaster::new(store.clone(), q.clone(), 2, &control()).unwrap(), desc, &control()).unwrap();
    let history = layer.clone();
    let mut before = [0; 32];
    history.checked_view().read_pixel_native_into(3, 4, &mut before, &control()).unwrap();
    assert_eq!(before, pixel.as_slice());
    let changed = [0.5f64.to_ne_bytes(), 0.25f64.to_ne_bytes(), (-0.0f64).to_ne_bytes(), 1f64.to_ne_bytes()].concat();
    layer.try_write_native(Rect::from_xywh(3, 4, 1, 1), &changed, &control()).unwrap();
    let mut old = [0; 32];
    history.checked_view().read_pixel_native_into(3, 4, &mut old, &control()).unwrap();
    assert_eq!(old, before);
    let write_control = control();
    let prepared = PreparedCanonicalTile::prepare(&layer, TileCoord::new(0, 0), &write_control).unwrap();
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    let metadata = prepared.write_entry(&mut writer).unwrap();
    let encoded = writer.finish(&Interrupt::NONE).unwrap().into_inner();
    let current_archive = capture(&root, &encoded, &q);
    drop(encoded);
    let current_store = Arc::new(
        CanonicalArchivePlaneStore::new(
            current_archive.clone(),
            desc,
            &[CanonicalTileReference { coord: TileCoord::new(0, 0), hash: metadata.hash() }],
            q.clone(),
            &control(),
        )
        .unwrap(),
    );
    let current = BackedRaster::new(current_store.clone(), q.clone(), 2, &control()).unwrap();
    let mut actual = [0; 32];
    current.read_pixel_into(3, 4, &mut actual, &control()).unwrap();
    assert_eq!(actual, changed.as_slice());
    drop(current);
    drop(current_store);
    drop(current_archive);
    drop(layer);
    drop(history);
    drop(store);
    drop(archive);
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn actual_capture_late_error_and_cancel_remove_partial_file_and_refund() {
    let root = Root::new();
    let q = quota();
    let job = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let service = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    for failure in [BackingError::Io, BackingError::Cancelled] {
        let handle = service.create_owned_byte_storage(9001, &q, &control()).unwrap();
        let error = OwnedCanonicalArchive::capture(handle, 9001, q.clone(), NativeStoreLimits::default(), &control(), |offset, out, _| {
            if offset >= 4096 {
                return Err(failure);
            }
            out.fill(0x79);
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error, failure);
        assert_eq!(root.entries(), 0);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
