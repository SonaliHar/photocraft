//! UNRUN paid project discovery drafts over independently saved, SHA-pinned originals.
//! No fabricated current metadata is used as original-XCF qualification.
use super::*;
use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLease};
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

fn scrub_ids(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "id" && value.is_string() {
                    *value = Value::String("namespace-id".into());
                } else {
                    scrub_ids(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                scrub_ids(value);
            }
        }
        _ => {}
    }
}
#[test]
#[ignore = "UNRUN: requires pinned Alpha and Histogram Threshold originals"]
fn default_null_and_all_supported_pages_match_resident_metadata_schema() {
    for histogram in [false, true] {
        let e = evidence(histogram);
        let s = session(&e);
        let imported = photocraft_io::import(&e.name, &e.bytes).unwrap();
        let mut resident = Session::new();
        resident.add_document(imported.document, None).unwrap();
        for params in [Value::Null, json!({}), json!({"offset":0,"limit":1}), json!({"offset":1,"limit":128}), json!({"offset":100000,"limit":128})] {
            let baseline = e.quota.usage().unwrap();
            let reply = prepare(&s, &params, &ctl()).unwrap();
            assert_eq!(reply.command(), "xcf.inspect");
            assert!(reply.quota().shares_ledger(&e.quota));
            assert_eq!(e.quota.usage().unwrap().index - baseline.index, reply.charged_index_bytes());
            // Independent resident/current namespaces can differ. Pay for this
            // derivative comparison tree, then compare all non-ID fields and
            // assert actual canonical owner/filter IDs separately below.
            let _comparison = e.quota.reserve(QuotaKind::Index, 2 << 20).unwrap();
            let mut actual = reply.value().clone();
            let mut expected = resident.execute(COMMAND, params.clone()).unwrap();
            scrub_ids(&mut actual);
            scrub_ids(&mut expected);
            assert_eq!(actual, expected);
            assert_eq!(reply.value()["filters"]["total"], 1);
            let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
            assert_eq!(reply.value()["filters"]["offset"], offset);
            if offset == 0 {
                let state = s.active().unwrap().doc.xcf_state.as_ref().unwrap();
                assert_eq!(reply.value()["filters"]["entries"][0]["id"], state.filter_stacks[0].nodes[0].id.0.to_string());
                assert_eq!(reply.value()["filterStacks"]["entries"][0]["revision"], state.filter_stacks[0].revision.to_string());
                assert!(reply.value()["filterStacks"]["entries"][0]["revision"].is_string());
                assert_eq!(reply.value()["filters"]["entries"][0]["inspectCommand"], "xcf.filter.inspect");
            } else {
                assert!(reply.value()["filters"]["entries"].as_array().unwrap().is_empty());
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: requires pinned original sources"]
fn discovery_returns_stable_id_then_exact_filter_query_without_mutating_history_or_source() {
    for histogram in [false, true] {
        let e = evidence(histogram);
        let s = session(&e);
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        let view = s.active().unwrap().view_revision;
        let source = s.active().unwrap().source.clone();
        let journal = s.journal.clone();
        let used = AtomicU64::new(0);
        let debit = |n| {
            used.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
        let discovery = prepare(&s, &Value::Null, &c).unwrap();
        let first = used.load(Ordering::Relaxed);
        let id = discovery.value()["filters"]["entries"][0]["id"].as_str().unwrap();
        let detail = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":id}), &c).unwrap();
        assert!(used.load(Ordering::Relaxed) > first);
        assert_eq!(detail.command(), "xcf.filter.inspect");
        assert_eq!(detail.value()["filterId"], id);
        assert!(detail.value()["revision"].is_u64());
        assert_eq!(detail.value()["argumentCount"], if histogram { 3 } else { 1 });
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, rev);
        assert_eq!(s.active().unwrap().view_revision, view);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.journal, journal);
        assert_eq!(s.active().unwrap().doc.icc_profile, before.icc_profile);
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_masks, before.xcf_state.as_ref().unwrap().filter_masks);
        drop(s);
        assert_eq!(discovery.command(), "xcf.inspect");
        assert_eq!(detail.command(), "xcf.filter.inspect");
        let wire = crate::accounted_filter_inspect::encode_cli(discovery, true, 1 << 20, &ctl()).unwrap();
        let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
        assert_eq!(value["command"], "xcf.inspect");
    }
}
#[test]
#[ignore = "UNRUN: requires pinned original sources"]
fn malformed_pagination_work_quota_and_cancellation_preserve_installed_owner_and_refund() {
    let e = evidence(true);
    let s = session(&e);
    let before = s.active().unwrap().doc.clone();
    let baseline = e.quota.usage().unwrap();
    for params in [
        json!({"filterId":"1"}),
        json!([]),
        json!({"offset":-1}),
        json!({"offset":100001}),
        json!({"offset":null}),
        json!({"limit":0}),
        json!({"limit":129}),
        json!({"limit":1.5}),
        json!({"limit":"1"}),
    ] {
        assert!(prepare(&s, &params, &ctl()).is_err());
        assert_eq!(e.quota.usage().unwrap(), baseline);
    }
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(prepare(&s, &Value::Null, &c).is_err());
    assert_eq!(e.quota.usage().unwrap(), baseline);
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(prepare(&s, &Value::Null, &c).is_err());
    assert_eq!(e.quota.usage().unwrap(), baseline);
    let blocker = e.quota.reserve(QuotaKind::Index, (32 << 20) - baseline.index).unwrap();
    assert!(prepare(&s, &Value::Null, &ctl()).is_err());
    drop(blocker);
    assert_eq!(e.quota.usage().unwrap(), baseline);
    let prepaid = AtomicU64::new(0);
    let calls = AtomicU64::new(0);
    let debit = |n| {
        if n > 100000 {
            prepaid.store(1, Ordering::Relaxed);
        } else if prepaid.load(Ordering::Relaxed) != 0 && calls.fetch_add(1, Ordering::Relaxed) > 5 {
            return Err(BackingError::Cancelled);
        }
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    assert!(prepare(&s, &Value::Null, &c).is_err());
    assert_eq!(e.quota.usage().unwrap(), baseline);
    assert!(s.active().unwrap().doc.shares_state(&before));
}
#[test]
fn bounded_utf8_truncation_counts_complete_strings_even_across_chunk_edges() {
    for text in ["a".repeat(256), "a".repeat(257), "lambda \u{03bb}".repeat(20000), format!("{}\u{1f642}{}", "x".repeat(32767), "y".repeat(80000))] {
        let used = AtomicU64::new(0);
        let visits = AtomicU64::new(0);
        let debit = |n| {
            assert!(n <= 32769);
            used.fetch_add(n, Ordering::Relaxed);
            visits.fetch_add(1, Ordering::Relaxed);
            Ok(())
        };
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
        assert_eq!(over_256(&text, &c).unwrap(), text.chars().count() > 256);
        assert_eq!(used.load(Ordering::Relaxed), text.len() as u64 + visits.load(Ordering::Relaxed));
    }
    let visits = AtomicU64::new(0);
    let debit = |_| if visits.fetch_add(1, Ordering::Relaxed) > 1 { Err(BackingError::Cancelled) } else { Ok(()) };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    assert!(over_256(&"x".repeat(100000), &c).is_err());
}
#[test]
fn borrowed_custom_resolution_matches_public_validation_with_bounded_unit_scans() {
    let mut resolution = photocraft_doc::PhysicalResolution {
        x_dpi: 120.,
        y_dpi: 240.,
        unit: photocraft_doc::PhysicalUnit::Custom(photocraft_doc::CustomUnit {
            name: "unit".repeat(20000),
            factor: 2.,
            digits: 3,
            symbol: "\u{03bc}".into(),
            abbreviation: "u".into(),
        }),
    };
    let visits = AtomicU64::new(0);
    let debit = |n| {
        assert!(n <= 32769);
        visits.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    assert_eq!(validate_resolution(&resolution, &c).is_ok(), resolution.validate().is_ok());
    assert!(visits.load(Ordering::Relaxed) > 3);
    if let photocraft_doc::PhysicalUnit::Custom(unit) = &mut resolution.unit {
        unit.name.push('\0');
    }
    assert_eq!(validate_resolution(&resolution, &ctl()).is_ok(), resolution.validate().is_ok());
    resolution.x_dpi = f64::NAN;
    assert!(validate_resolution(&resolution, &ctl()).is_err());
    assert!(prepare(&Session::new(), &Value::Null, &ctl()).is_err());
}
