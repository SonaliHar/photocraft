//! UNRUN wire, transaction and pinned-source integration regression drafts.
use super::*;
use photocraft_raster::Interrupt;
use serde_json::json;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn set(filter: &str, bits: u32) -> Value {
    json!({"filterId":filter,"arguments":[{"name":"value","value":{"Number":{"Float32Bits":bits}}}]})
}

#[test]
fn exact_scalar_wire_retains_signed_zero_and_only_its_journal_depth_exception() {
    for bits in [0, (-0.0f32).to_bits(), 0.25f32.to_bits(), 1f32.to_bits()] {
        let p = set("18446744073709551615", bits);
        assert_eq!(wire("xcf.filter.set", &p).unwrap().value, Some(bits));
        assert_eq!(journal_depth("xcf.filter.set", &p), 5);
        assert_eq!(journal_depth("layer.setProps", &p), 4);
    }
    assert_eq!(journal_depth("xcf.filter.enable", &json!({"filterId":"1","enabled":false})), 4);
    assert!(wire("xcf.filter.enable", &json!({"filterId":"1","enabled":false})).is_some());
}

#[test]
fn unsupported_storage_opacity_and_extra_shapes_never_get_deeper_journal() {
    for p in [
        json!({"filterId":"1","opacityF64Bits":"0x3fe0000000000000"}),
        json!({"filterId":"1","enabled":true,"coalesce":null}),
        json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float64Bits":"0x3fd0000000000000"}}}]}),
        json!({"filterId":"1","arguments":[{"name":"other","value":{"Number":{"Float32Bits":0}}}]}),
        json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float32Bits":0,"Signed":0}}}]}),
        json!({"filterId":"1","arguments":[{"name":"value","value":{"Number":{"Float32Bits":0}}},{"name":"value","value":{"Number":{"Float32Bits":0}}}]}),
    ] {
        assert!(wire("xcf.filter.set", &p).is_none());
        assert_eq!(journal_depth("xcf.filter.set", &p), 4);
    }
    assert!(wire("xcf.filter.set", &json!({"filterId":"1","opacityF64Bits":"0x3ff0000000000000"})).is_some());
}

#[test]
fn malformed_identity_nonfinite_out_of_unit_and_activation_shapes_fail_closed() {
    for id in ["0", "01", "+1", "18446744073709551616", ""] {
        assert!(wire("xcf.filter.set", &set(id, 0)).is_none());
    }
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.01f32).to_bits(), 1.01f32.to_bits()] {
        assert!(wire("xcf.filter.set", &set("1", bits)).is_none());
    }
    for p in [json!({"filterId":"1"}), json!({"filterId":"1","enabled":0}), json!({"filterId":"1","enabled":true,"arguments":[]})] {
        assert!(wire("xcf.filter.enable", &p).is_none());
    }
    let stop = || true;
    assert!(matches!(
        Request::parse(&Session::new(), "xcf.filter.set", &set("1", 0), &ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) }),
        Err(EngineError::Cancelled)
    ));
}

#[cfg(feature = "corpus")]
mod actual_source {
    use super::*;
    use photocraft_raster::backing::quota::{QuotaKind, QuotaLimits};
    use sha2::{Digest, Sha256};
    use std::{path::PathBuf, sync::Arc};
    fn hash(v: &[u8]) -> String {
        Sha256::digest(v).iter().map(|b| format!("{b:02x}")).collect()
    }
    fn session() -> Session {
        let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("requires pinned precision evidence"))
            .join("alpha-threshold-precision-evidence");
        let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
        assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
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
            let len = std::fs::metadata(&path).unwrap().len();
            assert!(len <= 16 << 20);
            let capture = q.reserve(QuotaKind::Resident, 2 * len + 4096).unwrap();
            let bytes = Arc::new(std::fs::read(path).unwrap());
            assert!(bytes.capacity() as u64 <= 2 * len);
            assert_eq!(hash(&bytes), manifest[&name].as_str().unwrap());
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            if raw.model != photocraft_xcf::Model::Rgb
                || raw.sample != photocraft_xcf::Sample::F64
                || raw.encoding != photocraft_xcf::Encoding::NonlinearProfile
                || !raw.layers.first().and_then(|l| l.effects.first()).is_some_and(|f| f.operation == "gimp:threshold-alpha" && f.state.enabled == Some(true))
            {
                continue;
            }
            let imported =
                photocraft_io::import_owned_staged_threshold_alpha_bytes_in_quota(&name, bytes, &Default::default(), &limits, q.clone(), &ctl()).unwrap();
            let mut session = Session::new();
            session.add_owned_threshold_alpha_xcf_import(&imported, &name, Some("owned/original.xcf"), &ctl()).unwrap();
            drop(imported);
            drop(capture);
            return session;
        }
        panic!("missing pinned qualified RGB/F64 source")
    }
    fn filter(s: &Session) -> String {
        s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()
    }
    fn run(s: &mut Session, p: &Value) -> Value {
        crate::accounted_commands::PreparedAccountedCommand::prepare(s, "xcf.filter.set", p, &ctl()).unwrap().execute(s, &ctl()).unwrap()
    }
    #[test]
    #[ignore = "UNRUN: pinned independently authored source required"]
    fn registered_patch_enable_noop_undo_redo_and_reply_keep_original_owner() {
        let mut s = session();
        let id = filter(&s);
        let before = s.active().unwrap().doc.clone();
        let q = before.accounted().unwrap().quota().clone();
        let old_node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let old_bits = match old_node.arguments[0].value {
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(v)) => v,
            _ => panic!(),
        };
        let bits = if old_bits == 0.25f32.to_bits() { 0.75f32.to_bits() } else { 0.25f32.to_bits() };
        assert_eq!(run(&mut s, &set(&id, bits)), json!({"filterId":id,"updated":true}));
        let changed = s.active().unwrap().doc.clone();
        assert!(changed.accounted().unwrap().quota().shares_ledger(&q));
        assert_eq!(changed.source_color_context, before.source_color_context);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Edit GIMP Stock Filter"));
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&before));
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        assert_eq!(run(&mut s, &set(&id, old_bits))["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.redo_with_control(&ctl()).unwrap());
        let p = json!({"filterId":id,"enabled":false});
        let out = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &p, &ctl()).unwrap().execute(&mut s, &ctl()).unwrap();
        assert_eq!(out["updated"], true);
        assert!(!s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    }
    #[test]
    #[ignore = "UNRUN: pinned independently authored source required"]
    fn stale_view_source_history_cancel_and_work_leave_model_unpublished() {
        let mut s = session();
        let id = filter(&s);
        let p = set(&id, 0.5f32.to_bits());
        let before = s.active().unwrap().doc.clone();
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        s.active_mut().unwrap().source_read_only = false;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().source_read_only = true;
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        let max_states = s.active().unwrap().history.max_states;
        s.active_mut().unwrap().history.max_states = max_states + 1;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().history.max_states = max_states;
        let deny = |_: u64| Err(BackingError::Work);
        let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
        assert!(Request::parse(&s, "xcf.filter.set", &p, &c).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        let stop = || true;
        assert!(matches!(prepared.execute(&mut s, &ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().revision += 1;
        assert!(s.active().unwrap().source.as_ref().unwrap().check_destination("owned/original.xcf").is_err());
    }

    #[test]
    #[ignore = "UNRUN: synthetic CURRENT native lock metadata over pinned owned source; not GIMP-authored lock evidence"]
    fn actual_native_locked_current_owners_refuse_edits_and_visibility_only_guards_changes() {
        use photocraft_format::{
            LoadOptions,
            canonical_archive::ImmutableCanonicalArchive,
            owned_document::{load_document_from_owned_archive, save_current_document_archive},
        };
        #[derive(Debug)]
        struct Archive {
            bytes: Vec<u8>,
            q: BackingQuota,
            _fee: photocraft_raster::backing::quota::QuotaLease,
        }
        impl ImmutableCanonicalArchive for Archive {
            fn len(&self) -> u64 {
                self.bytes.len() as u64
            }
            fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
                if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
            }
            fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
                c.check(out.len() as u64)?;
                let at = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
                let end = at.checked_add(out.len()).ok_or(BackingError::Length)?;
                out.copy_from_slice(self.bytes.get(at..end).ok_or(BackingError::Bounds)?);
                Ok(())
            }
        }
        for lock in 0..4 {
            let source = session();
            let owner = source.active().unwrap().doc.accounted().unwrap();
            let q = owner.quota().clone();
            let model =
                photocraft_io::retained_native_model_bytes(owner.document(), photocraft_io::NativeSaveLimits::default().preparation, &Interrupt::NONE).unwrap();
            assert!(model <= 2 << 20, "bounded fixture only");
            // One current layer/mask; held original backed payloads stay shared.
            // Pay model containers plus the bounded one-mask sparse map before
            // this synthetic CURRENT metadata clone, without source admission.
            let metadata = q.reserve(QuotaKind::Index, (2 * model + (1 << 20)) as u64).unwrap();
            let mut doc = owner.document().clone();
            let layer = doc.layers[0].id;
            match lock {
                0 => doc.layers[0].locks.all = true,
                1 => doc.layers[0].locks.pixels = true,
                2 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.content = true,
                _ => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().locks.visibility = true,
            }
            let encoded_fee = q.reserve(QuotaKind::Resident, (8 << 20) + 256).unwrap();
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(8 << 20).unwrap();
            assert!(bytes.capacity() <= 8 << 20);
            let bytes = save_current_document_archive(&doc, std::io::Cursor::new(bytes), Default::default(), &q, &ctl()).unwrap().into_inner();
            assert!(bytes.capacity() <= 8 << 20);
            let archive = Arc::new(Archive { bytes, q: q.clone(), _fee: encoded_fee });
            drop(doc);
            drop(metadata);
            let loaded = load_document_from_owned_archive(archive, &LoadOptions::default(), Default::default(), q.clone(), &ctl()).unwrap();
            let mut s = Session::new();
            s.add_owned_native_threshold_alpha_archive(loaded, "locked-current.pcraft", None, &ctl()).unwrap();
            let id = filter(&s);
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.clone();
            let revision = s.active().unwrap().revision;
            let journal = s.journal.clone();
            let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let bits = match node.arguments[0].value {
                photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(v)) => v,
                _ => panic!(),
            };
            let newbits = if bits == 0.25f32.to_bits() { 0.75f32.to_bits() } else { 0.25f32.to_bits() };
            if lock < 3 {
                for (command, p) in [("xcf.filter.set", set(&id, newbits)), ("xcf.filter.enable", json!({"filterId":id,"enabled":!node.enabled}))] {
                    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, command, &p, &ctl()).is_err());
                    assert!(s.active().unwrap().doc.shares_state(&before));
                    assert!(s.active().unwrap().history.shares_accounted_history(&history));
                    assert_eq!(s.active().unwrap().revision, revision);
                    assert_eq!(s.journal, journal);
                }
            } else {
                // Content-only edits remain allowed under a visibility lock.
                assert_eq!(run(&mut s, &set(&id, newbits))["updated"], true);
                assert!(s.undo_with_control(&ctl()).unwrap());
                let redo = s.active().unwrap().history.clone();
                let rev = s.active().unwrap().revision;
                let same = json!({"filterId":id,"enabled":node.enabled});
                let out = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &same, &ctl())
                    .unwrap()
                    .execute(&mut s, &ctl())
                    .unwrap();
                assert_eq!(out["updated"], false);
                assert!(s.active().unwrap().history.shares_accounted_history(&redo));
                assert_eq!(s.active().unwrap().revision, rev);
                let changed = json!({"filterId":id,"enabled":!node.enabled});
                assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "xcf.filter.enable", &changed, &ctl()).is_err());
                assert!(s.active().unwrap().history.shares_accounted_history(&redo));
                assert!(s.redo_with_control(&ctl()).unwrap());
            }
        }
    }
}
