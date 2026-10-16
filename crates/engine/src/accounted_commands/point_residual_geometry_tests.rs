//! UNRUN larger synthetic CURRENT continuation of seven residual/configured precision
//! epochs. Original 8x6 inputs seed metadata/native words only; no larger GIMP
//! geometry, source, output oracle or blanket unit-domain claim is made.
use super::*;
use photocraft_doc::{Document, SampleType, Size};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
#[path = "../../../ui-egui/src/residual_point_geometry_fixture.rs"]
#[allow(dead_code)]
mod fixture;
fn ctl() -> ReadControl<'static> {
    fixture::ctl()
}
fn session(owner: &AccountedDocument) -> Session {
    let mut s = Session::new();
    s.add_owned_native_point_archive(fixture::archive(owner), "Synthetic existing precision role", None, &ctl()).unwrap();
    s
}
fn node(s: &Session) -> &photocraft_doc::xcf_filters::FilterNode {
    &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
}
fn retained(before: &Document, after: &Document) {
    assert_eq!(before.size, after.size);
    assert_eq!(before.mode, after.mode);
    assert_eq!(before.depth, after.depth);
    assert_eq!(before.sample_encoding, after.sample_encoding);
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(fixture::raw(before), fixture::raw(after));
    let (a, b) = (before.xcf_state.as_ref().unwrap(), after.xcf_state.as_ref().unwrap());
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.filter_stacks[0].owner, b.filter_stacks[0].owner);
    assert_eq!(a.filter_stacks[0].nodes[0].composition, b.filter_stacks[0].nodes[0].composition);
}
#[test]
#[ignore = "UNRUN: seven residual/configured epoch roles have real larger native owners, exact argument/activation transactions, readonly rename/history/noop and native custody"]
fn all_seven_existing_roles_keep_native_and_original_metadata() {
    for index in 0..fixture::case_count() {
        let owner = fixture::build(index, Size::new(257, 259));
        let mut s = session(&owner);
        let original = s.active().unwrap().doc.clone();
        let original_node = node(&s).clone();
        assert!(crate::accounted_native_open::is_current_pure_point_work_candidate(s.active().unwrap().doc.accounted().unwrap(), &ctl()).unwrap());
        let changed = fixture::changed(&s.active().unwrap().doc);
        let command = if changed.is_some() { "xcf.filter.set" } else { "xcf.filter.enable" };
        let p = changed.unwrap_or_else(|| json!({"filterId":node(&s).id.0.to_string(),"enabled":!original_node.enabled}));
        let rev = original.xcf_state.as_ref().unwrap().filter_stacks[0].revision;
        assert_eq!(s.execute(command, p.clone()).unwrap()["updated"], true);
        let edited = s.active().unwrap().doc.clone();
        assert_eq!(node(&s).id, original_node.id);
        assert_eq!(node(&s).operation, original_node.operation);
        assert_eq!(node(&s).arguments, original_node.arguments);
        assert_eq!(node(&s).enabled, !original_node.enabled);
        assert_eq!(edited.xcf_state.as_ref().unwrap().filter_stacks[0].revision, rev + 1);
        retained(&original, &edited);
        let history = s.active().unwrap().history.clone();
        assert_eq!(s.execute(command, p).unwrap()["updated"], false);
        assert!(s.active().unwrap().doc.shares_state(&edited));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        s.execute("layer.setProps", json!({"name":"Seven-role current name"})).unwrap();
        let named = s.active().unwrap().doc.clone();
        assert_eq!(named.layers[0].name, "Seven-role current name");
        assert_eq!(named.xcf_state.as_ref().unwrap().items[0].name, "Seven-role current name");
        retained(&original, &named);
        let reopened = session(named.accounted().unwrap());
        retained(&named, &reopened.active().unwrap().doc);
        assert_eq!(reopened.active().unwrap().doc.xcf_state, named.xcf_state);
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&edited));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&named));
    }
}
#[test]
#[ignore = "UNRUN: origin geometry does not grant singleton Move/remove/directcompound or a new derived-empty route"]
fn geometry_does_not_expand_command_or_empty_authority() {
    for index in 0..fixture::case_count() {
        let owner = fixture::build(index, Size::new(1, 257));
        let mut s = session(&owner);
        let before = s.active().unwrap().doc.clone();
        let h = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let id = node(&s).id.0.to_string();
        let layer = s.active().unwrap().active_layer.unwrap();
        for (command, p) in
            [("layer.translate", json!({"dx":1,"dy":0})), ("xcf.filter.move", json!({"filterId":id,"index":0})), ("xcf.filter.remove", json!({"filterId":id}))]
        {
            assert!(s.execute(command, p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(s.active().unwrap().history.shares_accounted_history(&h));
            assert_eq!(s.journal, journal);
            assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 1);
        }
        assert!(
            s.edit_accounted(
                "No new compound grant",
                photocraft_doc::accounted_document::AccountedEdit::LayerMetadata { layer, name: None, visible: None, opacity: None },
                &ctl()
            )
            .is_err()
        );
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.execute("layer.setProps", json!({"name":"No gesture grant","coalesce":"seven-role"})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        // No remove success => no family-derived empty can be published. The
        // independent preexisting RGB float empty CURRENT policy is unchanged.
    }
}
struct Extra {
    _root: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
fn changed_owner(root: AccountedDocument, change: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = root.quota().clone();
    let index = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut d = root.document().clone();
    change(&mut d);
    assert!(photocraft_io::retained_native_model_census(&d, Default::default(), &Interrupt::NONE).unwrap().retained_bytes() <= 4 << 20);
    AccountedDocument::new_readonly_point_storage(d, Arc::new(Extra { _root: root, _index: index, _resident: resident }), q, &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: full BYTE-zero mask default and boundary padding proof survives disabled filter; numeric negative zero is not byte zero"]
fn disabled_filters_still_reject_nonzero_physical_mask_bytes() {
    for index in 0..fixture::case_count() {
        let root = fixture::build(index, Size::new(257, 259));
        let original_mask = &root.document().xcf_state.as_ref().unwrap().filter_masks[0];
        assert!(original_mask.surface.default_bytes().iter().all(|byte| *byte == 0));
        assert!(original_mask.surface.tiles().all(|(_, tile)| tile.bytes().iter().all(|byte| *byte == 0)));
        let disabled = changed_owner(root, |d| d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false);
        let valid = session(&disabled);
        assert!(!node(&valid).enabled);
        for variant in 0..2 {
            let bad = changed_owner(disabled.clone(), |d| {
                let mask = &mut d.xcf_state.as_mut().unwrap().filter_masks[0];
                let sample = mask.surface.format().sample;
                let byte: Vec<u8> = match sample {
                    SampleType::F32 => (-0.0f32).to_ne_bytes().to_vec(),
                    SampleType::F64 => (-0.0f64).to_ne_bytes().to_vec(),
                    _ => {
                        let mut b = vec![0; sample.bytes()];
                        b[0] = 1;
                        b
                    }
                };
                if variant == 0 {
                    let mut surface = Surface::try_with_default_bytes(mask.surface.format(), &byte).unwrap();
                    for (coord, tile) in mask.surface.tiles() {
                        surface.put_tiles(std::iter::once((*coord, Arc::clone(tile))));
                    }
                    mask.surface = surface;
                } else {
                    // Inside the retained boundary tile, outside operative canvas.
                    mask.surface.try_write_interleaved_with(Rect::new(258, 259, 259, 260), &byte, &Interrupt::NONE).unwrap();
                }
            });
            let mut target = Session::new();
            assert!(target.add_owned_native_point_archive(fixture::archive(&bad), "Invalid physical mask", None, &ctl()).is_err());
            assert!(target.active().is_none());
        }
    }
}
#[test]
#[ignore = "UNRUN: exact residual/configured service pairs and scalar/deep configuration types remain closed; no Levels/Curves/opaque-original enrollment"]
fn service_pairs_and_typed_configuration_boundaries_remain_closed() {
    for index in 0..fixture::case_count() {
        let root = fixture::build(index, Size::new(257, 1));
        for wrong in ["wrong-compatibility", "xcf-22-stock-point-replace-fields-v1"] {
            let owner = changed_owner(root.clone(), |d| {
                let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                n.composition.as_mut().unwrap().compatibility_epoch = wrong.into();
            });
            assert!(!crate::accounted_native_open::is_current_pure_point_work_candidate(&owner, &ctl()).unwrap());
        }
        for (operation, schema) in [
            ("gegl:value-invert", "xcf-22-stock-point-replace"),
            ("gimp:levels", "xcf-22-stock-levels-replace"),
            ("gimp:curves", "xcf-22-stock-curves-replace"),
        ] {
            let excluded = changed_owner(root.clone(), |d| {
                let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
                n.operation.name = operation.into();
                n.operation.schema_epoch = schema.into();
                n.composition.as_mut().unwrap().compatibility_epoch = schema.into();
            });
            assert!(!crate::accounted_native_open::is_current_pure_point_work_candidate(&excluded, &ctl()).unwrap());
        }
        let wrong_version = changed_owner(root.clone(), |d| d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "0:1".into());
        assert!(!crate::accounted_native_open::is_current_pure_point_work_candidate(&wrong_version, &ctl()).unwrap());
        let mut s = session(&root);
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let id = node(&s).id.0.to_string();
        assert!(s.execute("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":"unrecognized","value":{"Number":{"Float32Bits":0}}}]})).is_err());
        if node(&s).operation.name == "gimp:color-balance" {
            let valid = fixture::changed(&s.active().unwrap().doc).unwrap();
            for variant in 0..6 {
                let mut bad = valid.clone();
                let config = bad["arguments"].as_array_mut().unwrap().iter_mut().find(|a| a["name"] == "config").unwrap();
                let ranges = config["value"]["Config"].as_array_mut().unwrap();
                match variant {
                    0 => ranges.swap(0, 1),
                    1 => {
                        ranges[0]["value"]["Config"].as_array_mut().unwrap().pop();
                    }
                    2 => ranges[0]["value"]["Config"][1]["name"] = json!("cyan-red"),
                    3 => ranges[0]["value"]["Config"][0]["value"] = json!({"Number":{"Float32Bits":0}}),
                    4 => ranges[0]["value"]["Config"][0]["value"] = json!({"Number":{"Float64Bits":"0x7ff8000000000000"}}),
                    _ => ranges[0]["value"]["Config"][0]["value"] = json!({"Number":{"Float64Bits":format!("0x{:016x}",2f64.to_bits())}}),
                }
                assert!(s.execute("xcf.filter.set", bad).is_err());
                assert!(s.active().unwrap().doc.shares_state(&before));
                assert!(s.active().unwrap().history.shares_accounted_history(&history));
                assert_eq!(s.journal, journal);
            }
        } else if matches!(node(&s).operation.name.as_str(), "gegl:saturation" | "gimp:desaturate") {
            let name = if node(&s).operation.name == "gegl:saturation" { "colorspace" } else { "mode" };
            let stored = node(&s).arguments.iter().find(|a| a.name == name).unwrap();
            let photocraft_doc::xcf_filters::FilterValue::Enum { domain, .. } = &stored.value else { panic!("exact stored enum") };
            let domain = domain.clone();
            for value in [json!({"Enum":{"domain":"wrong-domain","value":0}}), json!({"Enum":{"domain":domain,"value":99}})] {
                assert!(s.execute("xcf.filter.set", json!({"filterId":id,"arguments":[{"name":name,"value":value}]})).is_err());
            }
        }
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
    }
}
#[test]
#[ignore = "UNRUN: one cumulative finite ledger through paid old/staged/reply/journal work; Work/cancel/stale-view failures publish no state"]
fn same_control_and_publication_fences_are_atomic() {
    for index in 0..fixture::case_count() {
        let root = fixture::build(index, Size::new(257, 3));
        let mut s = session(&root);
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let p = json!({"filterId":node(&s).id.0.to_string(),"enabled":!node(&s).enabled});
        let work =
            photocraft_io::accounted_operation_work::OperationWork::standard(&photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()).unwrap();
        let deny = AtomicBool::new(false);
        let debits = AtomicU64::new(0);
        let charge = |n| {
            debits.fetch_add(1, Ordering::Relaxed);
            if deny.load(Ordering::Relaxed) { Err(BackingError::Work) } else { work.check(n) }
        };
        let deadline = || work.deadline_expired();
        let interrupt = Interrupt::cancel_only(&deadline);
        let c = ReadControl { interrupt: &interrupt, work: &charge };
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &c).unwrap();
        let used = work.diagnostics().used;
        assert!(used > 0);
        deny.store(true, Ordering::Relaxed);
        assert!(prepared.execute(&mut s, &c).is_err());
        assert_eq!(work.diagnostics().used, used);
        assert!(debits.load(Ordering::Relaxed) > 0);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &ctl()).unwrap();
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        assert!(prepared.execute(&mut s, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.journal, journal);
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &ctl()).unwrap();
        s.active_mut().unwrap().view_revision += 1;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.journal, journal);
        let mut stale_source = session(&root);
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&stale_source, "xcf.filter.enable", &p, &ctl()).unwrap();
        let source_snapshot = stale_source.active().unwrap().doc.clone();
        stale_source.active_mut().unwrap().source.as_mut().unwrap().label.push('x');
        assert!(prepared.execute(&mut stale_source, &ctl()).is_err());
        assert!(stale_source.active().unwrap().doc.shares_state(&source_snapshot));
        let mut bad_history = session(&root);
        let foreign = session(&root);
        let foreign_history = foreign.active().unwrap().history.clone();
        let history_snapshot = bad_history.active().unwrap().doc.clone();
        bad_history.active_mut().unwrap().history = foreign_history.clone();
        assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&bad_history, "xcf.filter.enable", &p, &ctl()).is_err());
        assert!(bad_history.active().unwrap().doc.shares_state(&history_snapshot));
        assert!(bad_history.active().unwrap().history.shares_accounted_history(&foreign_history));
    }
}
