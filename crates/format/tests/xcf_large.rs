//! Streaming container gates; these alone do not enable native_large XCF policy.
use photocraft_format::{
    FormatError,
    zip::{
        ZipWriter,
        stream::{Limits, Reader, Writer},
    },
};
use photocraft_raster::Interrupt;
use std::io::{Cursor, Read, Write};

mod common;
#[test]
fn native_zip64_stream_retains_whole_document_and_reads_old_store() {
    for depth in [photocraft_color::SampleType::U8, photocraft_color::SampleType::U32, photocraft_color::SampleType::F64] {
        let doc = common::rich_doc(photocraft_color::ColorMode::Rgb, depth);
        let mut writer = photocraft_format::PcraftWriter::new();
        let (output, _) = writer.save_zip_stream(&doc, &Default::default(), Cursor::new(Vec::new()), Limits::default(), 64 << 20, &Interrupt::NONE).unwrap();
        let reopened = photocraft_format::load_from_stream(output, &Default::default(), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(reopened, doc);
        let old = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_stream(Cursor::new(old), &Default::default(), Limits::default(), &Interrupt::NONE).unwrap(), doc);
    }
}

#[test]
fn stream_reads_old_archive_and_writes_zip64_without_payload_collection() {
    let mut old = ZipWriter::new();
    old.add("manifest.json", b"{}").unwrap();
    old.add("blob", b"source").unwrap();
    let mut reader = Reader::new(Cursor::new(old.finish().unwrap()), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(reader.read_by_name("blob", 6, &Interrupt::NONE).unwrap(), b"source");
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add("manifest.json", 2, &mut Cursor::new(b"{}"), &Interrupt::NONE).unwrap();
    writer.add("blob", 6, &mut Cursor::new(b"source"), &Interrupt::NONE).unwrap();
    let bytes = writer.finish(&Interrupt::NONE).unwrap().into_inner();
    assert!(bytes.windows(4).any(|bytes| bytes == 0x06064b50u32.to_le_bytes()));
    let mut reader = Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(reader.read_by_name("blob", 6, &Interrupt::NONE).unwrap(), b"source");
    assert!(matches!(reader.read_by_name("blob", 5, &Interrupt::NONE), Err(FormatError::LimitExceeded(_))));
}

/// A conventional one-entry ZIP, with an independently supplied raw DEFLATE
/// member and the original payload CRC. Local/central sizes agree exactly.
fn deflated_archive(member: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::new();
    writer.add("a", b"data").unwrap();
    let bytes = writer.finish().unwrap();
    let central = bytes.windows(4).position(|v| v == [0x50, 0x4b, 0x01, 0x02]).unwrap();
    let mut local = bytes[..31].to_vec();
    let mut directory = bytes[central..].to_vec();
    local[8..10].copy_from_slice(&8u16.to_le_bytes());
    local[18..22].copy_from_slice(&(member.len() as u32).to_le_bytes());
    directory[10..12].copy_from_slice(&8u16.to_le_bytes());
    directory[20..24].copy_from_slice(&(member.len() as u32).to_le_bytes());
    directory[47 + 16..47 + 20].copy_from_slice(&((31 + member.len()) as u32).to_le_bytes());
    local.extend_from_slice(member);
    local.extend_from_slice(&directory);
    local
}
#[test]
fn deflate_requires_real_end_marker_and_exact_member_boundary() {
    // RFC1951 stored block: BFINAL=1, LEN=4, NLEN=!4, then data.
    let member = [1, 4, 0, 251, 255, b'd', b'a', b't', b'a'];
    let mut reader = Reader::new(Cursor::new(deflated_archive(&member)), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(reader.read_by_name("a", 4, &Interrupt::NONE).unwrap(), b"data");
    // BFINAL=0 produces every declared byte and the right CRC, but is not a
    // complete DEFLATE stream. Transport EOF must not count as its end marker.
    let mut unfinished = member;
    unfinished[0] = 0;
    let mut reader = Reader::new(Cursor::new(deflated_archive(&unfinished)), Limits::default(), &Interrupt::NONE).unwrap();
    assert!(reader.read_by_name("a", 4, &Interrupt::NONE).is_err());
    let mut extra = member.to_vec();
    extra.push(0);
    let mut reader = Reader::new(Cursor::new(deflated_archive(&extra)), Limits::default(), &Interrupt::NONE).unwrap();
    assert!(reader.read_by_name("a", 4, &Interrupt::NONE).is_err());
}

fn archive() -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add("a", 4, &mut Cursor::new(b"data"), &Interrupt::NONE).unwrap();
    writer.finish(&Interrupt::NONE).unwrap().into_inner()
}
#[test]
fn truncation_crc_and_conflicting_headers_fail() {
    let bytes = archive();
    for length in 0..bytes.len() {
        assert!(Reader::new(Cursor::new(bytes[..length].to_vec()), Limits::default(), &Interrupt::NONE).is_err(), "prefix {length}");
    }
    let mut bad_crc = bytes.clone();
    let payload = bytes.windows(4).position(|b| b == b"data").unwrap();
    bad_crc[payload] ^= 1;
    let mut reader = Reader::new(Cursor::new(bad_crc), Limits::default(), &Interrupt::NONE).unwrap();
    assert!(reader.read_by_name("a", 4, &Interrupt::NONE).is_err());
    let mut conflict = bytes.clone();
    conflict[30] = b'b';
    assert!(Reader::new(Cursor::new(conflict), Limits::default(), &Interrupt::NONE).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(Reader::new(Cursor::new(trailing), Limits::default(), &Interrupt::NONE).is_err());
}

#[test]
fn duplicate_names_metadata_limits_and_poisoned_output_fail() {
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add("a", 1, &mut Cursor::new(b"a"), &Interrupt::NONE).unwrap();
    assert!(writer.add("a", 1, &mut Cursor::new(b"b"), &Interrupt::NONE).is_err());
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    assert!(writer.add("a", 3, &mut Cursor::new(b"a"), &Interrupt::NONE).is_err());
    assert!(writer.finish(&Interrupt::NONE).is_err());
    let limits = Limits { max_entries: 0, ..Limits::default() };
    assert!(Reader::new(Cursor::new(archive()), limits, &Interrupt::NONE).is_err());
    let limits = Limits { max_metadata_bytes: 1, ..Limits::default() };
    assert!(Reader::new(Cursor::new(archive()), limits, &Interrupt::NONE).is_err());
    let limits = Limits { max_archive_bytes: 1, ..Limits::default() };
    assert!(Reader::new(Cursor::new(archive()), limits, &Interrupt::NONE).is_err());
}

struct ChunkedRead {
    remaining: u64,
}
impl Read for ChunkedRead {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        assert!(bytes.len() <= 64 * 1024);
        let count = self.remaining.min(bytes.len() as u64) as usize;
        bytes[..count].fill(0x51);
        self.remaining -= count as u64;
        Ok(count)
    }
}
#[derive(Default)]
struct BoundedSink {
    total: u64,
}
impl Write for BoundedSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        assert!(bytes.len() <= 64 * 1024);
        self.total += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn streaming_copy_has_bounded_work_and_cancellation() {
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add("large", 256 * 1024 + 3, &mut ChunkedRead { remaining: 256 * 1024 + 3 }, &Interrupt::NONE).unwrap();
    let bytes = writer.finish(&Interrupt::NONE).unwrap().into_inner();
    let mut reader = Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE).unwrap();
    let mut sink = BoundedSink::default();
    assert_eq!(reader.copy_by_name("large", &mut sink, 256 * 1024 + 3, &Interrupt::NONE).unwrap(), sink.total);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(reader.copy_by_name("large", &mut sink, 1 << 20, &interrupt), Err(FormatError::Cancelled)));
}

#[test]
fn streaming_path_replacement_preserves_original_on_budget_failure_and_cancel() {
    let root = common::temp_dir("stream-atomic");
    let path = root.join("working.pcraft");
    let original = b"original work must survive";
    std::fs::write(&path, original).unwrap();
    let doc = common::rich_doc(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let before = doc.clone();
    let mut writer = photocraft_format::PcraftWriter::new();
    assert!(matches!(writer.save_path_stream(&doc, &Default::default(), &path, Limits::default(), 1, &Interrupt::NONE), Err(FormatError::LimitExceeded(_))));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let cancelled = || true;
    assert!(matches!(
        writer.save_path_stream(&doc, &Default::default(), &path, Limits::default(), 64 << 20, &Interrupt::cancel_only(&cancelled)),
        Err(FormatError::Cancelled)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    writer.save_path_stream(&doc, &Default::default(), &path, Limits::default(), 64 << 20, &Interrupt::NONE).unwrap();
    assert_eq!(photocraft_format::load_path(&path).unwrap(), doc);
    assert_eq!(doc, before);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn streaming_large_blob_cancels_during_payload_and_preserves_destination() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let root = common::temp_dir("stream-blob-cancel");
    let path = root.join("working.pcraft");
    let original = b"previous native document";
    std::fs::write(&path, original).unwrap();
    let mut doc =
        photocraft_doc::Document::new("large blob", photocraft_doc::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    doc.metadata.exif = Some(Arc::new((0..2 * 1024 * 1024usize).map(|index| (index.wrapping_mul(37) & 255) as u8).collect()));
    let before = doc.clone();
    let observed = AtomicU64::new(0);
    // Observe actual payload writes: Windows can defer reporting the open
    // file's changing length. Preparation remains separately unqualified.
    struct ObservedOutput<'a> {
        file: &'a mut std::fs::File,
        written: &'a AtomicU64,
    }
    impl Write for ObservedOutput<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            assert!(bytes.len() <= 64 * 1024);
            let count = self.file.write(bytes)?;
            self.written.fetch_add(count as u64, Ordering::Relaxed);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    impl std::io::Seek for ObservedOutput<'_> {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(self.file, position)
        }
    }
    let cancelled = || observed.load(Ordering::Relaxed) >= 200 * 1024;
    let mut writer = photocraft_format::PcraftWriter::new();
    assert!(matches!(
        photocraft_format::atomic::atomic_write_stream(
            &path,
            |file| {
                writer
                    .save_zip_stream(
                        &doc,
                        &Default::default(),
                        ObservedOutput { file, written: &observed },
                        Limits::default(),
                        4 << 20,
                        &Interrupt::cancel_only(&cancelled),
                    )
                    .map(|(_, stats)| stats)
            },
            || if cancelled() { Err(FormatError::Cancelled) } else { Ok(()) }
        ),
        Err(FormatError::Cancelled)
    ));
    assert!((200 * 1024..=264 * 1024).contains(&observed.load(Ordering::Relaxed)), "observed temporary bytes {}", observed.load(Ordering::Relaxed));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(doc, before);
    writer.save_path_stream(&doc, &Default::default(), &path, Limits::default(), 4 << 20, &Interrupt::NONE).unwrap();
    assert_eq!(photocraft_format::load_path(&path).unwrap(), doc);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn streaming_atomic_commit_gate_discards_completed_output() {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = common::temp_dir("stream-atomic");
    let path = root.join("document.pcraft");
    std::fs::write(&path, b"previous document").unwrap();
    let checks = AtomicUsize::new(0);
    let result: photocraft_format::Result<()> = photocraft_format::atomic::atomic_write_stream(
        &path,
        |file| {
            file.write_all(b"completed replacement")?;
            Ok(())
        },
        || if checks.fetch_add(1, Ordering::Relaxed) == 0 { Ok(()) } else { Err(FormatError::Cancelled) },
    );
    assert!(matches!(result, Err(FormatError::Cancelled)));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous document");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    let result: photocraft_format::Result<()> = photocraft_format::atomic::atomic_write_stream(
        &path,
        |file| {
            file.write_all(b"partial")?;
            Err(FormatError::Unsupported("injected writer failure".into()))
        },
        || Ok(()),
    );
    assert!(matches!(result, Err(FormatError::Unsupported(_))));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous document");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
#[ignore = "writes and rereads a real >4 GiB ZIP64 entry; container gate only"]
fn actual_zip64_entry_crosses_four_gib_with_bounded_copy() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_ZIP64_ACCEPTANCE_DIR").expect("set isolated acceptance directory"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("entry-boundary.zip");
    let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();
    let size = (1u64 << 32) + 4096;
    let started = std::time::Instant::now();
    let mut writer = Writer::new(file, Limits::default()).unwrap();
    writer.add("wide-source", size, &mut ChunkedRead { remaining: size }, &Interrupt::NONE).unwrap();
    let file = writer.finish(&Interrupt::NONE).unwrap();
    file.sync_all().unwrap();
    assert!(file.metadata().unwrap().len() > 1 << 32);
    let mut reader = Reader::new(file, Limits::default(), &Interrupt::NONE).unwrap();
    let mut output = BoundedSink::default();
    assert_eq!(reader.copy_by_name("wide-source", &mut output, size, &Interrupt::NONE).unwrap(), size);
    assert_eq!(output.total, size);
    println!("actual ZIP64 entry {size} bytes write+read {:?}; bounded64KiB transfer; no XCF policy qualification", started.elapsed());
}
