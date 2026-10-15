//! UNRUN bounded raw storage helper drafts. Input fixtures are external test
//! values; copy tests explicitly hold their prospective destination quota fee.
use super::*;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLimits};
use std::{
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering},
};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn arg(name: &str, value: FilterValue) -> FilterArgument {
    FilterArgument { name: name.into(), value }
}
fn f(bits: u64) -> FilterValue {
    FilterValue::Number(ExactNumber::Float64Bits(bits))
}
fn config(n: usize, inner: usize) -> FilterValue {
    FilterValue::Config(
        (0..n)
            .map(|channel| {
                arg(&format!("channel-{channel}"), FilterValue::Config((0..inner).map(|i| arg(&format!("word-{i}"), f((i as f64).to_bits()))).collect()))
            })
            .collect(),
    )
}
fn curves() -> FilterValue {
    FilterValue::Config(
        (0..5)
            .map(|i| {
                arg(
                    &format!("channel-{i}"),
                    FilterValue::Curve { free: true, points_f64_bits: Vec::new(), point_types: Vec::new(), samples_f64_bits: vec![(-0f64).to_bits(); 256] },
                )
            })
            .collect(),
    )
}

#[test]
fn actual_configured_storage_shapes_have_exact_global_word_counts_and_capacity_costs() {
    for (value, words, entries, arrays) in [(config(3, 3), 9, 12, 0), (config(5, 5), 25, 30, 0), (curves(), 1280, 5, 15)] {
        let arguments = [arg("config", value)];
        let shape = point_argument_shape(&arguments, &ctl()).unwrap();
        assert_eq!(shape.f64_words, words);
        assert_eq!(shape.config_entries, entries);
        assert_eq!(shape.curve_arrays, arrays);
        assert!(shape.utf8_bytes > 0);
        assert!(shape.copy_index_bytes >= shape.input_index_bytes);
    }
    let mut samples = Vec::with_capacity(400);
    samples.resize(256, 0x7ff8000012345678);
    let curve = FilterValue::Curve { free: true, points_f64_bits: Vec::with_capacity(31), point_types: Vec::with_capacity(17), samples_f64_bits: samples };
    let shape = point_value_shape(&curve, &ctl()).unwrap();
    assert_eq!(shape.f64_words, 256);
    let FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } = &curve else { panic!() };
    assert_eq!(
        shape.input_index_bytes,
        (points_f64_bits.capacity() * size_of::<[u64; 2]>() + point_types.capacity() * size_of::<i32>() + samples_f64_bits.capacity() * size_of::<u64>())
            as u64
    );
    assert_eq!(shape.copy_index_bytes, 512 * size_of::<u64>() as u64);
}

#[test]
fn depth_entries_arrays_and_cross_argument_word_limits_refuse_before_copy() {
    let deep = FilterValue::Config(vec![arg("one", FilterValue::Config(vec![arg("two", FilterValue::Config(vec![]))]))]);
    let oversized = FilterValue::Curve { free: true, points_f64_bits: vec![], point_types: vec![], samples_f64_bits: vec![0; 257] };
    for value in
        [deep, config(6, 1), oversized, FilterValue::String("inert unsupported".into()), FilterValue::Number(ExactNumber::Unsigned { width: 32, value: 1 })]
    {
        assert!(point_value_shape(&value, &ctl()).is_err());
        assert!(copy_value(&value, &ctl()).is_err());
    }
    let full = [arg("config", curves()), arg("extra", f(0))];
    assert!(matches!(point_argument_shape(&full, &ctl()), Err(BackingError::Length)));
    let arguments = (0..11).map(|i| arg(&format!("arg-{i}"), FilterValue::Bool(false))).collect::<Vec<_>>();
    assert!(matches!(point_argument_shape(&arguments, &ctl()), Err(BackingError::Length)));
}

#[test]
fn raw_equality_preserves_signed_zero_nan_bits_order_and_representation_shape() {
    let a = FilterValue::Config(vec![arg("first", f((-0f64).to_bits())), arg("second", f(0x7ff8000012345678))]);
    let b = FilterValue::Config(vec![arg("first", f(0)), arg("second", f(0x7ff8000012345678))]);
    assert!(point_values_same_shape(&a, &b, &ctl()).unwrap());
    assert!(!point_values_equal(&a, &b, &ctl()).unwrap());
    assert!(point_values_equal(&a, &a, &ctl()).unwrap());
    let swapped = FilterValue::Config(vec![arg("second", f(0x7ff8000012345678)), arg("first", f((-0f64).to_bits()))]);
    assert!(!point_values_same_shape(&a, &swapped, &ctl()).unwrap());
    let mut free = curves();
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 });
    let _fee = quota.reserve(QuotaKind::Index, point_value_shape(&free, &ctl()).unwrap().copy_index_bytes).unwrap();
    let copied = copy_value(&free, &ctl()).unwrap();
    let FilterValue::Config(channels) = &mut free else { panic!() };
    let FilterValue::Curve { free, .. } = &mut channels[0].value else { panic!() };
    *free = false;
    assert!(!point_values_same_shape(&FilterValue::Config(channels.clone()), &copied, &ctl()).unwrap());
}

#[test]
fn paid_fallible_copy_keeps_bits_and_bounded_capacity_then_refunds() {
    let value = curves();
    let shape = point_value_shape(&value, &ctl()).unwrap();
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 });
    let before = quota.usage().unwrap();
    let fee = quota.reserve(QuotaKind::Index, shape.copy_index_bytes).unwrap();
    let copied = copy_value(&value, &ctl()).unwrap();
    assert!(point_values_equal(&value, &copied, &ctl()).unwrap());
    let actual = point_value_shape(&copied, &ctl()).unwrap();
    assert!(actual.input_index_bytes <= fee.bytes());
    let FilterValue::Config(channels) = &copied else { panic!() };
    for c in channels {
        let FilterValue::Curve { samples_f64_bits, .. } = &c.value else { panic!() };
        assert!(samples_f64_bits.capacity() >= 256 && samples_f64_bits.capacity() <= 512);
        assert!(samples_f64_bits.iter().all(|v| *v == (-0f64).to_bits()));
    }
    drop(copied);
    drop(fee);
    assert_eq!(quota.usage().unwrap(), before);
}

#[test]
fn caller_work_and_cancellation_cover_shape_equality_and_actual_copy_without_reset() {
    let value = curves();
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 });
    let _fee = quota.reserve(QuotaKind::Index, point_value_shape(&value, &ctl()).unwrap().copy_index_bytes).unwrap();
    let work = AtomicU64::new(0);
    let charge = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    point_value_shape(&value, &control).unwrap();
    let first = work.load(Ordering::Relaxed);
    let copied = copy_value(&value, &control).unwrap();
    assert!(work.load(Ordering::Relaxed) > first);
    assert!(point_values_equal(&value, &copied, &control).unwrap());
    // Release the first output before using the same held envelope for the
    // later private partial-copy cancellation attempt.
    drop(copied);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(copy_value(&value, &deny), Err(BackingError::Work)));
    let calls = AtomicU64::new(0);
    // Shape requires approximately 1300 visits; cancellation after that point
    // exercises a private partial copy, without mutating the original value.
    let cancelled = || calls.load(Ordering::Relaxed) > 1500;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let debit = |n| {
        calls.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    assert!(matches!(copy_value(&value, &ReadControl { interrupt: &interrupt, work: &debit }), Err(BackingError::Cancelled)));
    assert_eq!(point_value_shape(&value, &ctl()).unwrap().f64_words, 1280);
    let local = Cell::new(0u64);
    let debit_local = |n| {
        local.set(local.get() + n);
        Ok(())
    };
    point_argument_shape_with(&[arg("config", value)], &Interrupt::NONE, &debit_local).unwrap();
    assert!(local.get() >= 1280);
}
