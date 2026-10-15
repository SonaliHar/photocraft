//! UNRUN exact existing MOVE wire and genuine witnessed Point stack transactions.
use super::*;
use serde_json::json;
#[test]
fn move_wire_is_exact_canonical_and_does_not_gain_deep_journal_authority() {
    for index in [0, 1, 1023] {
        let p = json!({"filterId":"18446744073709551615","index":index});
        let (f, i) = move_wire(&p).unwrap();
        assert_eq!(f.filter, FilterId(u64::MAX));
        assert_eq!(i, index);
        assert_eq!(journal_depth("xcf.filter.move", &p), 4);
    }
    for p in [
        json!({"filterId":"01","index":0}),
        json!({"filterId":"0","index":0}),
        json!({"filterId":"1","index":1024}),
        json!({"filterId":"1","index":-1}),
        json!({"filterId":"1","index":0.5}),
        json!({"filterId":"1","index":0,"coalesce":null}),
        json!({"filterId":"1"}),
    ] {
        assert!(move_wire(&p).is_none());
    }
}
#[cfg(all(feature = "corpus", not(target_arch = "wasm32")))]
mod actual {
    use super::super::stack_tests::{native, session};
    use super::*;
    use photocraft_raster::Interrupt;
    fn ctl() -> ReadControl<'static> {
        ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
    }
    #[test]
    #[ignore = "UNRUN: genuine witnessed order swap and disabled descriptor preservation through one undo/noopredo"]
    fn registered_move_swaps_only_current_order_and_retains_stable_nodes_native_and_history() {
        for name in ["stack.source.xcf", "stack.disable.xcf"] {
            let (mut s, _) = session(name);
            let before = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
            let id = stack.nodes[0].id.0.to_string();
            let revision = s.active().unwrap().revision;
            assert_eq!(s.execute("xcf.filter.move", json!({"filterId":id,"index":1})).unwrap(), json!({"filterId":id,"updated":true}));
            let moved = s.active().unwrap().doc.clone();
            let next = &moved.xcf_state.as_ref().unwrap().filter_stacks[0];
            assert_eq!(next.nodes[0], stack.nodes[1]);
            assert_eq!(next.nodes[1], stack.nodes[0]);
            assert_eq!(next.revision, stack.revision + 1);
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Move GIMP Stock Filter"));
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(native(&s), raw);
            assert_eq!(moved.icc_profile, before.icc_profile);
            assert_eq!(moved.source_color_context, before.source_color_context);
            assert!(s.undo_with_control(&ctl()).unwrap());
            let history = s.active().unwrap().history.clone();
            let rev = s.active().unwrap().revision;
            assert_eq!(s.execute("xcf.filter.move", json!({"filterId":id,"index":0})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(s.active().unwrap().revision, rev);
            assert!(s.redo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&moved));
        }
    }
    #[test]
    #[ignore = "UNRUN: actual Point move unknown target/index, coalesce/background, stale/cancel/quota rollback"]
    fn refused_or_stale_move_cannot_publish_or_clear_redo() {
        let (mut s, _) = session("stack.source.xcf");
        let before = s.active().unwrap().doc.clone();
        let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
        let id = stack.nodes[0].id.0.to_string();
        let p = json!({"filterId":id,"index":1});
        let history = s.active().unwrap().history.clone();
        let journal = s.journal.clone();
        let q = s.active().unwrap().doc.quota().unwrap().clone();
        let usage = q.usage().unwrap();
        for invalid in [json!({"filterId":id,"index":2}), json!({"filterId":"18446744073709551615","index":0})] {
            assert!(s.execute("xcf.filter.move", invalid).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert_eq!(s.journal, journal);
            assert_eq!(q.usage().unwrap(), usage);
        }
        s.coalesce_request = Some("gesture".into());
        assert!(Request::parse(&s, "xcf.filter.move", &p, &ctl()).is_err());
        assert_eq!(s.coalesce_request.as_deref(), Some("gesture"));
        s.coalesce_request = None;
        assert!(crate::xcf_filter_cmds::pre_dispatch_point_move(&s, "xcf.filter.move", &p, true).is_err());
        let prepared = Request::parse(&s, "xcf.filter.move", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        s.active_mut().unwrap().channel_view.quick_mask_hidden = true;
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        s.active_mut().unwrap().channel_view.quick_mask_hidden = false;
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        let stopped = Interrupt::cancel_only(&|| true);
        assert!(Request::parse(&s, "xcf.filter.move", &p, &ReadControl { interrupt: &stopped, work: &|_| Ok(()) }).is_err());
        let blocker = q.reserve(QuotaKind::Index, (32 << 20) - q.usage().unwrap().index).unwrap();
        assert!(Request::parse(&s, "xcf.filter.move", &p, &ctl()).unwrap().prepare(&s, &ctl()).is_err());
        drop(blocker);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(q.usage().unwrap(), usage);
    }
}
