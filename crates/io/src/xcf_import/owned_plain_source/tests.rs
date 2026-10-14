//! UNRUN owned plain-XCF handoff/rollback regression drafts.
use super::*;
use crate::xcf_fixture;
use photocraft_geom::TileCoord;
use photocraft_raster::checked_view::NativeTileBytes;
use std::sync::atomic::{AtomicUsize, Ordering};
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn pixels(precision: u32) -> Vec<u8> {
    match precision {
        150 => [64u8, 128, 192, 255].repeat(3),
        250 => [16384u16, 32768, 49152, u16::MAX].into_iter().cycle().take(12).flat_map(u16::to_be_bytes).collect(),
        350 => [0x12345678u32, 0x76543210, 0xabcdef01, u32::MAX].into_iter().cycle().take(12).flat_map(u32::to_be_bytes).collect(),
        650 => [-0.0f32, 0.25, 0.75, 1.0].into_iter().cycle().take(12).flat_map(f32::to_be_bytes).collect(),
        750 => [-0.0f64, 0.25, 0.75, 1.0].into_iter().cycle().take(12).flat_map(f64::to_be_bytes).collect(),
        _ => unreachable!(),
    }
}
#[test]
fn owned_words_default_and_full_padded_tile_match_actual_staged_resident_route() {
    for precision in [150, 250, 350, 650, 750] {
        let raw = xcf_fixture::image(12, precision, 0, 3, 1, &pixels(precision));
        let budget = photocraft_xcf::Budget::new(limits(), None);
        let mut source = Bytes(&raw);
        let resident = staged_source::import_staged_plain_source_with_budget(
            "same",
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            photocraft_xcf::PlainSourceAccounting::BorrowedMemory,
        )
        .unwrap();
        let owned = import_owned_staged_plain_bytes("same", Arc::new(raw), &DecodeOptions::default(), &limits(), &Interrupt::NONE).unwrap();
        let LayerContent::BackedRaster(backed) = &owned.document().document().layers[0].content else { panic!() };
        let original = resident.document.layers[0].surface().unwrap();
        assert_eq!(backed.format(), original.format());
        assert_eq!(backed.default_bytes(), original.default_bytes());
        let pin = backed.checked_view().pin_tile(TileCoord::new(0, 0), &control()).unwrap();
        let NativeTileBytes::FullTile(bytes) = pin.encoded() else { panic!() };
        assert_eq!(bytes, original.tile(TileCoord::new(0, 0)).unwrap().bytes());
        assert_eq!(owned.document().document().source_color_context, resident.document.source_color_context);
        assert_eq!(owned.document().document().icc_profile, resident.document.icc_profile);
        assert_eq!(owned.origin(), resident.origin);
        assert!(owned.source_read_only());
    }
}
#[test]
fn encoded_owner_and_original_receipts_live_through_shared_history_model() {
    let bytes = Arc::new(xcf_fixture::image(12, 150, 0, 3, 1, &pixels(150)));
    let weak = Arc::downgrade(&bytes);
    let owned = import_owned_staged_plain_bytes("held", bytes, &DecodeOptions::default(), &limits(), &Interrupt::NONE).unwrap();
    let history = owned.document().clone();
    let quota = history.quota().clone();
    drop(owned);
    assert!(weak.upgrade().is_some());
    assert!(quota.usage().unwrap().resident > 0);
    let mut pixel = [0; 4];
    let LayerContent::BackedRaster(layer) = &history.document().layers[0].content else { panic!() };
    layer.checked_view().read_logical_pixel_native_into(0, 0, &mut pixel, &control()).unwrap();
    assert_eq!(pixel, [64, 128, 192, 255]);
    drop(history);
    assert!(weak.upgrade().is_none());
    assert_eq!(quota.usage().unwrap().scratch, 0);
}
#[test]
fn zero_lower_limit_and_cancel_are_structured_xcf_failures_not_native_format_errors() {
    let bytes = Arc::new(xcf_fixture::image(12, 150, 0, 3, 1, &pixels(150)));
    let zero = limits().lowered(&[("residentBytes", 0)]).unwrap();
    let error = import_owned_staged_plain_bytes("limited", bytes.clone(), &DecodeOptions::default(), &zero, &Interrupt::NONE).unwrap_err();
    assert!(matches!(error, IoError::Xcf(XcfError { kind: ErrorKind::Limit, .. })));
    let cancel = || true;
    let ctl = Interrupt::cancel_only(&cancel);
    assert!(matches!(import_owned_staged_plain_bytes("cancel", bytes, &DecodeOptions::default(), &limits(), &ctl), Err(IoError::Cancelled)));
}
#[test]
fn staged_batch_failures_destroy_payload_before_refunding_actual_receipts() {
    // Sweep real cancellation polls across handle admission, source detach and
    // later Surface detach; successful endpoints are also dropped privately.
    for allowance in 0..96 {
        let calls = AtomicUsize::new(0);
        let threshold = AtomicUsize::new(usize::MAX);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= threshold.load(Ordering::Relaxed);
        let budget = photocraft_xcf::Budget::new_persistent(limits(), Some(&cancel)).unwrap();
        let baseline = budget.used(BudgetKind::Resident);
        let bytes = Arc::new(xcf_fixture::image(12, 150, 0, 3, 1, &pixels(150)));
        let weak = Arc::downgrade(&bytes);
        let completed = {
            let mut source = Bytes(&bytes);
            staged_source::complete_staged_plain_source(
                "transfer",
                &mut source,
                &Interrupt::NONE,
                &DecodeOptions::default(),
                &budget,
                photocraft_xcf::PlainSourceAccounting::BorrowedMemory,
            )
            .unwrap()
        };
        let mut extra = budget.reserve(BudgetKind::Resident, 128, "test encoded header").unwrap();
        threshold.store(calls.load(Ordering::Relaxed) + allowance, Ordering::Relaxed);
        let outcome = completed.detach_owned(bytes, &mut [&mut extra], &budget);
        if let Err(error) = &outcome {
            assert!(matches!(error, IoError::Cancelled | IoError::Xcf(XcfError { kind: ErrorKind::Cancelled, .. })));
        }
        drop(outcome);
        drop(extra);
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.used(BudgetKind::Resident), baseline);
    }
}
#[test]
fn owned_endpoint_does_not_fallback_for_linear_source_outside_closed_staged_class() {
    let bytes = xcf_fixture::image(12, 100, 0, 3, 1, &pixels(150));
    assert!(import_owned_staged_plain_bytes("linear", Arc::new(bytes), &DecodeOptions::default(), &limits(), &Interrupt::NONE).is_err());
}

#[test]
fn prepaid_phase_cannot_reset_or_exceed_original_work_ceiling() {
    let limits = limits().lowered(&[("sampleWorkUnits", 4096)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    prepay_phase_work(&budget, 4096).unwrap();
    assert!(matches!(prepay_phase_work(&budget, 1), Err(IoError::Xcf(XcfError { kind: ErrorKind::Limit, .. }))));
    assert_eq!(budget.used(BudgetKind::Work), 4096);
}
#[test]
fn private_native_validation_retains_serial_original_poll_at_each_row() {
    let surface = Surface::try_from_interleaved(PixelFormat::RGBA8, Rect::new(0, 0, 3, 1), &[0; 12]).unwrap();
    let calls = AtomicUsize::new(0);
    let poll = || if calls.fetch_add(1, Ordering::Relaxed) >= 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(matches!(store::validate(&surface, Rect::new(0, 0, 3, 1), &control(), &poll), Err(BackingError::Cancelled)));
    assert!(calls.load(Ordering::Relaxed) >= 5);
}
