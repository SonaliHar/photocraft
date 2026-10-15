//! UNRUN exact Histogram Threshold wire/transaction/failure drafts.
use super::*;
use photocraft_raster::Interrupt;
use serde_json::json;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn low(id: &str, bits: u32) -> Value {
    json!({"filterId":id,"arguments":[{"name":"low","value":{"Number":{"Float32Bits":bits}}}]})
}
fn channel(value: i32) -> Value {
    json!({"name":"channel","value":{"Enum":{"domain":"GimpHistogramChannel","value":value}}})
}
#[test]
fn exact_three_patches_preserve_crossed_limits_signed_zero_enum_and_closed_depth() {
    for v in 0..=5 {
        let p = json!({"filterId":"18446744073709551615","enabled":false,"arguments":[{"name":"high","value":{"Number":{"Float32Bits":0.125f32.to_bits()}}},channel(v),{"name":"low","value":{"Number":{"Float32Bits":0.875f32.to_bits()}}}]});
        let f = wire("xcf.filter.set", &p).unwrap();
        assert_eq!(f.high, Some(0.125f32.to_bits()));
        assert_eq!(f.low, Some(0.875f32.to_bits()));
        assert_eq!(f.channel, Some(v));
        assert_eq!(f.enabled, Some(false));
        assert_eq!(journal_depth("xcf.filter.set", &p), 5);
        assert_eq!(journal_depth("layer.setProps", &p), 4);
    }
    assert_eq!(wire("xcf.filter.set", &low("1", (-0.0f32).to_bits())).unwrap().low, Some((-0.0f32).to_bits()));
    assert_eq!(journal_depth("xcf.filter.enable", &json!({"filterId":"1","enabled":false})), 4);
    assert!(wire("xcf.filter.set", &json!({"filterId":"1","opacityF64Bits":"0x3ff0000000000000"})).is_some());
}
#[test]
fn duplicate_wrong_domain_nonfinite_storage_and_unrelated_fields_cannot_gain_paid_depth() {
    for value in [
        json!({"filterId":"1","arguments":[channel(0),channel(1)]}),
        json!({"filterId":"1","arguments":[channel(-1)]}),
        json!({"filterId":"1","arguments":[channel(6)]}),
        json!({"filterId":"1","arguments":[{"name":"channel","value":{"Enum":{"domain":"other","value":0}}}]}),
        json!({"filterId":"1","arguments":[{"name":"channel","value":{"Enum":{"domain":"GimpHistogramChannel","value":0,"other":false}}}]}),
        json!({"filterId":"1","arguments":[{"name":"low","value":{"Number":{"Float64Bits":"0x3fe0000000000000"}}}]}),
        json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float32Bits":0}}}]}),
        json!({"filterId":"1","arguments":[channel(0),channel(0),channel(0),channel(0)]}),
        json!({"filterId":"1","enabled":true,"coalesce":null}),
        json!({"filterId":"1","opacityF64Bits":"0x3fe0000000000000"}),
    ] {
        assert!(wire("xcf.filter.set", &value).is_none());
        assert_eq!(journal_depth("xcf.filter.set", &value), 4);
    }
    for id in ["0", "01", "+1", "18446744073709551616", ""] {
        assert!(wire("xcf.filter.set", &low(id, 0)).is_none());
    }
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.01f32).to_bits(), 1.01f32.to_bits()] {
        assert!(wire("xcf.filter.set", &low("1", bits)).is_none());
    }
    assert!(wire("xcf.filter.enable", &json!({"filterId":"1","enabled":false,"arguments":[]})).is_none());
    let stopped = Interrupt::cancel_only(&|| true);
    assert!(matches!(
        Request::parse(&Session::new(), "xcf.filter.set", &low("1", 0), &ReadControl { interrupt: &stopped, work: &|_| Ok(()) }),
        Err(EngineError::Cancelled)
    ));
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
    fn session() -> Session {
        let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("requires pinned Histogram precision evidence"))
            .join("threshold-precision-evidence");
        let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
        assert_eq!(hash(&manifest), "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142");
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
                || !raw.layers.first().and_then(|l| l.effects.first()).is_some_and(|f| f.operation == "gimp:threshold" && f.state.enabled == Some(true))
            {
                continue;
            }
            let owned = photocraft_io::import_owned_xcf_bytes_in_quota(&name, bytes, &Default::default(), &limits, q.clone(), &ctl()).unwrap();
            let photocraft_io::OwnedXcfImport::HistogramThreshold(imported) = owned else { panic!("wrong original family") };
            let mut s = Session::new();
            s.add_owned_histogram_threshold_xcf_import(&imported, &name, Some("owned/original.xcf"), &ctl()).unwrap();
            drop(imported);
            drop(capture);
            return s;
        }
        panic!("missing pinned qualified enabled RGB/F64 source")
    }
    fn id(s: &Session) -> String {
        s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()
    }
    fn bits(s: &Session, name: &str) -> u32 {
        let node = &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(v)) =
            node.arguments.iter().find(|a| a.name == name).unwrap().value
        else {
            panic!()
        };
        v
    }
    #[test]
    #[ignore = "UNRUN: requires independently authored pinned Histogram Threshold source"]
    fn registered_patch_is_one_history_entry_and_exact_noop_preserves_redo_and_source() {
        let mut s = session();
        let filter = id(&s);
        let before = s.active().unwrap().doc.clone();
        let original = bits(&s, "low");
        let next = if original == 0.875f32.to_bits() { 0.25f32.to_bits() } else { 0.875f32.to_bits() };
        let patch = json!({"filterId":filter,"arguments":[{"name":"low","value":{"Number":{"Float32Bits":next}}},channel(5)]});
        let out = s.execute("xcf.filter.set", patch).unwrap();
        assert_eq!(out, json!({"filterId":filter,"updated":true}));
        let changed = s.active().unwrap().doc.clone();
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(changed.source_color_context, before.source_color_context);
        assert_eq!(changed.icc_profile, before.icc_profile);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&before));
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let noop = s.execute("xcf.filter.set", low(&filter, original)).unwrap();
        assert_eq!(noop["updated"], false);
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":filter,"enabled":false})).unwrap()["updated"], true);
        assert!(!s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert!(s.active().unwrap().source.as_ref().unwrap().check_destination("owned/original.xcf").is_err());
    }
    #[test]
    #[ignore = "UNRUN: requires independently authored pinned Histogram Threshold source"]
    fn prepared_candidate_stale_source_history_and_cancel_never_publish() {
        let mut s = session();
        let filter = id(&s);
        let p = low(&filter, 0.5f32.to_bits());
        let before = s.active().unwrap().doc.clone();
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        let max = s.active().unwrap().history.max_states;
        s.active_mut().unwrap().history.max_states = max + 1;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().history.max_states = max;
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        s.active_mut().unwrap().source_read_only = false;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().source_read_only = true;
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        let stop = Interrupt::cancel_only(&|| true);
        assert!(matches!(prepared.execute(&mut s, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        let work = |_| Err(BackingError::Work);
        assert!(Request::parse(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
}
