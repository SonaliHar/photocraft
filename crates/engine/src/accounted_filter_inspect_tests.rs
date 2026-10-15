//! UNRUN inspection carrier drafts over independently saved, SHA-pinned originals.
//! No fabricated current metadata is used as original-XCF qualification.
use super::*;
use photocraft_doc::{ColorMode, Document, SampleType, Size};
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
use photocraft_xcf::{BudgetKind, DecodeOptions, Encoding, Model, ResourceLimits, Sample};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Evidence {
    imported: photocraft_io::OwnedXcfImport,
    bytes: Arc<Vec<u8>>,
    quota: BackingQuota,
    name: String,
    _capture: QuotaLease,
}
fn evidence(histogram: bool) -> Evidence {
    let (packet, pin, operation) = if histogram {
        ("threshold-precision-evidence", "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142", "gimp:threshold")
    } else {
        ("alpha-threshold-precision-evidence", "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263", "gimp:threshold-alpha")
    };
    let sample = Sample::F64;
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires pinned full-XCF evidence corpus")).join(packet);
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), pin);
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let proof_bytes = std::fs::read(root.join("fresh-verification.json")).unwrap();
    assert_eq!(hash(&proof_bytes), manifest["fresh-verification.json"].as_str().unwrap());
    let proof: serde_json::Value = serde_json::from_slice(&proof_bytes).unwrap();
    let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits =
        ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)]).unwrap();
    for row in proof["checks"].as_array().unwrap() {
        let name = format!("{}.xcf", row["id"].as_str().unwrap());
        assert!(name.len() <= 4096 && !name.contains(['/', '\\']));
        let path = root.join(&name);
        let length = std::fs::metadata(&path).unwrap().len();
        assert!(length > 0 && length <= 16 << 20);
        let capture = quota.reserve(QuotaKind::Resident, length.checked_mul(2).unwrap() + 4096).unwrap();
        let bytes = Arc::new(std::fs::read(path).unwrap());
        assert!(bytes.capacity() as u64 <= 2 * length);
        assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
        let raw = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
        // Choose the independently authored RGB requested-depth nonlinear state whose exact
        // source class is admitted by the existing qualified importer.
        if raw.model != Model::Rgb
            || raw.sample != sample
            || raw.encoding != Encoding::NonlinearProfile
            || raw.precision
                != match sample {
                    Sample::U16 => 250,
                    Sample::F32 => 650,
                    Sample::F64 => 750,
                    _ => panic!("closed sample"),
                }
            || !raw
                .layers
                .first()
                .and_then(|layer| layer.effects.first())
                .is_some_and(|effect| effect.operation == operation && effect.state.enabled == Some(true))
        {
            continue;
        }
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            &name,
            &mut std::io::Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &DecodeOptions::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        assert!(imported.document().quota().shares_ledger(&quota));
        assert_eq!(limits.get(BudgetKind::Scratch), 0);
        return Evidence { imported, bytes, quota, name, _capture: capture };
    }
    panic!("pinned packet has no supported RGB requested-depth nonlinear Histogram Threshold state")
}

fn session(e: &Evidence) -> Session {
    let mut s = Session::new();
    match &e.imported {
        photocraft_io::OwnedXcfImport::ThresholdAlpha(v) => {
            s.add_owned_threshold_alpha_xcf_import(v, &e.name, Some("authorized/original.xcf"), &ctl()).unwrap();
        }
        photocraft_io::OwnedXcfImport::HistogramThreshold(v) => {
            s.add_owned_histogram_threshold_xcf_import(v, &e.name, Some("authorized/original.xcf"), &ctl()).unwrap();
        }
        _ => panic!("not a supported source class"),
    }
    s
}
fn request(s: &Session) -> Value {
    json!({"filterId":s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}
#[test]
#[ignore = "UNRUN: requires pinned Alpha and Histogram originals"]
fn exact_resident_response_parity_and_carrier_retains_actual_charge() {
    for histogram in [false, true] {
        let e = evidence(histogram);
        let s = session(&e);
        let p = request(&s);
        let before = e.quota.usage().unwrap();
        let reply = prepare(&s, &p, &ctl()).unwrap();
        assert!(reply.quota().shares_ledger(&e.quota));
        let during = e.quota.usage().unwrap();
        assert_eq!(during.index - before.index, reply.charged_index_bytes());
        assert_eq!(during.resident, before.resident);
        let imported = photocraft_io::import(&e.name, &e.bytes).unwrap();
        let mut resident = Session::new();
        resident.add_document(imported.document, None).unwrap();
        let resident_id = resident.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let mut expected = resident.execute(COMMAND, json!({"filterId":resident_id})).unwrap();
        // Independent imports may allocate different canonical current IDs.
        // Verify actual captured identities, then compare all response fields
        // after substituting just those namespace-dependent identities.
        let state = s.active().unwrap().doc.xcf_state.as_ref().unwrap();
        let stack = &state.filter_stacks[0];
        let node = &stack.nodes[0];
        assert_eq!(reply.value()["filterId"], node.id.0.to_string());
        assert_eq!(reply.value()["owner"]["id"], stack.owner.raw().to_string());
        assert_eq!(reply.value()["mask"]["id"], node.mask.unwrap().raw().to_string());
        for field in ["filterId", "owner", "mask"] {
            expected[field] = reply.value()[field].clone();
        }
        assert_eq!(reply.value(), &expected);
        assert_eq!(reply.value()["argumentsTruncated"], false);
        assert_eq!(reply.value()["argumentCount"], if histogram { 3 } else { 1 });
        assert_eq!(reply.value()["admission"], "staged checked renderer required; metadata inspection does not qualify activation");
        let id = reply.value()["filterId"].as_str().unwrap().to_owned();
        drop(s);
        assert_eq!(reply.value()["filterId"], id);
        assert!(e.quota.usage().unwrap().index >= reply.charged_index_bytes());
        drop(reply);
        assert!(e.imported.document().quota().shares_ledger(&e.quota));
    }
}
#[test]
#[ignore = "UNRUN: requires pinned original sources"]
fn borrowed_query_preserves_redo_source_and_journal_and_uses_same_control() {
    for histogram in [false, true] {
        let e = evidence(histogram);
        let mut s = session(&e);
        let p = request(&s);
        let name = if histogram { "low" } else { "value" };
        let old = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|v| v.name == name).unwrap();
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(bits)) = old.value else { panic!() };
        let next = if bits == 0.25f32.to_bits() { 0.75f32.to_bits() } else { 0.25f32.to_bits() };
        s.execute("xcf.filter.set", json!({"filterId":p["filterId"],"arguments":[{"name":name,"value":{"Number":{"Float32Bits":next}}}]})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().history.can_redo());
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        let view = s.active().unwrap().view_revision;
        let source = s.active().unwrap().source.clone();
        let journal = s.journal.clone();
        let count = AtomicU64::new(0);
        let debit = |n| {
            count.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
        let reply = prepare(&s, &p, &c).unwrap();
        let first = count.load(Ordering::Relaxed);
        let second = prepare(&s, &p, &c).unwrap();
        assert!(count.load(Ordering::Relaxed) > first);
        assert_eq!(reply.value(), second.value());
        let state = s.active().unwrap();
        assert!(state.doc.shares_state(&before));
        assert!(state.history.shares_accounted_history(&history));
        assert_eq!(state.revision, rev);
        assert_eq!(state.view_revision, view);
        assert_eq!(state.source, source);
        assert_eq!(s.journal, journal);
        assert_eq!(state.doc.icc_profile, before.icc_profile);
        assert_eq!(state.doc.xcf_state.as_ref().unwrap().filter_masks, before.xcf_state.as_ref().unwrap().filter_masks);
    }
}
#[test]
#[ignore = "UNRUN: requires pinned original sources"]
fn malformed_ids_cancel_work_quota_and_late_failure_refund_private_reply() {
    let e = evidence(true);
    let s = session(&e);
    let p = request(&s);
    let before = e.quota.usage().unwrap();
    for bad in [
        Value::Null,
        json!({}),
        json!({"filterId":"0"}),
        json!({"filterId":"01"}),
        json!({"filterId":"18446744073709551616"}),
        json!({"filterId":"\u{ff11}"}),
        json!({"filterId":"1","extra":true}),
        json!({"filterId":"18446744073709551615"}),
    ] {
        assert!(prepare(&s, &bad, &ctl()).is_err());
        assert_eq!(e.quota.usage().unwrap(), before);
    }
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(prepare(&s, &p, &c).is_err());
    assert_eq!(e.quota.usage().unwrap(), before);
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(prepare(&s, &p, &c).is_err());
    assert_eq!(e.quota.usage().unwrap(), before);
    let blocker = e.quota.reserve(QuotaKind::Index, (32 << 20) - before.index).unwrap();
    assert!(prepare(&s, &p, &ctl()).is_err());
    drop(blocker);
    assert_eq!(e.quota.usage().unwrap(), before);
    let calls = AtomicU64::new(0);
    let prepaid = AtomicU64::new(0);
    let debit = |n| {
        if n > 100000 {
            prepaid.store(1, Ordering::Relaxed);
        } else if prepaid.load(Ordering::Relaxed) != 0 && calls.fetch_add(1, Ordering::Relaxed) > 3 {
            return Err(BackingError::Cancelled);
        }
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    assert!(prepare(&s, &p, &c).is_err());
    assert_eq!(e.quota.usage().unwrap(), before);
}
#[test]
fn unsupported_empty_and_unowned_sessions_do_not_gain_a_runtime_class() {
    assert!(prepare(&Session::new(), &json!({"filterId":"1"}), &ctl()).is_err());
    let mut s = Session::new();
    s.add_document(Document::new("resident", Size::new(8, 6), ColorMode::Rgb, SampleType::F32), None).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(prepare(&s, &json!({"filterId":"1"}), &ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
