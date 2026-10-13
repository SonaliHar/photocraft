//! Explicit recovery for decoded profile-question imports not yet opened.
//! This utility never grants source access or alters current tabs/history.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
const ID: &str = "file.open.discardPending";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
fn held_id(params: &Value) -> Result<Option<u64>> {
    if params.is_null() {
        return Ok(None);
    }
    let fields = params.as_object().ok_or_else(|| bad("params must be an object"))?;
    if fields.len() > 1 || fields.keys().any(|key| key != "heldId") {
        return Err(bad("only optional heldId is accepted"));
    }
    let Some(value) = fields.get("heldId") else { return Ok(None) };
    let text = value.as_str().ok_or_else(|| bad("heldId must be a canonical nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("heldId must be a canonical nonzero decimal string"));
    }
    let id = text.parse::<u64>().map_err(|_| bad("heldId is out of range"))?;
    Ok(Some(id))
}
/// Root dispatch may call this borrowed bounded check before generic cloning.
pub(crate) fn dispatch_preflight(params: &Value) -> Result<()> {
    held_id(params).map(|_| ())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    match held_id(params)? {
        Some(id) => {
            if session.held_open_with_origin(id).is_none() {
                return Err(bad("heldId does not identify a pending import"));
            }
            let response = json!({"discarded":1,"heldId":id.to_string()});
            if !session.discard_held_open(id) {
                return Err(bad("heldId no longer identifies a pending import"));
            }
            Ok(response)
        }
        None => {
            let response = json!({"discarded":session.held_open_count()});
            session.discard_held_opens();
            Ok(response)
        }
    }
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Cancel Failed Opens",
        menu: &[],
        shortcut: None,
        params: r#"{heldId?:canonical nonzero decimal string} → {discarded,heldId?}; explicitly discard one or all private unopened imports; existing tabs/history and UI profile queue remain unchanged"#,
        enabled: |_| Ok(()),
        run,
        journal: false,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_identity_and_unknown_fields_refuse() {
        for params in [
            json!({"heldId":0}),
            json!({"heldId":"0"}),
            json!({"heldId":"01"}),
            json!({"heldId":"+1"}),
            json!({"heldId":null}),
            json!({"heldId":"18446744073709551616"}),
            json!({"other":true}),
        ] {
            assert!(dispatch_preflight(&params).is_err(), "{params}");
        }
        assert_eq!(held_id(&json!({})).unwrap(), None);
        assert_eq!(held_id(&json!({"heldId":"18446744073709551615"})).unwrap(), Some(u64::MAX));
    }
    #[test]
    fn explicit_discard_preserves_live_document_and_real_redo() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        session.execute("layer.new.layer", json!({})).unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let redo: Vec<_> = session.active().unwrap().history.redo_labels().map(str::to_string).collect();
        let doc = photocraft_doc::Document::new("Pending", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let held = session
            .hold_open_import(doc, false, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft), &photocraft_raster::Interrupt::NONE)
            .unwrap();
        assert!(session.execute(ID, json!({"heldId":"999999999"})).is_err());
        assert_eq!(session.held_open_count(), 1);
        assert_eq!(session.execute(ID, json!({"heldId":held.to_string()})).unwrap()["discarded"], 1);
        let state = session.active().unwrap();
        assert!(state.doc.shares_state(&before));
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), redo.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(session.held_open_count(), 0);
        session.execute("edit.redo", json!({})).unwrap();
    }
    fn hold_small(session: &mut Session) -> u64 {
        let doc = photocraft_doc::Document::new(
            "Retained unopened input",
            photocraft_doc::Size::new(1, 1),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
        );
        session.hold_open_import(doc, true, photocraft_io::ImportOrigin::xcf(), &photocraft_raster::Interrupt::NONE).unwrap()
    }
    #[test]
    fn discard_keeps_valid_and_stale_transient_preview_view_and_redo() {
        for stale in [false, true] {
            let mut session = Session::new();
            session.execute("file.new", json!({"width":3,"height":2,"background":"white"})).unwrap();
            session.execute("select.all", json!({})).unwrap();
            let layer = session.active().unwrap().active_layer.unwrap();
            session
                .edit("Real recovery redo", |doc, _| {
                    doc.layer_mut(layer).unwrap().name = "Redo layer name".into();
                    Ok(())
                })
                .unwrap();
            session.execute("edit.undo", json!({})).unwrap();
            session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
            {
                let state = session.active_mut().unwrap();
                if stale {
                    state.floating.as_mut().unwrap().revision = u64::MAX;
                }
                state.channel_view.target = crate::channel_cmds::ChannelTarget::Color(1);
                state.isolated_layers = vec![layer];
                state.color_samplers = vec![[1.25, 0.75]];
            }
            let state = session.active().unwrap();
            let doc = state.doc.clone();
            let floating = state.floating.clone().unwrap();
            let view = state.channel_view.clone();
            let revisions = (state.revision, state.saved_revision, state.view_revision);
            let selected = state.selected_layers.clone();
            let active = state.active_layer;
            let past = state.history.entry_labels().map(str::to_string).collect::<Vec<_>>();
            let redo = state.history.redo_labels().map(str::to_string).collect::<Vec<_>>();
            assert!(!redo.is_empty());
            let journal = session.journal.len();
            hold_small(&mut session);
            assert_eq!(session.execute(ID, json!({})).unwrap()["discarded"], 1);
            let state = session.active().unwrap();
            assert!(state.doc.shares_state(&doc));
            assert_eq!((state.revision, state.saved_revision, state.view_revision), revisions);
            assert_eq!(state.channel_view, view);
            assert_eq!(state.active_layer, active);
            assert_eq!(state.selected_layers, selected);
            assert_eq!(state.isolated_layers, vec![layer]);
            assert_eq!(state.color_samplers, vec![[1.25, 0.75]]);
            assert_eq!(state.history.entry_labels().collect::<Vec<_>>(), past.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), redo.iter().map(String::as_str).collect::<Vec<_>>());
            let now = state.floating.as_ref().unwrap();
            assert_eq!((now.layer, now.offset, now.revision), (floating.layer, floating.offset, floating.revision));
            assert!(std::sync::Arc::ptr_eq(&now.parts, &floating.parts));
            assert_eq!(session.journal.len(), journal);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().name, "Redo layer name");
        }
    }
    #[test]
    fn discard_without_any_open_document_does_not_create_tab_or_journal() {
        let mut session = Session::new();
        let held = hold_small(&mut session);
        assert!(session.documents().is_empty());
        assert!(session.active().is_none());
        let journal = session.journal.len();
        assert_eq!(session.execute(ID, json!({"heldId":held.to_string()})).unwrap()["discarded"], 1);
        assert!(session.documents().is_empty());
        assert!(session.active().is_none());
        assert_eq!(session.held_open_count(), 0);
        assert_eq!(session.journal.len(), journal);
        assert_eq!(session.execute(ID, json!({})).unwrap()["discarded"], 0);
    }
    #[test]
    fn authorization_refusal_keeps_unopened_source_and_session() {
        fn deny(command: &str, _: &Value) -> Result<()> {
            if command == ID { Err(EngineError::Other("own pending-open authorization denial".into())) } else { Ok(()) }
        }
        let mut session = Session::new();
        let held = hold_small(&mut session);
        let (doc, read_only, origin) = session.held_open_with_origin(held).unwrap();
        let original = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
        assert!(read_only);
        assert_eq!(origin, Some(photocraft_io::ImportOrigin::xcf()));
        session.authorize = Some(deny);
        let error = session.execute(ID, json!({"heldId":held.to_string()})).unwrap_err();
        assert!(error.to_string().contains("own pending-open authorization denial"));
        let (doc, read_only, origin) = session.held_open_with_origin(held).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(doc, &Default::default()).unwrap(), original);
        assert!(read_only);
        assert_eq!(origin, Some(photocraft_io::ImportOrigin::xcf()));
        assert_eq!(session.held_open_count(), 1);
        assert!(session.documents().is_empty());
        assert!(session.active().is_none());
        assert!(session.journal.is_empty());
        session.authorize = None;
        assert_eq!(session.execute(ID, json!({})).unwrap()["discarded"], 1);
    }
}
