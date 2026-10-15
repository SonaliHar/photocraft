//! UNRUN wire-only serialization/accounting tests. Test-private reply construction
//! establishes no source/model eligibility; production callers cannot forge it.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits, QuotaUsage},
};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 1 << 20, index: 1 << 20, scratch: 0 })
}
fn reply(q: &BackingQuota) -> OwnedFilterInspectReply {
    let lease = q.reserve(QuotaKind::Index, 8192).unwrap();
    OwnedFilterInspectReply {
        value: json!({"filterId":"17","enabled":true,"name":"quote\" slash\\ newline\n λ","arguments":[{"name":"low","value":{"Number":{"Float32Bits":2147483648u32}}}]}),
        quota: q.clone(),
        _lease: lease,
    }
}
#[test]
fn rpc_id_and_result_have_exact_existing_envelope_and_hold_both_fees_until_drop() {
    let q = quota();
    let input = reply(&q);
    let expected = json!({"id":{"nested":["escaped\"",17,null]},"ok":true,"result":input.value()});
    let wire = encode_rpc(input, &expected["id"], 65536, &ctl()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(wire.bytes()).unwrap(), expected);
    assert_eq!(wire.bytes(), serde_json::to_vec(&expected).unwrap());
    assert!(q.usage().unwrap().index >= 8192);
    assert!(q.usage().unwrap().resident >= wire.bytes().len() as u64);
    let mut written = Vec::new();
    wire.write_line_to(&mut written).unwrap();
    assert_eq!(written, [wire.bytes(), b"\n"].concat());
    drop(wire);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn cli_pretty_and_compact_preserve_complete_record_without_result_extraction() {
    for compact in [false, true] {
        let q = quota();
        let input = reply(&q);
        let expected = json!({"command":"xcf.filter.inspect","result":input.value()});
        let wire = encode_cli(input, compact, 65536, &ctl()).unwrap();
        let expected = if compact { serde_json::to_vec(&expected).unwrap() } else { serde_json::to_vec_pretty(&expected).unwrap() };
        assert_eq!(wire.bytes(), expected);
        drop(wire);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn exact_newline_limit_hostile_ids_and_quota_work_cancel_fail_closed() {
    let q = quota();
    let wire = encode_rpc(reply(&q), &json!(7), 65536, &ctl()).unwrap();
    let len = wire.bytes().len();
    drop(wire);
    assert!(encode_rpc(reply(&q), &json!(7), len, &ctl()).is_err());
    drop(encode_rpc(reply(&q), &json!(7), len + 1, &ctl()).unwrap());
    let mut deep = Value::Null;
    for _ in 0..9 {
        deep = json!([deep]);
    }
    assert!(encode_rpc(reply(&q), &deep, 65536, &ctl()).is_err());
    assert!(encode_rpc(reply(&q), &json!("x".repeat((1 << 20) + 1)), 2 << 20, &ctl()).is_err());
    let blocked = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    assert!(encode_cli(reply(&q), true, 65536, &ctl()).is_err());
    drop(blocked);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(encode_cli(reply(&q), true, 65536, &deny).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(encode_rpc(reply(&q), &Value::Null, 65536, &ReadControl { interrupt: &cancelled, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn count_and_actual_encoding_share_cumulative_work_and_write_failure_keeps_owner() {
    let q = quota();
    let used = AtomicU64::new(0);
    let charge = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    let wire = encode_cli(reply(&q), true, 65536, &control).unwrap();
    assert!(used.load(Ordering::Relaxed) > 2 * wire.bytes().len() as u64);
    let held = q.usage().unwrap();
    struct Refuse;
    impl Write for Refuse {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("writer failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(wire.write_line_to(&mut Refuse).is_err());
    assert_eq!(q.usage().unwrap(), held);
    drop(wire);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn input_escaping_scans_are_paid_before_serde_output_callbacks() {
    let q = quota();
    let input = reply(&q);
    let allowance = input.charged_index_bytes().checked_mul(8).unwrap() + 1;
    let used = AtomicU64::new(0);
    let charge = |n| {
        used.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(n).filter(|next| *next <= allowance)).map_err(|_| BackingError::Work)?;
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert!(encode_rpc(input, &json!("\"".repeat(32)), 65536, &control).is_err());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    drop(encode_rpc(reply(&q), &json!("\"".repeat(32)), 65536, &ctl()).unwrap());
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
