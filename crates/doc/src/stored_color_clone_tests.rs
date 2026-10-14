use super::*;
fn values() -> Vec<FilterArgument> {
    let mut samples = Vec::with_capacity(2048);
    samples.extend([0x80000000u32, 0x7fc00042, 0x3f800000].into_iter().flat_map(u32::to_le_bytes));
    vec![FilterArgument {
        name: "raw".into(),
        value: FilterValue::StoredColor(ExactColorStorage { layout: "HSL float".into(), samples, opaque_tail: vec![255; 65_537] }),
    }]
}
#[test]
fn controlled_copy_retains_raw_words_and_census_uses_actual_capacity() {
    let input = values();
    let copy = arguments(&input, 0, &Interrupt::NONE).unwrap();
    assert_eq!(copy, input);
    let mut census = Census { control: &Interrupt::NONE, model: 0, payload: 0, work: 0, visits: 0, seen: HashSet::new() };
    census.arguments(&input, 0).unwrap();
    let FilterValue::StoredColor(raw) = &input[0].value else { panic!("own fixture") };
    assert!(
        census.model
            >= (input.capacity() * size_of::<FilterArgument>()
                + input[0].name.capacity()
                + raw.layout.capacity()
                + raw.samples.capacity()
                + raw.opaque_tail.capacity()) as u64
    );
    assert!(census.work >= (raw.samples.len() + raw.opaque_tail.len()) as u64);
}
#[test]
fn cancellation_during_private_tail_copy_preserves_caller_data() {
    let input = values();
    let before = input.clone();
    let calls = std::cell::Cell::new(0usize);
    let stop = || {
        let n = calls.get();
        calls.set(n + 1);
        n >= 8
    };
    assert!(arguments(&input, 0, &Interrupt::cancel_only(&stop)).is_err());
    assert_eq!(input, before);
    assert!(calls.get() > 8);
}
#[test]
fn oversized_or_malformed_raw_storage_is_not_cloned() {
    for raw in [
        ExactColorStorage { layout: "bad\0layout".into(), samples: vec![1], opaque_tail: vec![] },
        ExactColorStorage { layout: "HSL float".into(), samples: vec![0; 1025], opaque_tail: vec![] },
        ExactColorStorage { layout: "HSL float".into(), samples: vec![1], opaque_tail: vec![0; (1 << 20) + 1] },
    ] {
        assert!(arguments(&[FilterArgument { name: "raw".into(), value: FilterValue::StoredColor(raw) }], 0, &Interrupt::NONE).is_err());
    }
}
