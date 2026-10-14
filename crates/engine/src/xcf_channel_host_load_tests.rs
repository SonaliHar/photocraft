//! UNRUN drafts. The positive lane needs an explicitly supplied sealed original
//! XCF and corresponding canonical native/current archive; it never treats a
//! synthetic node or source label as an original-byte proof.
use super::*;
fn limits() -> HostLoadLimits {
    HostLoadLimits {
        aggregate: Quota { resident_bytes: 384 << 20, index_bytes: 16 << 20, work_units: 1 << 34 },
        original: Quota { resident_bytes: 64 << 20, index_bytes: 2 << 20, work_units: 16_000_000 },
        mapping: MappingLimits::default(),
        current: photocraft_format::PreparationLimits { resident_bytes: 64 << 20, index_bytes: 1 << 20, manifest_bytes: 64 << 20, object_bytes: 64 << 20 },
        selected: SelectedLimits { resident_bytes: 128 << 20, index_bytes: 4 << 20, work_units: 1 << 32 },
        publication: crate::xcf_channel_stored_load::Limits { resident_bytes: 128 << 20, index_bytes: 4 << 20, work_units: 1 << 31 },
    }
}
#[test]
fn whole_phase_quota_and_lower_work_overflow_refuse_without_resetting_receipts() {
    let phase = PhaseWork::new(20);
    phase.charge(17).unwrap();
    assert!(phase.charge(4).is_err());
    assert_eq!(phase.remaining(), 3);
    assert!(phase.exhausted.load(Ordering::Relaxed));
    assert!(add(u64::MAX, 1).is_err());
    let base = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let mut low = limits();
    low.original.work_units = base.get(BudgetKind::Work) + 1;
    assert!(validate_limits(&low, &base).is_err());
}
#[test]
fn pre_cancelled_host_entry_does_not_construct_or_publish_original_owner() {
    let mut session = Session::new();
    let context = JobCtx::new();
    context.cancel();
    let target = CapturedStoredTarget { document: DocId(1), revision: 0, channel: ChannelId(1) };
    assert!(matches!(
        load_from_original(&mut session, target, b"host input", 0, limits(), CurrentProfilePolicy::OriginalExact, &context),
        Err(EngineError::Cancelled)
    ));
    assert!(session.documents().is_empty());
    assert!(session.stored_load_offer.is_none());
}
fn fixture() -> (Session, Vec<u8>, ChannelId, usize, CurrentProfilePolicy) {
    let original_path = std::env::var("PHOTOCRAFT_STORED_HOST_ORIGINAL").expect("set sealed original XCF path explicitly");
    let current_path = std::env::var("PHOTOCRAFT_STORED_HOST_CURRENT").expect("set corresponding canonical current .pcraft path explicitly");
    let channel = ChannelId(std::env::var("PHOTOCRAFT_STORED_HOST_CHANNEL").expect("set canonical saved channel ID").parse().unwrap());
    let ordinal = std::env::var("PHOTOCRAFT_STORED_HOST_ORDINAL").expect("set independent original channel ordinal").parse().unwrap();
    let policy = match std::env::var("PHOTOCRAFT_STORED_HOST_PROFILE_POLICY").expect("choose originalExact or materializedImplicit").as_str() {
        "originalExact" => CurrentProfilePolicy::OriginalExact,
        "materializedImplicit" => CurrentProfilePolicy::MaterializedImplicit,
        other => panic!("unknown explicit profile policy {other}"),
    };
    let original = std::fs::read(original_path).unwrap();
    let current = std::fs::read(current_path).unwrap();
    let imported = photocraft_io::import("host-current.pcraft", &current).unwrap();
    let mut session = Session::new();
    session.add_document(imported.document, None).unwrap();
    (session, original, channel, ordinal, policy)
}
fn target(session: &Session, channel: ChannelId) -> CapturedStoredTarget {
    let state = session.active().unwrap();
    CapturedStoredTarget { document: state.doc.id, revision: state.revision, channel }
}
#[test]
#[ignore = "requires explicit sealed original/current host input; UNRUN draft"]
fn genuine_original_host_transfer_preserves_source_and_noop_redo() {
    let (mut session, original, channel, ordinal, policy) = fixture();
    let source = session.active().unwrap().doc.channels.iter().find(|value| value.id == channel).unwrap().surface.clone();
    let before = session.active().unwrap().doc.clone();
    let captured = target(&session, channel);
    let output = load_from_original(&mut session, captured, &original, ordinal, limits(), policy, &JobCtx::new()).unwrap();
    assert_eq!(output, serde_json::json!({"selected":true}));
    let bounds = photocraft_geom::Rect::new(0, 0, 32, 32);
    assert_eq!(session.active().unwrap().doc.selection.as_ref().unwrap().try_read_interleaved(bounds).unwrap(), source.try_read_interleaved(bounds).unwrap());
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks, before.xcf_state.as_ref().unwrap().filter_stacks);
    session
        .edit("Own unrelated marker", |doc, _| {
            doc.name.push('x');
            Ok(())
        })
        .unwrap();
    assert!(session.undo());
    assert!(session.active().unwrap().history.can_redo());
    let noop = session.active().unwrap().doc.clone();
    let captured = target(&session, channel);
    load_from_original(&mut session, captured, &original, ordinal, limits(), policy, &JobCtx::new()).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&noop));
    assert!(session.active().unwrap().history.can_redo());
    assert!(session.stored_load_offer.is_none());
}
#[test]
#[ignore = "requires explicit sealed original/current host input; UNRUN draft"]
fn stale_wrong_original_and_explicit_current_profile_policy_refuse_without_history_edit() {
    let (mut session, mut original, channel, ordinal, policy) = fixture();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let mut captured = target(&session, channel);
    captured.revision += 1;
    assert!(load_from_original(&mut session, captured, &original, ordinal, limits(), policy, &JobCtx::new()).is_err());
    let wrong_policy = match policy {
        CurrentProfilePolicy::OriginalExact => CurrentProfilePolicy::MaterializedImplicit,
        CurrentProfilePolicy::MaterializedImplicit => CurrentProfilePolicy::OriginalExact,
    };
    let captured = target(&session, channel);
    assert!(load_from_original(&mut session, captured, &original, ordinal, limits(), wrong_policy, &JobCtx::new()).is_err());
    original[0] ^= 1; // genuine source-byte discriminator, not a current-node substitute
    let captured = target(&session, channel);
    assert!(load_from_original(&mut session, captured, &original, ordinal, limits(), policy, &JobCtx::new()).is_err());
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert!(session.stored_load_offer.is_none());
}

#[test]
fn prospective_host_recipe_refuses_missing_headroom_and_hidden_idle_context_before_source_io() {
    let mut session = Session::new();
    session.execute("file.new", serde_json::json!({"width":1,"height":1})).unwrap();
    let captured = target(&session, ChannelId(2));
    let mut low = limits();
    low.aggregate.resident_bytes = low.original.resident_bytes + low.selected.resident_bytes + low.publication.resident_bytes;
    assert!(validate_host_request(&session, captured, 16384, &low).is_err());
    session.coalesce_request = Some("Own pending gesture".into());
    assert!(validate_host_request(&session, captured, 16384, &limits()).is_err());
    assert!(session.stored_load_offer.is_none());
}
