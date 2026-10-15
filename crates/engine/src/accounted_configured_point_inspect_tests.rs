//! UNRUN metadata-only configured Point queries. These deliberately unqualified
//! CURRENT models are installed with engine-private test primitives, never the
//! public generic opener. No source or renderer qualification is asserted.
use super::*;
use photocraft_doc::{
    ColorMode, SampleEncoding, SampleType,
    accounted_document::AccountedDocument,
    xcf_filters::{ExactNumber, FilterArgument},
};
use photocraft_raster::{Interrupt, backing::quota::QuotaKind};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn ctl() -> ReadControl<'static> {
    super::point_tests::ctl()
}
fn number(name: &str, value: f64) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(value.to_bits())) }
}
fn number_wire(name: &str, value: f64) -> Value {
    json!({"name":name,"value":{"Number":{"Float64Bits":format!("0x{:016x}",value.to_bits())}}})
}
fn configured(kind: usize) -> (&'static str, Vec<FilterArgument>, Value) {
    let channels = ["value", "red", "green", "blue", "alpha"];
    match kind {
        0 => {
            let axes = ["cyan-red", "magenta-green", "yellow-blue"];
            let mut fields = Vec::new();
            let mut expected = Vec::new();
            for (i, name) in ["shadows", "midtones", "highlights"].into_iter().enumerate() {
                let values = [-0.0, 0.125 * (i + 1) as f64, -0.25];
                fields.push(FilterArgument {
                    name: name.into(),
                    value: FilterValue::Config(axes.into_iter().zip(values).map(|(name, v)| number(name, v)).collect()),
                });
                expected.push(json!({"name":name,"value":{"Config":axes.into_iter().zip(values).map(|(name,v)|number_wire(name,v)).collect::<Vec<_>>()}}));
            }
            (
                "gimp:color-balance",
                vec![
                    FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) },
                    FilterArgument { name: "config".into(), value: FilterValue::Config(fields) },
                ],
                json!({"Config":expected}),
            )
        }
        1 => {
            let names = ["low-input", "high-input", "gamma", "low-output", "high-output"];
            let values = [-0.0, 1.0, 1.25, -0.0, 1.0];
            let mut fields = Vec::new();
            let mut expected = Vec::new();
            for name in channels {
                fields.push(FilterArgument {
                    name: name.into(),
                    value: FilterValue::Config(names.into_iter().zip(values).map(|(name, v)| number(name, v)).collect()),
                });
                expected.push(json!({"name":name,"value":{"Config":names.into_iter().zip(values).map(|(name,v)|number_wire(name,v)).collect::<Vec<_>>()}}));
            }
            (
                "gimp:levels",
                vec![
                    FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:levels::trc".into(), value: 1 } },
                    FilterArgument { name: "clamp-input".into(), value: FilterValue::Bool(false) },
                    FilterArgument { name: "clamp-output".into(), value: FilterValue::Bool(true) },
                    FilterArgument { name: "config".into(), value: FilterValue::Config(fields) },
                ],
                json!({"Config":expected}),
            )
        }
        _ => {
            let mut fields = Vec::new();
            let mut expected = Vec::new();
            for name in channels {
                let words: Vec<_> = (0..256).map(|i| if i == 0 { (-0.0f64).to_bits() } else { (f64::from(i) / 255.0).to_bits() }).collect();
                expected.push(json!({"name":name,"value":{"Curve":{"free":true,"points_f64_bits":[],"point_types":[],"samples_f64_bits":words.iter().map(|word|format!("0x{word:016x}")).collect::<Vec<_>>()}}}));
                fields.push(FilterArgument {
                    name: name.into(),
                    value: FilterValue::Curve { free: true, points_f64_bits: Vec::new(), point_types: Vec::new(), samples_f64_bits: words },
                });
            }
            (
                "gimp:curves",
                vec![
                    FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:curves::trc".into(), value: 1 } },
                    FilterArgument { name: "config".into(), value: FilterValue::Config(fields) },
                ],
                json!({"Config":expected}),
            )
        }
    }
}
fn session(kind: usize) -> (Session, BackingQuota, Arc<AtomicU64>, Value) {
    let (mut doc, quota, guard, reads, _, _) = super::point_tests::fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    // Prospectively cover fixture-only configured DTOs, expected wire and
    // metadata clones used by these synthetic tests, independent of reply fees.
    let extra = quota.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let (operation, arguments, expected) = configured(kind);
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = operation.into();
    node.operation.schema_epoch = "metadata-only-not-renderer-qualified".into();
    node.composition.as_mut().unwrap().compatibility_epoch = "metadata-only-not-renderer-qualified".into();
    node.arguments = arguments;
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, extra)), quota.clone(), &ctl()).unwrap();
    let mut s = Session::new();
    assert!(s.add_accounted_document(owner.clone(), None, &ctl()).is_err());
    let state = crate::DocState::new_accounted(owner, None, 16, &ctl()).unwrap();
    s.publish_document_state(state, &ctl()).unwrap();
    (s, quota, reads, expected)
}
fn params(s: &Session) -> Value {
    json!({"filterId":s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()})
}

#[test]
fn complete_cb_levels_and_free1280_hex_queries_are_paid_immutable_metadata_without_pixel_reads() {
    for kind in 0..3 {
        let (s, quota, reads, expected) = session(kind);
        let before = s.active().unwrap().doc.clone();
        let baseline = quota.usage().unwrap();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.len();
        let reply = prepare(&s, &params(&s), &ctl()).unwrap();
        assert!(reply.quota().shares_ledger(&quota));
        let row = reply.value()["arguments"].as_array().unwrap().iter().find(|row| row["name"] == "config").unwrap();
        assert_eq!(row["state"]["value"], expected);
        assert_eq!(row["state"]["structuredBits"], "canonical hexadecimal64");
        assert!(row["state"].get("unresolvedStructuredValue").is_none());
        if kind == 2 {
            let curves = row["state"]["value"]["Config"].as_array().unwrap();
            assert_eq!(curves.iter().map(|curve| curve["value"]["Curve"]["samples_f64_bits"].as_array().unwrap().len()).sum::<usize>(), 1280);
            assert_eq!(curves[0]["value"]["Curve"]["samples_f64_bits"][0], "0x8000000000000000");
        } else {
            assert_eq!(row["state"]["value"]["Config"][0]["value"]["Config"][0]["value"]["Number"]["Float64Bits"], "0x8000000000000000");
        }
        assert!(quota.usage().unwrap().index > baseline.index);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal.len(), journal);
        drop(reply);
        assert_eq!(quota.usage().unwrap(), baseline);
    }
}

#[test]
fn structured_shape_caps_refuse_before_serialization_without_widening_flat_or_threshold_bounds() {
    let (s, quota, _, _) = session(2);
    let _clone_fee = quota.reserve(QuotaKind::Index, 256 << 10).unwrap();
    let original = &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert!(point_metadata(original, &ctl()).is_err());
    assert!(metadata(original, &ctl()).is_err());
    assert_eq!((FILTER_BOUNDS.nodes, FILTER_BOUNDS.depth, POINT_FILTER_BOUNDS.nodes, POINT_FILTER_BOUNDS.depth), (256, 8, 512, 8));
    for variant in 0..5 {
        let mut node = original.clone();
        let FilterValue::Config(channels) = &mut node.arguments[1].value else { panic!("config fixture") };
        match variant {
            0 => channels.push(channels[0].clone()),
            1 => {
                let FilterValue::Curve { samples_f64_bits, .. } = &mut channels[0].value else { panic!("curve fixture") };
                samples_f64_bits.push(0);
            }
            2 => channels[0].name = String::new(),
            3 => {
                channels[0].value =
                    FilterValue::Config(vec![FilterArgument { name: "extra".into(), value: FilterValue::Config(vec![number("too-deep", 0.0)]) }])
            }
            _ => channels[0].value = FilterValue::String("unsupported".into()),
        }
        assert!(structured_point_plan(&node, &ctl()).is_err());
    }
}

#[test]
fn configured_query_work_cancel_and_real_quota_denial_keep_payload_fees_and_snapshot() {
    let (s, quota, reads, _) = session(2);
    let before = s.active().unwrap().doc.clone();
    let baseline = quota.usage().unwrap();
    let p = params(&s);
    let deny = |_| Err(BackingError::Work);
    assert!(prepare(&s, &p, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert_eq!(quota.usage().unwrap(), baseline);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(prepare(&s, &p, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    assert_eq!(quota.usage().unwrap(), baseline);
    let blocker = quota.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(prepare(&s, &p, &ctl()).is_err());
    drop(blocker);
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}
