//! UNRUN portable capture/archive lifecycle and raw-storage regression drafts.
use super::super::{Family, Request, WORK};
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Size;
use photocraft_raster::{
    Interrupt, Surface,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use std::sync::atomic::{AtomicU64, Ordering};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota(index: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 512 << 20, index, scratch: 0 })
}
fn capture(request: Request, bytes: &[u8]) -> Capture {
    let mut builder = request.begin_capture("browser-label.pcraft", bytes.len()).unwrap();
    for part in bytes.chunks(MAX_READ) {
        builder.append(part).unwrap();
    }
    builder.finish().unwrap()
}

#[test]
fn immutable_capture_is_shared_without_copy_and_future_control_is_independent() {
    let q = quota(16 << 20);
    let request = Request::with_family(q.clone(), Family::Native).unwrap();
    let captured = capture(request.clone(), &[1, 3, 5, 7]);
    let identity = Arc::as_ptr(&captured.bytes);
    let source = source(captured, &control()).unwrap();
    assert_eq!(Arc::as_ptr(&source.capture.bytes), identity);
    request.work.cancelled.store(true, Ordering::Relaxed);
    request.work.used.store(WORK, Ordering::Relaxed);
    drop(request);
    let held = q.usage().unwrap();
    let work = AtomicU64::new(0);
    let charge = |units| {
        work.fetch_add(units, Ordering::Relaxed);
        Ok(())
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let mut output = [0; 4];
    source.read_exact_at(0, &mut output, &ctl).unwrap();
    assert_eq!(output, [1, 3, 5, 7]);
    assert_eq!(work.load(Ordering::Relaxed), 5);
    assert_eq!(q.usage().unwrap(), held);
    assert!(source.validate_quota(&quota(16 << 20)).is_err());
    source.validate_quota(&q).unwrap();
    drop(source);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn failed_bounded_reads_preserve_output_and_consume_only_caller_work() {
    let q = quota(16 << 20);
    let source = source(capture(Request::with_family(q.clone(), Family::Native).unwrap(), &[11; 8]), &control()).unwrap();
    let mut output = [99; 8];
    let charge = |units| if units > 1 { Err(BackingError::Work) } else { Ok(()) };
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert_eq!(source.read_exact_at(0, &mut output, &denied), Err(BackingError::Work));
    assert_eq!(output, [99; 8]);
    let cancelled = Interrupt::cancel_only(&|| true);
    let cancelled = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
    assert_eq!(source.read_exact_at(0, &mut output, &cancelled), Err(BackingError::Cancelled));
    assert_eq!(output, [99; 8]);
    for offset in [1, u64::MAX] {
        assert!(source.read_exact_at(offset, &mut output, &control()).is_err());
        assert_eq!(output, [99; 8]);
    }
    let mut oversized = vec![99; MAX_READ + 1];
    assert!(source.read_exact_at(0, &mut oversized, &control()).is_err());
    assert!(oversized.iter().all(|v| *v == 99));
    drop(source);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn owned_load_preserves_native_words_saved_title_and_payload_until_last_label_guard() {
    for (sample, pixel) in [
        (SampleType::U16, [1u16, 32769, 65534, 65535].into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>()),
        (SampleType::F32, [-0.0f32, 0.125, 0.75, 1.0].into_iter().flat_map(f32::to_ne_bytes).collect()),
        (SampleType::F64, [-0.0f64, 1e-200, 0.75, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()),
    ] {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut doc = Document::new("persisted title", Size::new(2, 1), ColorMode::Rgb, sample);
        doc.layers.push(Layer::new("raw", LayerContent::Raster(Surface::try_with_default_bytes(format, &pixel).unwrap())));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let q = quota(16 << 20);
        let input = load(capture(Request::with_family(q.clone(), Family::Native).unwrap(), &bytes), &control()).unwrap();
        let (label, archive) = input.into_parts();
        assert_eq!(label.name(), "browser-label.pcraft");
        assert_eq!(archive.document().name, "persisted title");
        assert_eq!(archive.document().layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), pixel.repeat(2));
        assert!(archive.quota().shares_ledger(&q));
        // Cancelling the picker after load cannot invalidate lazy native reads.
        label.source.capture.request.work.cancelled.store(true, Ordering::Relaxed);
        let mut raw = [0; 4];
        label.source.read_exact_at(0, &mut raw, &control()).unwrap();
        assert_eq!(raw.as_slice(), &bytes[..4]);
        drop(archive);
        assert!(q.usage().unwrap().resident >= bytes.len() as u64);
        drop(label);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn header_quota_and_decode_failures_release_capture_only_after_payload_drop() {
    let q = quota(4096);
    let request = Request::with_family(q.clone(), Family::Native).unwrap();
    let baseline = q.usage().unwrap();
    let captured = capture(request.clone(), &[1; 8]);
    assert!(source(captured, &control()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    drop(request);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());

    let q = quota(16 << 20);
    assert!(load(capture(Request::with_family(q.clone(), Family::Native).unwrap(), &[1; 8]), &control()).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let cancelled = Interrupt::cancel_only(&|| true);
    let ctl = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
    assert!(load(capture(Request::with_family(q.clone(), Family::Native).unwrap(), &[1; 8]), &ctl).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
