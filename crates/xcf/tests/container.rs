mod common;
use photocraft_xcf::*;
use std::io::Write;
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
const BZIP: &[u8] = &[
    66, 90, 104, 57, 49, 65, 89, 38, 83, 89, 33, 3, 184, 239, 0, 0, 10, 241, 128, 96, 0, 64, 0, 64, 0, 11, 166, 64, 64, 32, 0, 49, 0, 208, 0, 209, 12, 53, 61,
    16, 252, 14, 204, 37, 98, 36, 207, 208, 200, 209, 119, 36, 83, 133, 9, 2, 16, 59, 142, 240,
];
const XZ: &[u8] = &[
    253, 55, 122, 88, 90, 0, 0, 4, 230, 214, 180, 70, 2, 0, 33, 1, 22, 0, 0, 0, 116, 47, 229, 163, 224, 0, 41, 0, 23, 93, 0, 51, 154, 73, 251, 227, 151, 228,
    69, 84, 21, 229, 181, 163, 175, 81, 216, 6, 189, 218, 183, 44, 96, 0, 0, 0, 237, 218, 95, 237, 144, 170, 238, 228, 0, 1, 51, 42, 205, 25, 86, 15, 31, 182,
    243, 125, 1, 0, 0, 0, 0, 4, 89, 90,
];
#[test]
fn wrapper_transient_leases_are_visible_and_cancelled_cleanup_releases_them() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let wrapped = gzip(&plain);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let stop = || budget.used(BudgetKind::Decoder) > 0;
    let options = DecodeOptions { cancelled: Some(&stop), ..Default::default() };
    assert_eq!(decode_candidate_with_budget(&wrapped, &options, &budget, SourceAccounting::Borrowed).unwrap_err().kind, ErrorKind::Cancelled);
    assert!(budget.peak(BudgetKind::Decoder) > 0);
    for kind in [BudgetKind::Resident, BudgetKind::Decoder, BudgetKind::Index, BudgetKind::Encoded] {
        assert_eq!(budget.used(kind), 0);
    }
}
#[test]
fn memory_tile_scratch_does_not_consume_forbidden_wasm_disk_scratch() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let limits = ResourceLimits::for_policy(ResourcePolicy::WasmStandard).unwrap();
    assert_eq!(limits.get(BudgetKind::Scratch), 0);
    assert!(decode_with_resources(&plain, &Default::default(), &limits).is_ok());
}
#[test]
fn parser_retained_allocations_block_simultaneous_callback_admission() {
    let plain = common::image(12, 150, 2, 1, 1, &[1, 2, 3, 255]);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", 1 << 20)]).unwrap();
    let budget = Budget::new(limits, None);
    let observed = std::cell::Cell::new(false);
    let progress = |_: f32| {
        if budget.used(BudgetKind::Resident) > plain.len() as u64 {
            observed.set(true);
            assert!(budget.reserve(BudgetKind::Resident, (1 << 20) - plain.len() as u64, "other concurrent output").is_err());
        }
    };
    let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
    let decoded = decode_candidate_with_budget(&plain, &options, &budget, SourceAccounting::Borrowed).unwrap().unwrap();
    assert!(observed.get());
    assert!(budget.peak(BudgetKind::Decoder) >= 128 * 1024);
    assert!(budget.peak(BudgetKind::Resident) > budget.used(BudgetKind::Resident));
    assert_eq!(budget.used(BudgetKind::Decoder), 0);
    drop(decoded);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}
#[test]
fn all_wrappers_are_content_detected_and_preserve_logical_samples() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let f = decode(&gzip(&plain), &Default::default()).unwrap();
    assert_eq!(f.layers[0].plane.as_ref().unwrap().bytes, [1, 2, 3, 255]);
    for bytes in [BZIP, XZ] {
        let f = decode(bytes, &Default::default()).unwrap();
        assert_eq!((f.width, f.height), (1, 1));
    }
}
#[test]
fn content_candidate_distinguishes_svgz_and_keeps_actual_xcf_errors() {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    assert!(decode_candidate(&gzip(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), &Default::default(), &limits).unwrap().is_none());
    assert!(decode_candidate(&gzip(b"gimp xcf bad"), &Default::default(), &limits).is_err());
    assert!(decode_candidate(&gzip(&gzip(b"gimp xcf bad")), &Default::default(), &limits).is_err());
}
#[test]
fn wrapper_checksums_trailing_and_recursive_wrappers_fail() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for original in [gzip(&plain), BZIP.to_vec(), XZ.to_vec()] {
        let mut damaged = original.clone();
        let mid = damaged.len() / 2;
        damaged[mid] ^= 128;
        assert!(decode(&damaged, &Default::default()).is_err());
        let mut trailing = original.clone();
        trailing.push(1);
        assert!(decode(&trailing, &Default::default()).is_err());
        for end in [1, original.len() / 2, original.len() - 1] {
            assert!(decode(&original[..end], &Default::default()).is_err());
        }
    }
    let mut bad = gzip(&plain);
    let end = bad.len();
    bad[end - 8] ^= 1;
    assert!(decode(&bad, &Default::default()).is_err());
    assert!(decode(&gzip(&gzip(&plain)), &Default::default()).is_err());
}
#[test]
fn concatenated_members_form_one_logical_xcf_and_policy_denies_expansion() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let split = plain.len() / 2;
    let mut members = gzip(&plain[..split]);
    members.extend(gzip(&plain[split..]));
    assert!(decode(&members, &Default::default()).is_ok());
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("logicalBytes", plain.len() as u64 - 1)]).unwrap();
    assert_eq!(decode_with_resources(&members, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("decoderBytes", 0)]).unwrap();
    assert_eq!(decode_with_resources(&members, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
}

#[test]
fn cancellation_resident_and_sample_work_are_transactional() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let plain = common::image(12, 750, 0, 64, 64, &vec![0; 64 * 64 * 4 * 8]);
    let compressed = gzip(&plain);
    let calls = AtomicUsize::new(0);
    let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 5;
    assert_eq!(decode(&compressed, &DecodeOptions { cancelled: Some(&stop), ..Default::default() }).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(calls.load(Ordering::Relaxed), 6);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", compressed.len() as u64)]).unwrap();
    assert_eq!(decode_with_resources(&compressed, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
    for work in [0, 16383] {
        let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", work)]).unwrap();
        assert_eq!(decode_with_resources(&plain, &Default::default(), &limits).unwrap_err().kind, ErrorKind::Limit);
    }
    let exact = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", 16384)]).unwrap();
    assert!(decode_with_resources(&plain, &Default::default(), &exact).is_ok());
    let legacy = DecodeOptions { limits: Limits { resident_bytes: compressed.len(), ..Default::default() }, ..Default::default() };
    assert_eq!(decode(&compressed, &legacy).unwrap_err().kind, ErrorKind::Limit);
}

#[test]
fn caller_snapshot_leases_count_once_and_survive_decoded_field_moves() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for (wrapped, bytes) in [(false, plain.clone()), (true, gzip(&plain))] {
        let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
        let budget = Budget::new(limits, None);
        let encoded = budget.reserve(BudgetKind::Encoded, bytes.len() as u64, "snapshot").unwrap();
        let resident = budget.reserve(BudgetKind::Resident, bytes.len() as u64, "snapshot").unwrap();
        budget.consume_work(9, "snapshot work").unwrap();
        let BudgetDecodedImport { decoded, mut reservations } =
            decode_candidate_with_budget(&bytes, &Default::default(), &budget, SourceAccounting::CallerReserved).unwrap().unwrap();
        assert_eq!(reservations.source_bytes(), 0);
        assert_eq!(reservations.encoded_bytes(), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), bytes.len() as u64);
        assert_eq!(budget.used(BudgetKind::Resident), decoded.file.charged_bytes as u64);
        let performed_work = budget.used(BudgetKind::Work);
        if wrapped {
            // The bounded prefix and authoritative full wrapper decoder both
            // retain their performed work on the original caller budget.
            assert!(performed_work > 13);
        } else {
            assert_eq!(performed_work, 13); // snapshot9 plus four decoded samples
        }
        let retained = reservations.retained_bytes();
        let plane = decoded.file.layers.into_iter().next().unwrap().plane.unwrap();
        assert_eq!(plane.bytes, [1, 2, 3, 255]);
        assert_eq!(budget.used(BudgetKind::Work), performed_work);
        reservations.resize_retained(retained + 4, "mapped output").unwrap();
        assert_eq!(budget.used(BudgetKind::Resident), bytes.len() as u64 + retained + 4);
        assert_eq!(budget.used(BudgetKind::Work), performed_work);
        drop(reservations);
        assert_eq!(budget.used(BudgetKind::Resident), bytes.len() as u64);
        assert_eq!(budget.used(BudgetKind::Work), performed_work);
        drop(resident);
        drop(encoded);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Work), performed_work);
    }
}
#[test]
fn shared_policy_denials_are_atomic_and_preserve_performed_work() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let wrapped = gzip(&plain);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let absent = Budget::new(limits.clone(), None);
    assert_eq!(decode_candidate_with_budget(&plain, &Default::default(), &absent, SourceAccounting::CallerReserved).unwrap_err().kind, ErrorKind::Limit);
    let limits = limits.lowered(&[("residentBytes", wrapped.len() as u64), ("sampleWorkUnits", 9)]).unwrap();
    let budget = Budget::new(limits, None);
    let _encoded = budget.reserve(BudgetKind::Encoded, wrapped.len() as u64, "snapshot").unwrap();
    let _resident = budget.reserve(BudgetKind::Resident, wrapped.len() as u64, "snapshot").unwrap();
    budget.consume_work(9, "snapshot").unwrap();
    assert_eq!(decode_candidate_with_budget(&wrapped, &Default::default(), &budget, SourceAccounting::CallerReserved).unwrap_err().kind, ErrorKind::Limit);
    assert_eq!(budget.used(BudgetKind::Resident), wrapped.len() as u64);
    assert_eq!(budget.used(BudgetKind::Work), 9);
}

#[test]
fn caller_work_and_decoder_capacity_remain_aggregate() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let wrapped = gzip(&plain);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", 9)]).unwrap();
    let budget = Budget::new(limits, None);
    budget.consume_work(9, "snapshot").unwrap();
    assert_eq!(decode_candidate_with_budget(&plain, &Default::default(), &budget, SourceAccounting::Borrowed).unwrap_err().kind, ErrorKind::Limit);
    assert_eq!(budget.used(BudgetKind::Work), 9);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let budget = Budget::new(limits.clone(), None);
    let _decoder = budget.reserve(BudgetKind::Decoder, limits.get(BudgetKind::Decoder), "other decoder").unwrap();
    assert_eq!(decode_candidate_with_budget(&wrapped, &Default::default(), &budget, SourceAccounting::Borrowed).unwrap_err().kind, ErrorKind::Limit);
    assert_eq!(budget.used(BudgetKind::Decoder), limits.get(BudgetKind::Decoder));
}
#[test]
fn options_cancellation_remains_active_with_independent_caller_budget() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let wrapped = gzip(&plain);
    let polls = std::cell::Cell::new(0);
    let cancel = || {
        polls.set(polls.get() + 1);
        polls.get() >= 2
    };
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let options = DecodeOptions { cancelled: Some(&cancel), ..Default::default() };
    assert_eq!(decode_candidate_with_budget(&wrapped, &options, &budget, SourceAccounting::Borrowed).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

#[test]
fn inner_tile_dictionary_and_memory_respect_other_caller_components() {
    let plain = common::image(12, 150, 2, 1, 1, &[1, 2, 3, 255]);
    // Resident tile memory is not the separate on-disk Scratch allowance.
    for kind in [BudgetKind::Decoder, BudgetKind::Resident] {
        let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
        let budget = Budget::new(limits.clone(), None);
        let _other = budget.reserve(kind, limits.get(kind), "other consumer").unwrap();
        assert_eq!(decode_candidate_with_budget(&plain, &Default::default(), &budget, SourceAccounting::Borrowed).unwrap_err().kind, ErrorKind::Limit);
        assert_eq!(budget.used(kind), limits.get(kind));
    }
}

#[test]
fn lowered_options_cover_caller_allocations_during_progress() {
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let lease = std::cell::RefCell::new(None);
    let progress = |_| {
        if lease.borrow().is_none() {
            *lease.borrow_mut() = Some(budget.reserve(BudgetKind::Resident, 1 << 20, "other progress consumer").unwrap());
        }
    };
    let options = DecodeOptions { limits: Limits { resident_bytes: 1 << 20, ..Default::default() }, progress: Some(&progress), ..Default::default() };
    assert_eq!(decode_candidate_with_budget(&plain, &options, &budget, SourceAccounting::Borrowed).unwrap_err().kind, ErrorKind::Limit);
    assert_eq!(budget.used(BudgetKind::Resident), 1 << 20);
}

#[test]
fn non_xcf_prefix_stops_without_full_output_but_claimed_xcf_keeps_full_errors() {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("logicalBytes", 64), ("residentBytes", 2 << 20)]).unwrap();
    let svg = [b"<svg xmlns".as_slice(), &vec![b' '; 1 << 20]].concat();
    let wrapped = gzip(&svg);
    let budget = Budget::new(limits.clone(), None);
    assert!(decode_candidate_with_budget(&wrapped, &Default::default(), &budget, SourceAccounting::Borrowed).unwrap().is_none());
    assert!(budget.peak(BudgetKind::Resident) < 1 << 20, "never allocate full non-XCF output");
    assert!(budget.used(BudgetKind::Work) > 0);
    for kind in [BudgetKind::Resident, BudgetKind::Decoder, BudgetKind::Index, BudgetKind::Encoded] {
        assert_eq!(budget.used(kind), 0);
    }
    let denied = limits.lowered(&[("sampleWorkUnits", 0)]).unwrap();
    assert_eq!(decode_candidate(&wrapped, &Default::default(), &denied).unwrap_err().kind, ErrorKind::Limit);
    let stop = || true;
    assert_eq!(decode_candidate(&wrapped, &DecodeOptions { cancelled: Some(&stop), ..Default::default() }, &limits).unwrap_err().kind, ErrorKind::Cancelled);
    let plain = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    for original in [gzip(&plain), BZIP.to_vec(), XZ.to_vec()] {
        let mut trailing = original.clone();
        trailing.push(1);
        assert!(decode_candidate(&trailing, &Default::default(), &ResourceLimits::for_policy(Default::default()).unwrap()).is_err());
        let truncated = original.get(..original.len() - 1).unwrap();
        assert!(decode_candidate(truncated, &Default::default(), &ResourceLimits::for_policy(Default::default()).unwrap()).is_err());
    }
}
