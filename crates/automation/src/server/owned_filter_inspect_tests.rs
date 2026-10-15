//! UNRUN retained single-query MCP / legacy-route / denial regression drafts.
use super::*;

#[tokio::test]
async fn ordinary_commands_and_resident_inspection_keep_the_legacy_result_path() {
    let h = Arc::new(Mutex::new(Headless::new()));
    let server = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
    let before = h.lock().unwrap().session.documents().len();
    let error = server.run_command("xcf.filter.inspect".into(), json!({"filterId":"1"}), false).await.unwrap();
    assert_eq!(error.is_error, Some(true));
    assert_eq!(h.lock().unwrap().session.documents().len(), before);
    let result = server.run_command("file.new".into(), json!({"width":2,"height":2}), true).await.unwrap();
    assert_ne!(result.is_error, Some(true));
    assert_eq!(h.lock().unwrap().session.documents().len(), before + 1);
    assert!(!h.lock().unwrap().has_owned_filter_inspection());
    let error = server.run_command("xcf.filter.inspect".into(), json!({"filterId":"01"}), true).await.unwrap();
    assert_eq!(error.is_error, Some(true));
}

mod actual_source {
    use super::*;
    // The engine companion binds the independently authored corpus SHA256
    // manifest and every source. These transport-only drafts reuse those files;
    // no new hashing dependency or independent source qualification is claimed.
    fn headless(alpha: bool) -> Headless {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("requires pinned source corpus")).join(if alpha {
            "alpha-threshold-precision-evidence"
        } else {
            "threshold-precision-evidence"
        });
        let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
        let manifest: Value = serde_json::from_slice(&manifest).unwrap();
        let proof: Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
        for row in proof["checks"].as_array().unwrap() {
            let name = format!("{}.xcf", row["id"].as_str().unwrap());
            assert!(!name.contains(['/', '\\']));
            let path = root.join(&name);
            assert!(std::fs::metadata(&path).unwrap().len() <= 16 << 20);
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(manifest[&name].as_str().unwrap().len(), 64);
            let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            if raw.model != photocraft_xcf::Model::Rgb
                || raw.sample != photocraft_xcf::Sample::F64
                || raw.encoding != photocraft_xcf::Encoding::NonlinearProfile
                || !raw
                    .layers
                    .first()
                    .and_then(|l| l.effects.first())
                    .is_some_and(|f| f.operation == if alpha { "gimp:threshold-alpha" } else { "gimp:threshold" })
            {
                continue;
            }
            let mut h = Headless::trusted_local();
            drop(h.open_owned_xcf_keep_current_bounded(&path).unwrap());
            assert!(h.has_owned_filter_inspection());
            return h;
        }
        panic!("missing pinned closed source")
    }
    fn text(result: &CallToolResult) -> &str {
        result.content.iter().find_map(|v| v.as_text()).unwrap().text.as_str()
    }
    #[tokio::test]
    #[ignore = "UNRUN: requires actual pinned Alpha/Histogram original owned imports"]
    async fn actual_query_keeps_exact_metadata_and_fee_through_sdk_text_but_never_publishes_state() {
        for alpha in [true, false] {
            let mut h = headless(alpha);
            let state = h.session.active().unwrap();
            let snapshot = state.doc.clone();
            let rev = state.revision;
            let history = state.history.clone();
            let journal = h.session.journal.clone();
            let q = snapshot.accounted().unwrap().quota().clone();
            let baseline = q.usage().unwrap();
            let id = snapshot.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
            let params = json!({"filterId":id});
            let owned = h.inspect_owned_filter_bounded(&params, true).unwrap();
            assert!(q.usage().unwrap().index > baseline.index);
            // Existing bounded encoded SDK transport is independent of catalog
            // model ownership; no raw Value is extracted from the paid carrier.
            let expected = crate::budgets::json_bytes(owned.value()).unwrap();
            drop(owned);
            assert_eq!(q.usage().unwrap(), baseline);
            let h = Arc::new(Mutex::new(h));
            let server = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
            for wait in [false, true] {
                let result = server.run_command("xcf.filter.inspect".into(), params.clone(), wait).await.unwrap();
                assert_ne!(result.is_error, Some(true));
                assert_eq!(text(&result).as_bytes(), expected);
                assert!(crate::budgets::json_bytes(&result).is_ok());
                let h = h.lock().unwrap();
                let state = h.session.active().unwrap();
                assert!(state.doc.shares_state(&snapshot));
                assert_eq!(state.revision, rev);
                assert!(state.history.shares_accounted_history(&history));
                assert_eq!(h.session.journal, journal);
                assert_eq!(q.usage().unwrap(), baseline);
            }
        }
    }
    #[tokio::test]
    #[ignore = "UNRUN: requires pinned original Alpha/Histogram project discovery"]
    async fn project_discovery_preserves_pagination_canonical_ids_and_original_authorizer_id() {
        for alpha in [true, false] {
            let h = Arc::new(Mutex::new(headless(alpha)));
            let server = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
            let (snapshot, history, revision, q, baseline) = {
                let h = h.lock().unwrap();
                let state = h.session.active().unwrap();
                let q = state.doc.accounted().unwrap().quota().clone();
                (state.doc.clone(), state.history.clone(), state.revision, q.clone(), q.usage().unwrap())
            };
            let output = server.run_command("xcf.inspect".into(), Value::Null, false).await.unwrap();
            assert_ne!(output.is_error, Some(true));
            let value: Value = serde_json::from_str(text(&output)).unwrap();
            assert_eq!(value["format"], "xcf");
            assert_eq!(value["filters"]["offset"], 0);
            assert_eq!(value["filters"]["limit"], 64);
            let filter = value["filters"]["entries"][0]["id"].as_str().unwrap();
            assert_eq!(filter, snapshot.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string());
            let inspected = server.run_command("xcf.filter.inspect".into(), json!({"filterId":filter}), true).await.unwrap();
            assert_ne!(inspected.is_error, Some(true));
            let page = server.run_command("xcf.inspect".into(), json!({"offset":1,"limit":1}), true).await.unwrap();
            let page: Value = serde_json::from_str(text(&page)).unwrap();
            assert_eq!(page["filters"]["offset"], 1);
            assert_eq!(page["filters"]["limit"], 1);
            assert_eq!(page["filters"]["entries"], json!([]));
            for params in [json!({"limit":0}), json!({"offset":-1}), json!({"limit":129}), json!({"owner":"not-a-project-field"})] {
                assert_eq!(server.run_command("xcf.inspect".into(), params, true).await.unwrap().is_error, Some(true));
            }
            h.lock().unwrap().session.authorize =
                Some(|id, _| if id == "xcf.inspect" { Err(photocraft_engine::EngineError::Other("deny project query".into())) } else { Ok(()) });
            assert_eq!(server.run_command("xcf.inspect".into(), json!({}), true).await.unwrap().is_error, Some(true));
            assert_ne!(server.run_command("xcf.filter.inspect".into(), json!({"filterId":filter}), true).await.unwrap().is_error, Some(true));
            let h = h.lock().unwrap();
            let state = h.session.active().unwrap();
            assert!(state.doc.shares_state(&snapshot));
            assert!(state.history.shares_accounted_history(&history));
            assert_eq!(state.revision, revision);
            assert_eq!(q.usage().unwrap(), baseline);
        }
    }
    #[tokio::test]
    #[ignore = "UNRUN: requires actual pinned Histogram source; authorizer/malformed/batch declines"]
    async fn actual_authorization_and_raw_batch_refusals_cannot_bypass_paid_reply_contract() {
        let h = Arc::new(Mutex::new(headless(false)));
        let server = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
        let (params, snapshot, baseline, q) = {
            let h = h.lock().unwrap();
            let state = h.session.active().unwrap();
            let q = state.doc.accounted().unwrap().quota().clone();
            (json!({"filterId":state.doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string()}), state.doc.clone(), q.usage().unwrap(), q)
        };
        for p in [json!({"filterId":"01"}), json!({"filterId":params["filterId"],"extra":true})] {
            assert_eq!(server.run_command("xcf.filter.inspect".into(), p, true).await.unwrap().is_error, Some(true));
        }
        h.lock().unwrap().session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("explicit policy denial".into())));
        assert_eq!(server.run_command("xcf.filter.inspect".into(), params.clone(), true).await.unwrap().is_error, Some(true));
        h.lock().unwrap().session.authorize = None;
        {
            let mut h = h.lock().unwrap();
            for (id, params) in [("xcf.filter.inspect", params.clone()), ("xcf.inspect", json!({}))] {
                assert!(h.command_start(id, params.clone(), true).is_err());
                assert!(h.handle("engine.execute", json!({"command":id,"params":params})).is_err());
                let batch = h.batch_with_budget(&json!({"steps":[{"command":id,"params":params}]}), BatchReplyBudget::escaped()).unwrap();
                assert_eq!(batch["failed"], 1);
                assert_eq!(batch["results"][0]["ok"], false);
            }
        }
        let h = h.lock().unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&snapshot));
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[path = "../../../xcf/tests/common/mod.rs"]
mod plain_fixture;

mod plain_current {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("photocraft-mcp-plain-current-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn headless(dir: &Directory) -> Headless {
        std::fs::write(dir.0.join("plain.xcf"), plain_fixture::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&dir.0.join("plain.xcf")).unwrap());
        assert!(h.session.active().unwrap().doc.accounted().is_some());
        assert!(h.has_owned_inspection_command("xcf.inspect"));
        assert!(!h.has_owned_inspection_command("xcf.filter.inspect"));
        h
    }
    fn text(reply: &CallToolResult) -> &str {
        reply.content.iter().find_map(|v| v.as_text()).unwrap().text.as_str()
    }
    #[tokio::test]
    #[ignore = "UNRUN: genuine plain owned current metadata MCP reply custody"]
    async fn project_only_plain_discovery_preserves_current_ids_pages_owner_and_sdk_boundary() {
        let dir = Directory::new();
        let h = Arc::new(Mutex::new(headless(&dir)));
        let server = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
        let (snapshot, history, revision, journal, q, baseline) = {
            let h = h.lock().unwrap();
            let st = h.session.active().unwrap();
            let q = st.doc.quota().unwrap().clone();
            (st.doc.clone(), st.history.clone(), st.revision, h.session.journal.clone(), q.clone(), q.usage().unwrap())
        };
        for wait in [false, true] {
            let reply = server.run_command("xcf.inspect".into(), Value::Null, wait).await.unwrap();
            assert_ne!(reply.is_error, Some(true));
            let value: Value = serde_json::from_str(text(&reply)).unwrap();
            assert_eq!(value["format"], "xcf");
            assert_eq!(value["filters"]["total"], 0);
            assert_eq!(value["items"]["total"], 1);
            assert_eq!(value["items"]["entries"][0]["item"]["id"], snapshot.layers[0].id.0.to_string());
            assert!(crate::budgets::json_bytes(&reply).is_ok());
            let page = server.run_command("xcf.inspect".into(), json!({"offset":1,"limit":128}), wait).await.unwrap();
            let page: Value = serde_json::from_str(text(&page)).unwrap();
            assert_eq!(page["items"]["entries"], json!([]));
            assert_eq!(server.run_command("xcf.filter.inspect".into(), json!({"filterId":"1"}), wait).await.unwrap().is_error, Some(true));
        }
        let h = h.lock().unwrap();
        let st = h.session.active().unwrap();
        assert!(st.doc.shares_state(&snapshot));
        assert!(st.history.shares_accounted_history(&history));
        assert_eq!(st.revision, revision);
        assert_eq!(h.session.journal, journal);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    #[tokio::test]
    #[ignore = "UNRUN: plain current discovery auth/malformed/raw-batch refusal and native persistence"]
    async fn plain_discovery_keeps_original_authorizer_and_no_raw_escape_then_reads_saved_current_name() {
        let dir = Directory::new();
        let h = Arc::new(Mutex::new(headless(&dir)));
        let server = PhotocraftMcp::with_backend(Backend::Headless(h.clone()));
        let before = h.lock().unwrap().session.active().unwrap().doc.clone();
        for params in [json!({"limit":0}), json!({"offset":100001}), json!({"path":"not-authority"})] {
            assert_eq!(server.run_command("xcf.inspect".into(), params, true).await.unwrap().is_error, Some(true));
        }
        {
            let mut h = h.lock().unwrap();
            assert!(h.command_run("xcf.inspect", json!({})).is_err());
            let batch = h.handle("batch", json!({"steps":[{"command":"xcf.inspect","params":{}}]})).unwrap();
            assert_eq!(batch["failed"], 1);
            h.session.authorize =
                Some(|id, _| if id == "xcf.inspect" { Err(photocraft_engine::EngineError::Other("deny original project command".into())) } else { Ok(()) });
        }
        assert_eq!(server.run_command("xcf.inspect".into(), json!({}), false).await.unwrap().is_error, Some(true));
        {
            let mut h = h.lock().unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            h.session.authorize = None;
        }
        assert_ne!(server.run_command("layer.setProps".into(), json!({"name":"MCP current name"}), true).await.unwrap().is_error, Some(true));
        let reply = server.run_command("xcf.inspect".into(), json!({}), true).await.unwrap();
        let value: Value = serde_json::from_str(text(&reply)).unwrap();
        assert_eq!(value["items"]["entries"][0]["name"], "MCP current name");
        let original = std::fs::read(dir.0.join("plain.xcf")).unwrap();
        let saved = dir.0.join("current.pcraft");
        h.lock().unwrap().save(None, Some(&saved), Some("pcraft"), &Default::default()).unwrap();
        assert_eq!(std::fs::read(dir.0.join("plain.xcf")).unwrap(), original);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&saved).unwrap());
        let reopened = Arc::new(Mutex::new(reopened));
        let again = PhotocraftMcp::with_backend(Backend::Headless(reopened));
        let reply = again.run_command("xcf.inspect".into(), Value::Null, true).await.unwrap();
        assert_ne!(reply.is_error, Some(true));
        let value: Value = serde_json::from_str(text(&reply)).unwrap();
        assert_eq!(value["items"]["entries"][0]["name"], "MCP current name");
    }
}
