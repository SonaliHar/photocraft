//! UNRUN journal compatibility regressions. A synthetic paid journal row tests
//! metadata accounting only; it supplies no filter/source execution authority.
use super::*;
use photocraft_raster::Interrupt;
use serde_json::json;
use std::sync::Arc;

#[path = "../../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn wire() -> Value {
    json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float32Bits":0.5f32.to_bits()}}}]})
}

#[test]
fn exact_filter_wire_depth_is_scoped_per_entry_in_both_retained_and_copy_censuses() {
    for retained in [false, true] {
        let visit = |c: &mut Census, id, value: &Value| {
            if retained { c.retained_command_value(id, value, &ctl()) } else { c.command_value(id, value, &ctl()) }
        };
        let mut census = Census::default();
        visit(&mut census, "xcf.filter.set", &wire()).unwrap();
        // An unrelated entry cannot inherit the preceding scalar exception.
        assert!(visit(&mut census, "layer.setProps", &wire()).is_err());
        // The ThresholdAlpha-specific parser stays narrow; the later Point
        // metadata adapter admits bounded names before actual target eligibility.
        let point_wire = json!({"filterId":"1","arguments":[{"name":"other","value":{"Number":{"Float32Bits":0}}}]});
        assert_eq!(threshold_alpha::journal_depth("xcf.filter.set", &point_wire), 4);
        visit(&mut Census::default(), "xcf.filter.set", &point_wire).unwrap();
        for value in [
            json!({"filterId":"1","arguments":[{"name":"","value":{"Number":{"Float32Bits":0}}}]}),
            json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float32Bits":f32::NAN.to_bits()}}}]}),
            json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float64Bits":0}}}]}),
        ] {
            assert!(visit(&mut Census::default(), "xcf.filter.set", &value).is_err());
        }
        let mut cumulative = Census::default();
        visit(&mut cumulative, "xcf.filter.set", &wire()).unwrap();
        let before = cumulative.nodes;
        visit(&mut cumulative, "file.saveAs", &json!({"path":"current.pcraft"})).unwrap();
        assert!(cumulative.nodes > before);
        assert!(visit(&mut cumulative, "file.saveAs", &wire()).is_err());
    }
}

struct Fixture {
    session: Session,
    quota: BackingQuota,
    _journal_fee: QuotaLease,
}
fn fixture() -> Fixture {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "original.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let quota = imported.document().quota().clone();
    let fee = quota.reserve(QuotaKind::Index, 64 << 10).unwrap();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "original.xcf", Some("authorized/original.xcf"), &ctl()).unwrap();
    session.journal.push(("xcf.filter.set".into(), wire()));
    Fixture { session, quota, _journal_fee: fee }
}

#[test]
fn native_save_witness_and_reply_preserve_a_prior_exact_filter_wire() {
    let mut fixture = fixture();
    let before = fixture.quota.usage().unwrap();
    let witness = save_journal::PreparedSaveJournalWitness::capture(&fixture.session, &fixture.quota, &ctl()).unwrap();
    witness.validate(&fixture.session, &ctl()).unwrap();
    let params = json!({"path":"current.pcraft"});
    let reply = PreparedAccountedSaveReply::prepare(&fixture.session, "current.pcraft", Some(("file.saveAs", &params)), &ctl()).unwrap();
    drop(reply);
    drop(witness);
    assert_eq!(fixture.quota.usage().unwrap(), before);
    assert_eq!(fixture.session.journal[0].1, wire());
    // The same deep value under an unrelated record keeps the original refusal.
    fixture.session.journal[0].0 = "layer.setProps".into();
    assert!(save_journal::PreparedSaveJournalWitness::capture(&fixture.session, &fixture.quota, &ctl()).is_err());
    assert!(PreparedAccountedSaveReply::prepare(&fixture.session, "current.pcraft", None, &ctl()).is_err());
}

#[test]
fn registered_dispatch_keeps_nonfilter_routes_and_refuses_filter_background_before_normalization() {
    let mut fixture = fixture();
    let layer = fixture.session.active().unwrap().active_layer.unwrap();
    // Family-specific filter preflights must not reject unrelated registered ids.
    fixture.session.execute("layer.select", json!({"layer":layer.0})).unwrap();
    assert_eq!(fixture.session.journal.last().unwrap().0, "layer.select");
    let before = fixture.session.active().unwrap().doc.clone();
    let journal = fixture.session.journal.clone();
    let error = fixture.session.start("xcf.filter.enable", json!({"filterId":"1","enabled":false})).unwrap_err();
    assert!(error.to_string().contains("synchronous"));
    assert!(fixture.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(fixture.session.journal, journal);
    assert!(!fixture.session.has_jobs());
}

#[test]
fn shared_borrowed_filter_identity_parser_preserves_canonical_u64_boundaries() {
    for text in ["1", "10", "18446744073709551615"] {
        assert_eq!(crate::xcf_filter_cmds::id("xcf.filter.set", &json!({"filterId":text})).unwrap().0, text.parse::<u64>().unwrap());
    }
    for text in ["", "0", "01", "0001", "+1", "-1", "١", "18446744073709551616", "100000000000000000000"] {
        assert!(crate::xcf_filter_cmds::id("xcf.filter.set", &json!({"filterId":text})).is_err());
    }
}
