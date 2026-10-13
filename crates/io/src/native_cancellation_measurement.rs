//! Scoped wall-clock measurements, deliberately opt-in rather than a CI timing promise.
use super::*;
use photocraft_doc::{Layer, LayerMask, PixelFormat, SampleEncoding, SampleType, Size, WorkingProfileInterpretation};
use std::{
    io::Cursor,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn masked(mode: ColorMode, sample: SampleType) -> Document {
    let mut doc = Document::new("Cancellation current-state control", Size::new(1024, 1024), mode, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let icc = if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() }.to_bytes();
    doc.working_profile_interpretation =
        if mode == ColorMode::Rgb { WorkingProfileInterpretation::implicit_srgb(&icc) } else { WorkingProfileInterpretation::implicit_sgray(&icc) };
    doc.icc_profile = Some(icc);
    for (name, alpha) in [("Backdrop", 1.), ("Masked foreground", 0.5)] {
        let mut layer = Layer::raster(name, doc.pixel_format());
        let values: &[f64] = if mode == ColorMode::Rgb { &[0.2, 0.5, 0.8, alpha] } else { &[0.5, alpha] };
        let mut raw = vec![0; doc.pixel_format().bytes_per_pixel()];
        for (channel, value) in values.iter().enumerate() {
            photocraft_color::try_write_sample_f64(&mut raw, sample, channel, *value).unwrap();
        }
        *layer.surface_mut().unwrap() = photocraft_raster::Surface::try_with_default_bytes(doc.pixel_format(), &raw).unwrap();
        if alpha < 1. {
            layer.mask = Some(LayerMask {
                surface: std::sync::Arc::new(photocraft_raster::Surface::try_with_default_bytes(PixelFormat::GRAY8, &[128]).unwrap()),
                enabled: true,
                linked: true,
                density: 1.,
                feather: 0.,
            });
        }
        doc.layers.push(layer);
    }
    assert!(PreparedRenderer::supports_accounted_scalar_preparation(&doc));
    doc
}

#[test]
#[ignore = "manual wall-clock native cancellation measurement; not a universal deadline guarantee"]
fn native_save_external_cancellation_before_publication() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::F32, SampleType::F64] {
            let doc = masked(mode, sample);
            let before = doc.clone();
            for repeat in 0..3 {
                let start = AtomicBool::new(false);
                let cancelled = AtomicBool::new(false);
                let (tx, rx) = mpsc::sync_channel(1);
                let mut output = Cursor::new(Vec::new());
                std::thread::scope(|scope| {
                    let cancel_ref = &cancelled;
                    let worker = scope.spawn(move || {
                        rx.recv_timeout(Duration::from_secs(10)).unwrap();
                        std::thread::sleep(Duration::from_millis(5));
                        let signalled = Instant::now();
                        cancel_ref.store(true, Ordering::Release);
                        signalled
                    });
                    let stop = || {
                        if !start.swap(true, Ordering::AcqRel) {
                            tx.send(()).unwrap();
                        }
                        cancelled.load(Ordering::Acquire)
                    };
                    let result = crate::save_native_stream(&doc, &mut output, crate::NativeSaveLimits::default(), &Interrupt::cancel_only(&stop));
                    let returned = Instant::now();
                    let signalled = worker.join().unwrap();
                    assert!(matches!(result, Err(IoError::Cancelled)), "{result:?}");
                    assert!(output.get_ref().is_empty());
                    assert_eq!(doc, before);
                    println!(
                        "PUBLIC_CANCEL mode={mode:?} sample={sample:?} repeat={repeat} signal_to_error_ms={:.6} output_bytes=0",
                        returned.duration_since(signalled).as_secs_f64() * 1000.
                    );
                });
            }
        }
    }
}

#[test]
#[ignore = "manual wall-clock native output cancellation/refund measurement"]
fn preview_output_external_cancellation_refunds_live_leases() {
    use photocraft_compose::preview_budget::PreparationCharge;
    for sample in [SampleType::F32, SampleType::F64] {
        let doc = masked(ColorMode::Rgb, sample);
        let before = doc.clone();
        for repeat in 0..3 {
            let started = AtomicBool::new(false);
            let cancelled = AtomicBool::new(false);
            let (tx, rx) = mpsc::sync_channel(1);
            let signal = Mutex::new(None);
            let observer = |event| {
                if matches!(event, PreparationCharge::Reserve(bytes) if bytes >= 1 << 20) && !started.swap(true, Ordering::AcqRel) {
                    tx.send(()).unwrap();
                }
            };
            let budget = PreviewPreparationBudget::new(64 << 20, &observer);
            // Conservative charge for this two-layer sparse fixture, retained across both outputs.
            let source_lease = budget.reserve_preparation_bytes(64 << 10, &Interrupt::NONE).unwrap();
            std::thread::scope(|scope| {
                let cancel_ref = &cancelled;
                let signal_ref = &signal;
                let worker = scope.spawn(move || {
                    rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    std::thread::sleep(Duration::from_millis(5));
                    *signal_ref.lock().unwrap() = Some(Instant::now());
                    cancel_ref.store(true, Ordering::Release);
                });
                let stop = || cancelled.load(Ordering::Acquire);
                let result = previews_with_budget(&doc, &Interrupt::cancel_only(&stop), &budget);
                let returned = Instant::now();
                worker.join().unwrap();
                assert!(matches!(result, Err(IoError::Cancelled)));
                assert_eq!(budget.usage().live_bytes, 64 << 10);
                assert_eq!(doc, before);
                let latency = returned.duration_since(signal.lock().unwrap().unwrap()).as_secs_f64() * 1000.;
                println!(
                    "OUTPUT_CANCEL sample={sample:?} repeat={repeat} signal_to_error_ms={latency:.6} refunded_bytes=all peak_bytes={}",
                    budget.usage().peak_bytes
                );
            });
            drop(source_lease);
            assert_eq!(budget.usage().live_bytes, 0);
        }
    }
}

#[test]
#[ignore = "manual wall-clock native preparation cancellation/refund measurement"]
fn retained_plane_preparation_external_cancellation_refunds() {
    use photocraft_compose::preview_budget::PreparationCharge;
    let mut doc = masked(ColorMode::Rgb, SampleType::F64);
    // Two modest retained planes make the constructor's checked source scan observable.
    // The canvas remains 1024 square; no source or output shape is newly admitted.
    let bounds = photocraft_doc::Rect::new(0, 0, 257, 257);
    let format = doc.pixel_format();
    for layer in &mut doc.layers {
        let surface = layer.surface().unwrap();
        let raw = surface.default_bytes().repeat(257 * 257);
        *layer.surface_mut().unwrap() = photocraft_raster::Surface::try_from_interleaved(format, bounds, &raw).unwrap();
    }
    let before = doc.clone();
    for repeat in 0..3 {
        let armed = AtomicBool::new(false);
        let started = AtomicBool::new(false);
        let cancelled = AtomicBool::new(false);
        let output_started = AtomicBool::new(false);
        let (tx, rx) = mpsc::sync_channel(1);
        let observer = |event| {
            if let PreparationCharge::Reserve(bytes) = event {
                if !armed.load(Ordering::Acquire) {
                    return;
                }
                if bytes >= 1 << 20 {
                    output_started.store(true, Ordering::Relaxed);
                }
                if !started.swap(true, Ordering::AcqRel) {
                    tx.send(()).unwrap();
                }
            }
        };
        let budget = PreviewPreparationBudget::new(64 << 20, &observer);
        // Conservative retained-source capacity charge, before the constructor is armed.
        let source_lease = budget.reserve_preparation_bytes(32 << 20, &Interrupt::NONE).unwrap();
        armed.store(true, Ordering::Release);
        std::thread::scope(|scope| {
            let cancel_ref = &cancelled;
            let worker = scope.spawn(move || {
                rx.recv_timeout(Duration::from_secs(10)).unwrap();
                std::thread::sleep(Duration::from_millis(1));
                let signal = Instant::now();
                cancel_ref.store(true, Ordering::Release);
                signal
            });
            let stop = || cancelled.load(Ordering::Acquire);
            let result = previews_with_budget(&doc, &Interrupt::cancel_only(&stop), &budget);
            let returned = Instant::now();
            let signal = worker.join().unwrap();
            assert!(matches!(result, Err(IoError::Cancelled)));
            assert!(!output_started.load(Ordering::Relaxed), "cancel must occur during preparation rather than output");
            assert_eq!(budget.usage().live_bytes, 32 << 20);
            assert_eq!(doc, before);
            println!(
                "PREP_CANCEL repeat={repeat} signal_to_error_ms={:.6} output_allocations=0 peak_bytes={}",
                returned.duration_since(signal).as_secs_f64() * 1000.,
                budget.usage().peak_bytes
            );
        });
        drop(source_lease);
        assert_eq!(budget.usage().live_bytes, 0);
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "manual wall-clock native path cancellation and destination cleanup measurement"]
fn native_path_external_cancellation_preserves_destination_and_cleans_up() {
    let doc = masked(ColorMode::Rgb, SampleType::F64);
    let before = doc.clone();
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::env::temp_dir().join(format!("photocraft-sol4-cancel-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let destination = directory.join("existing.pcraft");
    std::fs::write(&destination, b"existing destination authority").unwrap();
    for repeat in 0..3 {
        let started = AtomicBool::new(false);
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            let cancel_ref = &cancelled;
            let worker = scope.spawn(move || {
                rx.recv_timeout(Duration::from_secs(10)).unwrap();
                std::thread::sleep(Duration::from_millis(5));
                let signal = Instant::now();
                cancel_ref.store(true, Ordering::Release);
                signal
            });
            let stop = || {
                if !started.swap(true, Ordering::AcqRel) {
                    tx.send(()).unwrap();
                }
                cancelled.load(Ordering::Acquire)
            };
            let result = crate::save_native_path(&doc, &destination, Default::default(), &Interrupt::cancel_only(&stop));
            let returned = Instant::now();
            let signal = worker.join().unwrap();
            assert!(matches!(result, Err(IoError::Cancelled)));
            assert_eq!(std::fs::read(&destination).unwrap(), b"existing destination authority");
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1, "no temporary outputs remain");
            assert_eq!(doc, before);
            println!(
                "PATH_CANCEL repeat={repeat} signal_to_error_ms={:.6} destination_unchanged=true extra_files=0",
                returned.duration_since(signal).as_secs_f64() * 1000.
            );
        });
    }
    std::fs::remove_file(&destination).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

// Separate archive-phase controls: the earlier measurements deliberately stopped
// before publication. This generated current-state fixture has 16 MiB of retained
// native RGBA8 tiles, with nonrepeating RGB and unit alpha; it is not a source oracle.
fn archive_current() -> Document {
    let mut doc = Document::new("Archive cancellation current-state control", Size::new(2048, 2048), ColorMode::Rgb, SampleType::U8);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let icc = Builtin::Srgb.profile().to_bytes();
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&icc);
    doc.icc_profile = Some(icc);
    let mut raw = vec![0; 2048 * 2048 * 4];
    let mut state = 0x4a38_195b_u32;
    for pixel in raw.as_chunks_mut::<4>().0.iter_mut().map(|chunk| chunk.as_mut_slice()) {
        for value in pixel.iter_mut().take(3) {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *value = state as u8;
        }
        pixel[3] = 255;
    }
    let mut layer = Layer::raster("Generated native samples", doc.pixel_format());
    *layer.surface_mut().unwrap() =
        photocraft_raster::Surface::try_from_interleaved(doc.pixel_format(), photocraft_doc::Rect::new(0, 0, 2048, 2048), &raw).unwrap();
    doc.layers.push(layer);
    assert_eq!(crate::retained_native_payload_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap() / (1 << 20), 16);
    doc
}

const ARCHIVE_SIGNAL_BYTES: usize = 1 << 20;
struct ArchiveSignal<'a> {
    output: &'a mut Cursor<Vec<u8>>,
    signal: Option<mpsc::SyncSender<()>>,
}
impl std::io::Write for ArchiveSignal<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = std::io::Write::write(self.output, bytes)?;
        if self.output.get_ref().len() >= ARCHIVE_SIGNAL_BYTES
            && let Some(signal) = self.signal.take()
        {
            signal.send(()).map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(self.output)
    }
}
impl std::io::Seek for ArchiveSignal<'_> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(self.output, position)
    }
}

#[test]
#[ignore = "manual optimized-test public mid-archive streaming cancellation measurement"]
fn native_archive_stream_external_cancellation_preserves_private_prefix() {
    let doc = archive_current();
    let before = doc.clone();
    for repeat in 0..3 {
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = mpsc::sync_channel(1);
        let mut output = Cursor::new(Vec::new());
        std::thread::scope(|scope| {
            let cancel_ref = &cancelled;
            let worker = scope.spawn(move || {
                rx.recv_timeout(Duration::from_secs(30)).unwrap();
                let signal = Instant::now();
                cancel_ref.store(true, Ordering::Release);
                signal
            });
            let stop = || cancelled.load(Ordering::Acquire);
            let result =
                crate::save_native_stream(&doc, ArchiveSignal { output: &mut output, signal: Some(tx) }, Default::default(), &Interrupt::cancel_only(&stop));
            let returned = Instant::now();
            let signal = worker.join().unwrap();
            assert!(matches!(result, Err(IoError::Cancelled)), "unexpected archive result");
            assert!(output.get_ref().len() >= ARCHIVE_SIGNAL_BYTES);
            assert_eq!(output.get_ref().get(..4), Some(b"PK\x03\x04".as_slice()));
            assert!(
                photocraft_format::zip::stream::Reader::new(Cursor::new(output.get_ref()), Default::default(), &Interrupt::NONE).is_err(),
                "private prefix is not a completed archive"
            );
            assert_eq!(doc, before);
            println!(
                "ARCHIVE_STREAM_CANCEL repeat={repeat} signal_to_error_ms={:.6} private_prefix_bytes={} source_unchanged=true",
                returned.duration_since(signal).as_secs_f64() * 1000.,
                output.get_ref().len()
            );
        });
    }
}

#[test]
#[ignore = "manual optimized-test observed archive reservation cancellation/refund measurement"]
fn native_archive_observed_external_cancellation_refunds() {
    use std::cell::Cell;
    struct Ledger {
        live: Cell<usize>,
        peak: Cell<usize>,
    }
    impl photocraft_format::PreparationObserver for Ledger {
        fn reserve(&self, bytes: usize) -> photocraft_format::Result<()> {
            let next = self.live.get().checked_add(bytes).ok_or_else(|| photocraft_format::FormatError::LimitExceeded("test archive ledger".into()))?;
            if next > 64 << 20 {
                return Err(photocraft_format::FormatError::LimitExceeded("test archive ledger".into()));
            }
            self.live.set(next);
            self.peak.set(self.peak.get().max(next));
            Ok(())
        }
        fn release(&self, bytes: usize) {
            assert!(bytes <= self.live.get());
            self.live.set(self.live.get() - bytes);
        }
    }
    struct SourceOwner<'a> {
        ledger: &'a Ledger,
        bytes: usize,
    }
    impl Drop for SourceOwner<'_> {
        fn drop(&mut self) {
            assert!(self.bytes <= self.ledger.live.get());
            self.ledger.live.set(self.ledger.live.get() - self.bytes);
        }
    }
    let doc = archive_current();
    let before = doc.clone();
    let source_bytes = crate::retained_native_input_bytes(&doc, Default::default(), &Interrupt::NONE).unwrap();
    for repeat in 0..3 {
        let ledger = Ledger { live: Cell::new(source_bytes), peak: Cell::new(source_bytes) };
        let source_owner = SourceOwner { ledger: &ledger, bytes: source_bytes };
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = mpsc::sync_channel(1);
        let mut output = Cursor::new(Vec::new());
        std::thread::scope(|scope| {
            let cancel_ref = &cancelled;
            let worker = scope.spawn(move || {
                rx.recv_timeout(Duration::from_secs(30)).unwrap();
                let signal = Instant::now();
                cancel_ref.store(true, Ordering::Release);
                signal
            });
            let stop = || cancelled.load(Ordering::Acquire);
            let result = photocraft_format::PcraftWriter::save_zip_stream_observed(
                &doc,
                &Default::default(),
                ArchiveSignal { output: &mut output, signal: Some(tx) },
                Default::default(),
                Default::default(),
                &Interrupt::cancel_only(&stop),
                &ledger,
            );
            let returned = Instant::now();
            let signal = worker.join().unwrap();
            assert!(matches!(result, Err(photocraft_format::FormatError::Cancelled)));
            assert_eq!(ledger.live.get(), source_bytes, "all writer reservations refunded while borrowed source stays charged");
            assert!(ledger.peak.get() > source_bytes);
            assert_eq!(doc, before);
            println!(
                "ARCHIVE_REFUND_CANCEL repeat={repeat} signal_to_error_ms={:.6} source_live_bytes={source_bytes} writer_refunded=true peak_bytes={} private_prefix_bytes={}",
                returned.duration_since(signal).as_secs_f64() * 1000.,
                ledger.peak.get(),
                output.get_ref().len()
            );
        });
        // Private output storage is externally owned and intentionally not included
        // in this writer observer; this is not an aggregate output/RSS claim.
        drop(output);
        drop(source_owner);
        assert_eq!(ledger.live.get(), 0);
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "manual optimized-test public mid-archive atomic-path cancellation measurement"]
fn native_archive_path_external_cancellation_preserves_destination() {
    use std::io::Read;
    let doc = archive_current();
    let before = doc.clone();
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::env::temp_dir().join(format!("photocraft-sol4-archive-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let destination = directory.join("existing.pcraft");
    std::fs::write(&destination, b"existing destination authority").unwrap();
    for repeat in 0..3 {
        let started = AtomicBool::new(false);
        let cancelled = AtomicBool::new(false);
        let visits = std::sync::atomic::AtomicUsize::new(0);
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            let cancel_ref = &cancelled;
            let worker = scope.spawn(move || {
                let bytes = rx.recv_timeout(Duration::from_secs(30)).unwrap();
                let signal = Instant::now();
                cancel_ref.store(true, Ordering::Release);
                (signal, bytes)
            });
            let stop = || {
                let visit = visits.fetch_add(1, Ordering::Relaxed);
                // Synchronize the phase, not a wall-clock throughput deadline. A
                // directory-watching thread alone can lose the race to rename.
                // Only actual ZIP bytes in the sibling file arm this notification.
                if !started.load(Ordering::Acquire) && visit.is_multiple_of(64) {
                    for entry in std::fs::read_dir(&directory).unwrap() {
                        let entry = entry.unwrap();
                        if entry.path() == destination {
                            continue;
                        }
                        // Query the open file, not cached directory enumeration
                        // metadata (which may remain stale during Windows writes).
                        let mut file = std::fs::File::open(entry.path()).unwrap();
                        let bytes = file.metadata().unwrap().len();
                        if bytes < ARCHIVE_SIGNAL_BYTES as u64 {
                            continue;
                        }
                        let mut header = [0; 4];
                        file.read_exact(&mut header).unwrap();
                        assert_eq!(&header, b"PK\x03\x04");
                        started.store(true, Ordering::Release);
                        tx.send(bytes).unwrap();
                        let deadline = Instant::now() + Duration::from_secs(2);
                        while !cancelled.load(Ordering::Acquire) {
                            assert!(Instant::now() < deadline, "external cancellation thread did not signal");
                            std::thread::yield_now();
                        }
                        break;
                    }
                }
                cancelled.load(Ordering::Acquire)
            };
            let result = crate::save_native_path(&doc, &destination, Default::default(), &Interrupt::cancel_only(&stop));
            let returned = Instant::now();
            let (signal, streamed_bytes) = worker.join().unwrap();
            assert!(matches!(result, Err(IoError::Cancelled)), "{result:?}");
            assert_eq!(std::fs::read(&destination).unwrap(), b"existing destination authority");
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
            assert_eq!(doc, before);
            println!(
                "ARCHIVE_PATH_CANCEL repeat={repeat} signal_to_error_ms={:.6} observed_temp_bytes={streamed_bytes} destination_unchanged=true extra_files=0",
                returned.duration_since(signal).as_secs_f64() * 1000.
            );
        });
    }
    std::fs::remove_file(&destination).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}
