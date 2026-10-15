//! UNRUN genuine witnessed two-node Point transactions. External source Vecs
//! are harness captures; production reader/native/model owners are truly paid.
use super::*;
use photocraft_doc::{
    LayerContent,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits, checked_view::NativeTileBytes};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
pub(super) fn session(name: &str) -> (Session, Vec<u8>) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("filter-stack-edits-evidence");
    let proof = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&proof), "6427be73b8d8bca48f372293aa7e9899120dc20ee209ef5c14746072f4e54f70");
    let proof: Value = serde_json::from_slice(&proof).unwrap();
    let expected = proof["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == name).unwrap()["sha256"].as_str().unwrap();
    let bytes = std::fs::read(root.join(name)).unwrap();
    assert_eq!(hash(&bytes), expected);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let owner = photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(imported) = owner else { panic!("exact witnessed Point route required") };
    let mut s = Session::new();
    s.add_owned_point_xcf_import(&imported, name, None, &ctl()).unwrap();
    assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 2);
    (s, bytes)
}
pub(super) fn native(s: &Session) -> String {
    let doc = &s.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.operative_bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(v) | NativeTileBytes::DefaultPixel(v) => digest.update(v),
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    digest.finalize().iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "UNRUN: actual two-node original/reversed order stable second-ID activation and history"]
fn stable_second_id_activation_preserves_first_node_order_raw_native_and_noop_redo() {
    for name in ["stack.source.xcf", "stack.reverse.xcf"] {
        let (mut s, source) = session(name);
        let sourcehash = hash(&source);
        let before = s.active().unwrap().doc.clone();
        let native_before = native(&s);
        let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
        let second = &stack.nodes[1];
        let id = second.id.0.to_string();
        let revision = s.active().unwrap().revision;
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":!second.enabled})).unwrap()["updated"], true);
        let changed = s.active().unwrap().doc.clone();
        let next = &changed.xcf_state.as_ref().unwrap().filter_stacks[0];
        assert_eq!(next.revision, stack.revision + 1);
        assert_eq!(next.nodes[0], stack.nodes[0]);
        assert_eq!(next.nodes[1].id, second.id);
        assert_eq!(next.nodes[1].enabled, !second.enabled);
        assert_eq!(next.nodes[1].arguments, second.arguments);
        assert_eq!(next.nodes[1].composition, second.composition);
        assert_eq!(next.nodes[1].mask, second.mask);
        assert_eq!(s.active().unwrap().revision, revision + 1);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(native(&s), native_before);
        assert_eq!(changed.icc_profile, before.icc_profile);
        assert_eq!(changed.source_color_context, before.source_color_context);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":second.enabled})).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&changed));
        assert_eq!(hash(&source), sourcehash);
    }
}
#[test]
#[ignore = "UNRUN: stable parameter ID, entire witnessed recipe refusal, stale and cancelled transaction"]
fn exact_parameter_target_does_not_bypass_other_node_or_whole_stack_qualification() {
    let (mut s, _) = session("stack.reverse.xcf");
    let before = s.active().unwrap().doc.clone();
    let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
    let exposure = stack.nodes.iter().find(|n| n.operation.name == "gegl:exposure").unwrap();
    let id = exposure.id.0.to_string();
    let patch = |bits| json!({"filterId":id,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":bits}}}]});
    assert!(
        matches!(exposure.arguments.iter().find(|a|a.name=="exposure").unwrap().value,FilterValue::Number(ExactNumber::Float32Bits(v)) if v==0.5f32.to_bits())
    );
    assert_eq!(s.execute("xcf.filter.set", patch(0.5f32.to_bits())).unwrap()["updated"], false);
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let q = s.active().unwrap().doc.quota().unwrap().clone();
    let usage = q.usage().unwrap();
    for params in [patch(0.75f32.to_bits()), json!({"filterId":"18446744073709551615","enabled":false})] {
        let cmd = if params.get("arguments").is_some() { "xcf.filter.set" } else { "xcf.filter.enable" };
        assert!(s.execute(cmd, params).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        assert_eq!(q.usage().unwrap(), usage);
    }
    let second = &stack.nodes[1];
    let p = json!({"filterId":second.id.0.to_string(),"enabled":!second.enabled});
    let prepared = Request::parse(&s, "xcf.filter.enable", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
    s.active_mut().unwrap().history.max_states += 1;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let stop = Interrupt::cancel_only(&|| true);
    assert!(Request::parse(&s, "xcf.filter.enable", &p, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
