//! UNRUN real finite scratch initialization/capacity boundary draft.
use super::*;

#[test]
fn finite_scratch_checks_actual_capacity_before_bounded_initialization() {
    for count in [0, 1, 257, 4097] {
        let values = allocate_interrupt(count, &Interrupt::NONE).unwrap();
        assert_eq!(values.len(), count);
        assert!(values.capacity() <= count * 2);
        assert!(values.iter().all(|value| value.to_bits() == 0));
    }
    // An actual reserved but uninitialized buffer can be rejected by the same
    // guard without a write. This models an allocator capacity larger than the
    // declared request rather than claiming a deterministic allocator policy.
    let mut raw = Vec::<f64>::new();
    raw.try_reserve_exact(16).unwrap();
    assert_eq!(raw.len(), 0);
    assert_eq!(checked_allocation_capacity::<f64>(raw.capacity(), 1), Err(RasterInputError::LimitExceeded));
    assert_eq!(raw.len(), 0);
    assert_eq!(allocate_interrupt(usize::MAX, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
    let stopped = || true;
    let interrupt = Interrupt::cancel_only(&stopped);
    assert_eq!(allocate_interrupt(4097, &interrupt), Err(RasterInputError::Cancelled));
}
