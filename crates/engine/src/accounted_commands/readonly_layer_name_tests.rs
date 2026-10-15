//! UNRUN readonly-name wire and genuine source workflow drafts.
use super::*;
use photocraft_raster::Interrupt;
use serde_json::json;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
fn exact_names_targets_and_closed_fields() {
    for name in ["", "  exact  ", "\u{540d}"] {
        for target in [Value::Null, json!(0), json!(u64::MAX), json!("current")] {
            let value = json!({"layer":target,"name":name});
            assert_eq!(Request::parse(&value, &ctl()).unwrap().name, name);
        }
    }
    for value in [
        json!({}),
        json!({"name":null}),
        json!({"name":"a\0b"}),
        json!({"name":"ok","visible":true}),
        json!({"name":"ok","coalesce":null}),
        json!({"name":"ok","layer":-1}),
        json!({"name":"ok","layer":1.5}),
    ] {
        assert!(Request::parse(&value, &ctl()).is_err());
    }
    let value = json!({"name":"ok"});
    assert!(Request::parse(&value, &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }).is_err());
}
#[cfg(feature = "corpus")]
mod actual_source {
    use super::*;
    use photocraft_raster::backing::quota::{BackingQuota, QuotaKind, QuotaLimits};
    use sha2::{Digest, Sha256};
    use std::{path::PathBuf, sync::Arc};
    fn hash(v: &[u8]) -> String {
        Sha256::digest(v).iter().map(|b| format!("{b:02x}")).collect()
    }
    fn session(alpha: bool) -> Session {
        let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("requires pinned Histogram precision evidence")).join(if alpha {
            "alpha-threshold-precision-evidence"
        } else {
            "threshold-precision-evidence"
        });
        let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
        assert_eq!(
            hash(&manifest),
            if alpha {
                "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263"
            } else {
                "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142"
            }
        );
        let manifest: Value = serde_json::from_slice(&manifest).unwrap();
        let proof: Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let name = format!("{}.xcf", row["id"].as_str().unwrap());
            assert!(!name.contains(['/', '\\']));
            let path = root.join(&name);
            let length = std::fs::metadata(&path).unwrap().len();
            assert!(length <= 16 << 20);
            let capture = q.reserve(QuotaKind::Resident, 2 * length + 4096).unwrap();
            let bytes = Arc::new(std::fs::read(path).unwrap());
            assert!(bytes.capacity() as u64 <= 2 * length);
            assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            if raw.model != photocraft_xcf::Model::Rgb
                || raw.sample != photocraft_xcf::Sample::F64
                || raw.encoding != photocraft_xcf::Encoding::NonlinearProfile
                || !raw
                    .layers
                    .first()
                    .and_then(|l| l.effects.first())
                    .is_some_and(|f| f.operation == if alpha { "gimp:threshold-alpha" } else { "gimp:threshold" } && f.state.enabled == Some(true))
            {
                continue;
            }
            let owned = photocraft_io::import_owned_xcf_bytes_in_quota(&name, bytes, &Default::default(), &limits, q.clone(), &ctl()).unwrap();
            let mut s = Session::new();
            match owned {
                photocraft_io::OwnedXcfImport::ThresholdAlpha(ref imported) if alpha => {
                    s.add_owned_threshold_alpha_xcf_import(imported, &name, Some("owned/original.xcf"), &ctl()).unwrap();
                }
                photocraft_io::OwnedXcfImport::HistogramThreshold(ref imported) if !alpha => {
                    s.add_owned_histogram_threshold_xcf_import(imported, &name, Some("owned/original.xcf"), &ctl()).unwrap();
                }
                _ => panic!("wrong selected family"),
            }
            drop(owned);
            drop(capture);
            return s;
        }
        panic!("missing pinned qualified enabled RGB/F64 source")
    }

    fn retained(a: &DocumentSnapshot, b: &DocumentSnapshot) {
        assert_eq!(a.layers[0].content, b.layers[0].content);
        assert_eq!(a.xcf_state.as_ref().unwrap().filter_stacks, b.xcf_state.as_ref().unwrap().filter_stacks);
        assert_eq!(a.xcf_state.as_ref().unwrap().filter_masks, b.xcf_state.as_ref().unwrap().filter_masks);
        assert_eq!(a.source_color_context, b.source_color_context);
        assert_eq!(a.icc_profile, b.icc_profile);
        assert!(a.accounted().unwrap().quota().shares_ledger(b.accounted().unwrap().quota()));
    }
    #[test]
    #[ignore = "UNRUN: requires SHA-pinned Alpha/Histogram GIMP evidence"]
    fn registered_both_families_one_rename_noop_redo_and_exact_native_owners() {
        for alpha in [true, false] {
            let mut s = session(alpha);
            let before = s.active().unwrap().doc.clone();
            let id = before.layers[0].id;
            let original = before.layers[0].name.clone();
            let renamed = "  current \u{540d}  ";
            assert_eq!(s.execute("layer.setProps", json!({"layer":original,"name":renamed})).unwrap(), Value::Null);
            let changed = s.active().unwrap().doc.clone();
            retained(&before, &changed);
            assert_eq!(changed.layers[0].name, renamed);
            let item = changed.xcf_state.as_ref().unwrap().items.iter().find(|v| v.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap();
            assert_eq!(item.name, renamed);
            assert!(item.custom_name);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            assert!(s.undo_with_control(&ctl()).unwrap());
            let history = s.active().unwrap().history.clone();
            let revision = s.active().unwrap().revision;
            let name = s.active().unwrap().doc.layers[0].name.clone();
            s.execute("layer.setProps", json!({"name":name})).unwrap();
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(history.shares_accounted_history(&s.active().unwrap().history));
            assert!(s.redo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&changed));
            assert!(s.active().unwrap().source.as_ref().unwrap().check_destination("owned/original.xcf").is_err());
        }
    }
    #[test]
    #[ignore = "UNRUN: requires SHA-pinned Alpha/Histogram GIMP evidence"]
    fn prepared_stale_view_source_history_context_and_cancel_never_publish() {
        for alpha in [true, false] {
            let mut s = session(alpha);
            let value = json!({"name":"private candidate"});
            let before = s.active().unwrap().doc.clone();
            for kind in 0..4 {
                let prepared = Request::parse(&value, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
                match kind {
                    0 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
                    1 => s.active_mut().unwrap().source_read_only = false,
                    2 => s.active_mut().unwrap().history.max_states += 1,
                    _ => s.color_restrict = Some(0),
                }
                assert!(prepared.execute(&mut s, &ctl()).is_err());
                assert!(s.active().unwrap().doc.shares_state(&before));
                match kind {
                    0 => s.active_mut().unwrap().channel_view.quick_mask_hidden = false,
                    1 => s.active_mut().unwrap().source_read_only = true,
                    2 => s.active_mut().unwrap().history.max_states -= 1,
                    _ => s.color_restrict = None,
                }
            }
            let prepared = Request::parse(&value, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
            let stop = || true;
            let interrupt = Interrupt::cancel_only(&stop);
            assert!(prepared.execute(&mut s, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            retained(&before, &s.active().unwrap().doc);
        }
    }
}
