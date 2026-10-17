//! TEMP counted census regressions; uncompiled/unrun.
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Size;
use std::cell::Cell;
#[test]
fn actual_counted_visitors_match_old_receipts_and_refuse_before_first_identity() {
    let mut doc = Document::new("original", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("sparse native", doc.pixel_format()));
    let before = doc.clone();
    let work = Cell::new(0u64);
    let debit = |n| {
        work.set(work.get().checked_add(n).unwrap());
        Ok(())
    };
    let model = retained_native_model_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &debit).unwrap();
    let payload = retained_native_payload_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &debit).unwrap();
    assert_eq!(model.retained_bytes(), retained_native_model_bytes(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap());
    assert_eq!(payload.retained_bytes(), retained_native_payload_bytes(&doc, PreparationLimits::default(), &Interrupt::NONE).unwrap());
    assert!(model.belongs_to(&doc) && payload.belongs_to(&doc));
    assert!(work.get() > 0);
    let denied = |_| Err(limit("original parent Work"));
    assert!(retained_native_model_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &denied).is_err());
    assert!(retained_native_payload_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &denied).is_err());
    assert_eq!(doc, before);
    let empty = Document::new("empty", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let remaining = Cell::new(64u64);
    let fixed = |n| {
        let next = remaining.get().checked_sub(n).ok_or_else(|| limit("selected header Work"))?;
        remaining.set(next);
        Ok(())
    };
    let receipt = retained_native_payload_census_with_work(&empty, PreparationLimits::default(), &Interrupt::NONE, &fixed).unwrap();
    assert!(receipt.belongs_to(&empty));
    assert_eq!(receipt.retained_bytes(), 0);
    assert_eq!(remaining.get(), 0);
    for model in [false, true] {
        remaining.set(63);
        let error = if model {
            retained_native_model_census_with_work(&empty, PreparationLimits::default(), &Interrupt::NONE, &fixed).err().unwrap()
        } else {
            retained_native_payload_census_with_work(&empty, PreparationLimits::default(), &Interrupt::NONE, &fixed).err().unwrap()
        };
        assert!(error.to_string().contains("selected header Work"));
        assert_eq!(remaining.get(), 63);
    }
}
#[test]
fn origin_and_repeated_identity_phases_charge_each_time_before_receipt() {
    let mut doc = Document::new("origin", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("one", doc.pixel_format());
    layer.surface_mut().unwrap().try_write_pixel_f64(0, 0, &[0.25, 0.5, 0.75, 1.0]).unwrap();
    doc.layers.push(layer);
    let count = Cell::new(0u64);
    let debit = |n| {
        count.set(count.get() + n);
        Ok(())
    };
    let _ = retained_native_payload_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &debit).unwrap();
    let once = count.get();
    let _ = retained_native_payload_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &debit).unwrap();
    assert_eq!(count.get(), once * 2);
    let calls = Cell::new(0usize);
    let refuse = |_| {
        calls.set(calls.get() + 1);
        if calls.get() >= 3 { Err(IoError::Cancelled) } else { Ok(()) }
    };
    assert!(matches!(retained_native_payload_census_with_work(&doc, PreparationLimits::default(), &Interrupt::NONE, &refuse), Err(IoError::Cancelled)));
    assert_eq!(calls.get(), 3);
}
