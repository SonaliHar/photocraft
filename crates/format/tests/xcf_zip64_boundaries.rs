//! Small default-limit ZIP64 boundary controls, not native_large qualification.
use photocraft_format::{
    FormatError, PcraftWriter,
    zip::stream::{Limits, Reader, Writer},
};
use photocraft_raster::Interrupt;
use std::io::{Cursor, Read, Seek, SeekFrom};

fn archive() -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add("a", 4, &mut Cursor::new(b"data"), &Interrupt::NONE).unwrap();
    let bytes = writer.finish(&Interrupt::NONE).unwrap().into_inner();
    assert!(bytes.len() < 512);
    bytes
}
fn record(bytes: &[u8], signature: u32) -> usize {
    bytes.windows(4).position(|b| b == signature.to_le_bytes()).unwrap()
}
fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn central_extra(bytes: &[u8]) -> usize {
    let central = record(bytes, 0x0201_4b50);
    let name = u16::from_le_bytes(bytes[central + 28..central + 30].try_into().unwrap());
    central + 46 + usize::from(name)
}

#[test]
fn hostile_zip64_record_fields_reject_before_index_or_payload_collection() {
    let original = archive();
    let end = record(&original, 0x0606_4b50);
    let locator = record(&original, 0x0706_4b50);
    let extra = central_extra(&original);
    let mut valid = Reader::new(Cursor::new(&original), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(valid.read_by_name("a", 4, &Interrupt::NONE).unwrap(), b"data");
    for case in 0..8 {
        let mut bytes = original.clone();
        match case {
            0 => {
                put64(&mut bytes, end + 24, u64::MAX);
                put64(&mut bytes, end + 32, u64::MAX);
            }
            1 => put64(&mut bytes, end + 40, u64::MAX),
            2 => put64(&mut bytes, end + 48, u64::MAX),
            3 => put64(&mut bytes, locator + 8, u64::MAX),
            4 => put64(&mut bytes, end + 4, u64::MAX),
            5 => put64(&mut bytes, extra + 20, u64::MAX),
            6 => {
                put64(&mut bytes, extra + 4, u64::MAX);
                put64(&mut bytes, extra + 12, u64::MAX);
            }
            7 => {
                put64(&mut bytes, end + 24, 2);
                put64(&mut bytes, end + 32, 2);
            }
            _ => panic!("bounded case"),
        }
        let error = Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE).err().unwrap();
        if case <= 1 {
            assert!(matches!(error, FormatError::LimitExceeded(_)), "case{case}: {error}");
        } else {
            assert!(matches!(error, FormatError::Corrupt(_)), "case{case}: {error}");
        }
    }
    assert_eq!(valid.read_by_name("a", 4, &Interrupt::NONE).unwrap(), b"data", "rejections do not affect independent valid reader");
}

#[test]
fn required_zip64_extra_fields_and_local_sizes_cannot_be_omitted_or_conflict() {
    let original = archive();
    let central = central_extra(&original);
    let local = 31; // Fixed one-byte name emitted by the public writer.
    for case in 0..5 {
        let mut bytes = original.clone();
        match case {
            0 => put16(&mut bytes, central, 2), // Required ZIP64 extra omitted.
            1 => put16(&mut bytes, local, 2),
            2 => put16(&mut bytes, central + 2, 16), // Missing required offset value.
            3 => put16(&mut bytes, local + 2, 8),    // Missing second required size.
            4 => put64(&mut bytes, local + 4, 5),    // Local/central size conflict.
            _ => panic!("bounded case"),
        }
        assert!(matches!(Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE), Err(FormatError::Corrupt(_))), "case{case}");
    }
}

fn descriptor_archive(signed: bool) -> (Vec<u8>, usize) {
    let mut bytes = archive();
    let central = record(&bytes, 0x0201_4b50);
    let crc = bytes[14..18].to_vec();
    let mut descriptor = Vec::new();
    if signed {
        descriptor.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
    }
    descriptor.extend_from_slice(&crc);
    descriptor.extend_from_slice(&4u64.to_le_bytes());
    descriptor.extend_from_slice(&4u64.to_le_bytes());
    put16(&mut bytes, 6, 0x0808); // UTF8 plus deferred CRC/sizes.
    put16(&mut bytes, central + 8, 0x0808);
    bytes[14..18].fill(0);
    bytes.splice(central..central, descriptor);
    // Inserting a descriptor shifts the directory and terminators, not local data.
    let end = record(&bytes, 0x0606_4b50);
    let locator = record(&bytes, 0x0706_4b50);
    let directory = record(&bytes, 0x0201_4b50);
    put64(&mut bytes, end + 48, directory as u64);
    put64(&mut bytes, locator + 8, end as u64);
    (bytes, central + if signed { 4 } else { 0 })
}

#[test]
fn signed_and_unsigned_zip64_descriptors_require_exact_crc_and_both_sizes() {
    for signed in [false, true] {
        let (original, descriptor) = descriptor_archive(signed);
        let mut reader = Reader::new(Cursor::new(&original), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(reader.read_by_name("a", 4, &Interrupt::NONE).unwrap(), b"data");
        for field in [0, 4, 12] {
            let mut bytes = original.clone();
            bytes[descriptor + field] ^= 1;
            assert!(
                matches!(Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE), Err(FormatError::Corrupt(_))),
                "signed{signed} field{field}"
            );
        }
        let mut truncated = original;
        truncated.remove(descriptor + 19);
        let end = record(&truncated, 0x0606_4b50);
        let locator = record(&truncated, 0x0706_4b50);
        let directory = record(&truncated, 0x0201_4b50);
        put64(&mut truncated, end + 48, directory as u64);
        put64(&mut truncated, locator + 8, end as u64);
        assert!(
            matches!(Reader::new(Cursor::new(truncated), Limits::default(), &Interrupt::NONE), Err(FormatError::Corrupt(_))),
            "required descriptor truncated while end records remain valid"
        );
    }
}

#[test]
fn native_reopen_cancels_after_real_bounded_stream_reads_without_publication() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let mut doc =
        photocraft_doc::Document::new("Small reopen control", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    doc.metadata.exif = Some(Arc::new(vec![91; 192 << 10]));
    let before = doc.clone();
    let (output, _) =
        PcraftWriter::new().save_zip_stream(&doc, &Default::default(), Cursor::new(Vec::new()), Limits::default(), 64 << 20, &Interrupt::NONE).unwrap();
    let bytes = output.into_inner();
    assert!(bytes.len() < 256 << 10);
    assert_eq!(photocraft_format::load_from_stream(Cursor::new(&bytes), &Default::default(), Limits::default(), &Interrupt::NONE).unwrap(), doc);
    struct Counted<'a> {
        cursor: Cursor<&'a [u8]>,
        bytes_read: &'a AtomicUsize,
    }
    impl Read for Counted<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            assert!(output.len() <= 64 << 10, "bounded reader request");
            let read = self.cursor.read(output)?;
            self.bytes_read.fetch_add(read, Ordering::Relaxed);
            Ok(read)
        }
    }
    impl Seek for Counted<'_> {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(from)
        }
    }
    let reads = AtomicUsize::new(0);
    let cancelled = || reads.load(Ordering::Relaxed) >= 100 << 10;
    let result = photocraft_format::load_from_stream(
        Counted { cursor: Cursor::new(&bytes), bytes_read: &reads },
        &Default::default(),
        Limits::default(),
        &Interrupt::cancel_only(&cancelled),
    );
    assert!(matches!(result, Err(FormatError::Cancelled)), "{result:?}");
    assert!(reads.load(Ordering::Relaxed) >= 100 << 10);
    assert!(reads.load(Ordering::Relaxed) <= 256 << 10);
    assert_eq!(doc, before);
    assert_eq!(photocraft_format::load_from_stream(Cursor::new(bytes), &Default::default(), Limits::default(), &Interrupt::NONE).unwrap(), doc);
}
