//! UNRUN readonly current-property wire and genuine source workflow drafts.
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
            assert_eq!(Request::parse(&value, &ctl()).unwrap().name, Some(name));
        }
    }
    for value in [
        json!({}),
        json!({"name":null}),
        json!({"name":"a\0b"}),
        json!({"name":"ok","fill":0.5}),
        json!({"name":"ok","coalesce":null}),
        json!({"name":"ok","layer":-1}),
        json!({"name":"ok","layer":1.5}),
    ] {
        assert!(Request::parse(&value, &ctl()).is_err());
    }
    let value = json!({"name":"ok"});
    assert!(Request::parse(&value, &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }).is_err());
}
#[test]
fn new_shapes_ignore_wrong_types_only_with_a_typed_field_and_cast_before_clamp() {
    for (input, expected) in [(f64::MAX, 1f32.to_bits()), (-f64::MAX, 0f32.to_bits()), (-0.0, (-0.0f32).to_bits()), (0.3, (0.3f64 as f32).to_bits())] {
        let value = json!({"name":null,"visible":"ignored","opacity":input});
        let request = Request::parse(&value, &ctl()).unwrap();
        assert_eq!(request.name, None);
        assert_eq!(request.visible, None);
        assert_eq!(request.opacity, Some(expected));
        assert!(request.extended);
        assert!(!request.only_visibility);
    }
    for value in [
        json!({"visible":null}),
        json!({"name":9,"visible":"ignored","opacity":null}),
        json!({"visible":false,"fill":1.0}),
        json!({"opacity":0.5,"coalesce":null}),
    ] {
        assert!(Request::parse(&value, &ctl()).is_err());
    }
    let value = json!({"name":"exact","visible":null});
    let request = Request::parse(&value, &ctl()).unwrap();
    assert_eq!(request.name, Some("exact"));
    assert!(request.extended);
    let value = json!({"visible":false});
    assert!(Request::parse(&value, &ctl()).unwrap().only_visibility);
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
            let value = json!({"name":"private candidate","opacity":0.375});
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
    #[test]
    #[ignore = "UNRUN: requires SHA-pinned Alpha/Histogram GIMP evidence"]
    fn registered_compound_properties_are_one_history_step_with_exact_noop_redo() {
        for alpha in [true, false] {
            let mut s = session(alpha);
            let before = s.active().unwrap().doc.clone();
            let id = before.layers[0].id;
            let old_name = before.layers[0].name.clone();
            let old_visible = before.layers[0].visible;
            let old_opacity = before.layers[0].opacity;
            s.execute("layer.setProps", json!({"layer":id.0,"name":"combined current","visible":false,"opacity":0.375})).unwrap();
            let changed = s.active().unwrap().doc.clone();
            retained(&before, &changed);
            assert_eq!(changed.layers[0].name, "combined current");
            assert!(!changed.layers[0].visible);
            assert_eq!(changed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
            let item = changed.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap();
            assert_eq!(item.name, "combined current");
            assert!(!item.visible);
            assert!(item.custom_name);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Layer Properties"));
            assert!(s.undo_with_control(&ctl()).unwrap());
            let history = s.active().unwrap().history.clone();
            let revision = s.active().unwrap().revision;
            s.execute("layer.setProps", json!({"name":old_name,"visible":old_visible,"opacity":f64::from(old_opacity)})).unwrap();
            assert!(history.shares_accounted_history(&s.active().unwrap().history));
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(s.redo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&changed));
            s.execute("layer.setProps", json!({"visible":true})).unwrap();
            assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
            retained(&before, &s.active().unwrap().doc);
        }
    }
    // Current-storage-only lock fixture, NOT evidence that GIMP authored locks.
    // Real additional model/header charge precedes the bounded clone; the exact
    // original native owner/guard remains retained beneath this current model.
    struct CurrentRetention {
        _original: AccountedDocument,
        _model: QuotaLease,
    }
    fn locked_current(alpha: bool) -> Session {
        let source = session(alpha);
        let original = source.active().unwrap().doc.accounted().unwrap().clone();
        let quota = original.quota().clone();
        let fee = quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
        let mut document = original.document().clone();
        let id = document.layers[0].id;
        document.layers[0].locks.all = true;
        document.layers[0].locks.pixels = true;
        let item = document.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap();
        item.locks.content = true;
        item.locks.visibility = true;
        let retention: Arc<dyn std::any::Any + Send + Sync> = Arc::new(CurrentRetention { _original: original, _model: fee });
        let owner = if alpha {
            AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, retention, quota, &ctl()).unwrap()
        } else {
            AccountedDocument::new_readonly_threshold_storage(document, retention, quota, &ctl()).unwrap()
        };
        let mut result = Session::new();
        result.add_accounted_document(owner, None, &ctl()).unwrap();
        result
    }
    #[test]
    #[ignore = "UNRUN: real paid current-storage lock fixture, not source qualification"]
    fn only_actual_changed_visibility_is_locked_compound_decline_is_atomic() {
        for alpha in [true, false] {
            let mut s = locked_current(alpha);
            let before = s.active().unwrap().doc.clone();
            let journal = s.journal.clone();
            let history = s.active().unwrap().history.clone();
            let revision = s.active().unwrap().revision;
            assert!(s.execute("layer.setProps", json!({"name":"must not publish","visible":false,"opacity":0.25})).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert_eq!(s.journal, journal);
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(history.shares_accounted_history(&s.active().unwrap().history));
            // No content/name/opacity or unchanged-visibility lock is invented.
            s.execute("layer.setProps", json!({"name":"locked metadata allowed","visible":true,"opacity":0.25})).unwrap();
            let changed = s.active().unwrap().doc.clone();
            retained(&before, &changed);
            assert_eq!(changed.layers[0].name, "locked metadata allowed");
            assert_eq!(changed.layers[0].opacity.to_bits(), 0.25f32.to_bits());
            assert_eq!(s.active().unwrap().history.past_len(), 1);
        }
    }
}
