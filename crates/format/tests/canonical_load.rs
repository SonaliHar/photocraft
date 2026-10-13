use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_format::{
    canonical_load::{VerificationError, verify_tile},
    zip::stream::{Limits, Reader, Writer},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::io::{Cursor, Read, Seek, SeekFrom};
fn frame(raw: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x28, 0xb5, 0x2f, 0xfd, 0xc0, 0x38];
    bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    for (index, block) in raw.chunks(128 * 1024).enumerate() {
        let last = (index + 1) * 128 * 1024 >= raw.len();
        let header = ((block.len() as u32) << 3) | u32::from(last);
        bytes.extend_from_slice(&header.to_le_bytes()[..3]);
        bytes.extend_from_slice(block);
    }
    bytes
}
fn path(hash: [u8; 32]) -> String {
    format!("tiles/{}.zst", blake3::Hash::from_bytes(hash))
}
fn archive(hash: [u8; 32], z: &[u8]) -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    writer.add(&path(hash), z.len() as u64, &mut Cursor::new(z), &Interrupt::NONE).unwrap();
    writer.finish(&Interrupt::NONE).unwrap().into_inner()
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 * 1024, index: 0, scratch: 0 })
}
struct Fragmented(Cursor<Vec<u8>>);
impl Read for Fragmented {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = out.len().min(7);
        self.0.read(&mut out[..n])
    }
}
impl Seek for Fragmented {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(p)
    }
}
#[test]
fn fragmented_exact_all_native_widths_hash_size_and_bounded_staging() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let format = PixelFormat::new(mode, sample, true);
            let n = 256 * 256 * format.bytes_per_pixel();
            let raw: Vec<_> = (0..n).map(|i| (i.wrapping_mul(31) & 255) as u8).collect();
            let hash = *blake3::hash(&raw).as_bytes();
            let bytes = archive(hash, &frame(&raw));
            let mut reader = Reader::new(Fragmented(Cursor::new(bytes)), Limits::default(), &Interrupt::NONE).unwrap();
            let q = quota();
            let work = |_| Ok(());
            let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
            let mut staged = Vec::new();
            let mut calls = 0;
            let token = verify_tile(&mut reader, format, hash, &q, &ctl, |offset, b| {
                assert_eq!(offset as usize, staged.len());
                assert!(b.len() <= 4096);
                assert!(q.usage().unwrap().resident >= 65536);
                staged.extend_from_slice(b);
                calls += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(staged, raw);
            assert_eq!(token.hash(), hash);
            assert_eq!(token.length(), n as u64);
            assert_eq!(token.format(), format);
            assert!(calls > 1);
            assert_eq!(q.usage().unwrap(), QuotaUsage::default());
        }
    }
}
#[test]
fn corruption_missing_size_trailing_and_unsupported_never_return_token() {
    let format = PixelFormat::RGBA8;
    let raw = vec![13; 256 * 256 * 4];
    let hash = *blake3::hash(&raw).as_bytes();
    for kind in 0..9 {
        let mut z = frame(&raw);
        let mut expected = hash;
        match kind {
            0 => {
                expected[0] ^= 1;
            }
            1 => {
                z.pop();
            }
            2 => z.push(0),
            3 => z[6] ^= 1,
            4 => z[4] = 0xc1,
            5 => z[14] |= 2,
            6 => z[14] |= 4,
            7 => z[14..17].copy_from_slice(&((128_u32 * 1024 + 1) << 3).to_le_bytes()[..3]),
            _ => {
                z = ruzstd::encoding::compress_to_vec(raw.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
            }
        }
        let mut reader = Reader::new(Cursor::new(archive(hash, &z)), Limits::default(), &Interrupt::NONE).unwrap();
        let q = quota();
        let work = |_| Ok(());
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let mut staged = Vec::new();
        let result = verify_tile(&mut reader, format, expected, &q, &ctl, |_, b| {
            staged.extend_from_slice(b);
            Ok(())
        });
        assert!(result.is_err(), "kind{kind}");
        if matches!(kind, 4 | 5 | 6 | 8) {
            assert!(matches!(result, Err(VerificationError::Unsupported)));
        }
        drop(staged);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    // Correct path but wrong raw hash must fail after unpublished data has staged.
    let mut changed = raw.clone();
    changed[50] ^= 1;
    let mut reader = Reader::new(Cursor::new(archive(hash, &frame(&changed))), Limits::default(), &Interrupt::NONE).unwrap();
    let q = quota();
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut count = 0;
    assert!(matches!(
        verify_tile(&mut reader, format, hash, &q, &ctl, |_, b| {
            count += b.len();
            Ok(())
        }),
        Err(VerificationError::Integrity)
    ));
    assert_eq!(count, raw.len());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn callback_work_cancel_and_quota_failures_refund_private_verifier() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let raw = vec![9; 256 * 256 * 4];
    let hash = *blake3::hash(&raw).as_bytes();
    for kind in 0..4 {
        let bytes = archive(hash, &frame(&raw));
        let mut reader = Reader::new(Cursor::new(bytes), Limits::default(), &Interrupt::NONE).unwrap();
        let q = if kind == 3 { BackingQuota::new(QuotaLimits::default()) } else { quota() };
        let calls = AtomicU64::new(0);
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::SeqCst);
        let interrupt = Interrupt::cancel_only(&check);
        let work = |_| {
            if kind == 1 && calls.load(Ordering::SeqCst) > 2 { Err(BackingError::Work) } else { Ok(()) }
        };
        let ctl = ReadControl { interrupt: &interrupt, work: &work };
        let result = verify_tile(&mut reader, PixelFormat::RGBA8, hash, &q, &ctl, |_, _| {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            if n > 2 {
                if kind == 0 {
                    return Err(BackingError::Io);
                }
                if kind == 2 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        });
        assert!(matches!(result, Err(VerificationError::Backing(_))));
        if kind == 3 {
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn real_prepared_default_wide_object_zipcrc_and_late_io() {
    use photocraft_doc::BackedLayer;
    use photocraft_format::canonical_object::PreparedCanonicalTile;
    use photocraft_geom::{Rect, TileCoord};
    use photocraft_raster::backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Debug)]
    struct Store(NativePlaneDescriptor);
    impl NativePlaneStore for Store {
        fn descriptor(&self) -> NativePlaneDescriptor {
            self.0
        }
        fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
            Err(BackingError::Io)
        }
    }
    let q = BackingQuota::new(QuotaLimits { resident: 8 << 20, index: 1 << 20, scratch: 0 });
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let format = PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true);
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(0, 0, 1, 1), &[0; 40]).unwrap();
    let layer = BackedLayer::new(BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 2, &ctl).unwrap(), descriptor, &ctl).unwrap();
    let prepared = PreparedCanonicalTile::prepare(&layer, TileCoord::new(1, 1), &ctl).unwrap();
    let meta = prepared.metadata();
    drop(layer);
    let mut writer = Writer::new(Cursor::new(Vec::new()), Limits::default()).unwrap();
    prepared.write_entry(&mut writer).unwrap();
    let bytes = writer.finish(&Interrupt::NONE).unwrap().into_inner();
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut reader = Reader::new(Cursor::new(bytes.clone()), Limits::default(), &Interrupt::NONE).unwrap();
    let mut staged = 0;
    let token = verify_tile(&mut reader, format, meta.hash(), &q, &ctl, |_, b| {
        assert!(b.iter().all(|byte| *byte == 0));
        staged += b.len();
        Ok(())
    })
    .unwrap();
    assert_eq!(token.length(), 2_621_440);
    assert_eq!(staged, 2_621_440);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let mut damaged = bytes.clone();
    let name_length = u16::from_le_bytes([damaged[26], damaged[27]]) as usize;
    let extra = u16::from_le_bytes([damaged[28], damaged[29]]) as usize;
    damaged[30 + name_length + extra + 40] ^= 1;
    let mut reader = Reader::new(Cursor::new(damaged), Limits::default(), &Interrupt::NONE).unwrap();
    let mut staged = 0;
    assert!(matches!(
        verify_tile(&mut reader, format, meta.hash(), &q, &ctl, |_, b| {
            staged += b.len();
            Ok(())
        }),
        Err(VerificationError::Container(_))
    ));
    assert_eq!(staged, 2_621_440);
    struct Fault {
        input: Cursor<Vec<u8>>,
        armed: Arc<AtomicBool>,
    }
    impl Read for Fault {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.armed.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("late input failure"));
            }
            self.input.read(out)
        }
    }
    impl Seek for Fault {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.input.seek(p)
        }
    }
    let armed = Arc::new(AtomicBool::new(false));
    let mut reader = Reader::new(Fault { input: Cursor::new(bytes), armed: armed.clone() }, Limits::default(), &Interrupt::NONE).unwrap();
    let mut staged = 0;
    assert!(matches!(
        verify_tile(&mut reader, format, meta.hash(), &q, &ctl, |_, b| {
            staged += b.len();
            if staged > 70000 {
                armed.store(true, Ordering::SeqCst);
            }
            Ok(())
        }),
        Err(VerificationError::Container(_))
    ));
    assert!(staged > 70000 && staged < 2_621_440);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
#[ignore = "release-only repeated canonical object verification throughput, not a Document migration"]
fn release_24mp_equivalent_verification() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let raw: Vec<u8> = (0..256 * 256 * 4).map(|i| (i % 251) as u8).collect();
    let hash = *blake3::hash(&raw).as_bytes();
    let mut reader = Reader::new(Cursor::new(archive(hash, &frame(&raw))), Limits::default(), &Interrupt::NONE).unwrap();
    let q = quota();
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut verified = 0u64;
    let start = std::time::Instant::now();
    for _ in 0..384 {
        verify_tile(&mut reader, PixelFormat::RGBA8, hash, &q, &ctl, |_, b| {
            verified += b.len() as u64;
            Ok(())
        })
        .unwrap();
    }
    println!(
        "repeated384RGBA8 tile verification {}bytes elapsed={:?}; one small retained ZIP object, no decoded-output allocation/disk/RSS/fullDocument claim",
        verified,
        start.elapsed()
    );
    assert_eq!(verified, 100_663_296);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    Ok(())
}
#[test]
fn valid_deflate_container_is_refused_before_any_decoder_or_staging() {
    use photocraft_format::FormatError;
    use std::io::Write;
    let raw = vec![5; 256 * 256 * 4];
    let hash = *blake3::hash(&raw).as_bytes();
    let z = frame(&raw);
    let name = path(hash);
    let mut deflater = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    deflater.write_all(&z).unwrap();
    let compressed = deflater.finish().unwrap();
    let mut crc = u32::MAX;
    for byte in &z {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    crc = !crc;
    let mut zip = vec![0; 30];
    zip[..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
    zip[4..6].copy_from_slice(&20u16.to_le_bytes());
    zip[8..10].copy_from_slice(&8u16.to_le_bytes());
    zip[14..18].copy_from_slice(&crc.to_le_bytes());
    zip[18..22].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
    zip[22..26].copy_from_slice(&(z.len() as u32).to_le_bytes());
    zip[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
    zip.extend_from_slice(name.as_bytes());
    zip.extend_from_slice(&compressed);
    let offset = zip.len();
    let mut central = vec![0; 46];
    central[..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
    central[4..6].copy_from_slice(&20u16.to_le_bytes());
    central[6..8].copy_from_slice(&20u16.to_le_bytes());
    central[10..12].copy_from_slice(&8u16.to_le_bytes());
    central[16..20].copy_from_slice(&crc.to_le_bytes());
    central[20..24].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
    central[24..28].copy_from_slice(&(z.len() as u32).to_le_bytes());
    central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
    central.extend_from_slice(name.as_bytes());
    zip.extend_from_slice(&central);
    let mut end = vec![0; 22];
    end[..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
    end[8..10].copy_from_slice(&1u16.to_le_bytes());
    end[10..12].copy_from_slice(&1u16.to_le_bytes());
    end[12..16].copy_from_slice(&(central.len() as u32).to_le_bytes());
    end[16..20].copy_from_slice(&(offset as u32).to_le_bytes());
    zip.extend_from_slice(&end);
    let mut reader = Reader::new(Cursor::new(zip), Limits::default(), &Interrupt::NONE).unwrap();
    assert!(matches!(reader.stored_size(&name), Err(FormatError::Unsupported(_))));
    assert!(matches!(reader.stored_size("missing"), Err(FormatError::Corrupt(_))));
    let q = quota();
    let work = |_| Ok(());
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut calls = 0;
    assert!(matches!(
        verify_tile(&mut reader, PixelFormat::RGBA8, hash, &q, &ctl, |_, _| {
            calls += 1;
            Ok(())
        }),
        Err(VerificationError::Unsupported)
    ));
    assert_eq!(calls, 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    assert_eq!(reader.read_by_name(&name, z.len(), &Interrupt::NONE).unwrap(), z, "normal valid DEFLATE copy remains unchanged");
}
