//! History panel › click a state: `history.selectState` returns the document to that state in one
//! step (#2966), as Photoshop does. The panel used to run Edit › Undo / Redo once per state: each
//! bumped the document revision, so the canvas could no longer tell what the jump changed and
//! recomposited (and re-uploaded) the whole document, plus N rounds of per-command bookkeeping.
//!
//! States are numbered as the History panel lists them, like `history.deleteState`: 0 is the
//! oldest state still held, `past` (the number of undo states) is the current one, and the redo
//! states (dimmed in the panel) follow it. Jumping back leaves the skipped states redoable;
//! jumping forward redoes them. Nothing is discarded until the next edit.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, int};
use crate::{EngineError, Result, Session};

const CMD: &str = "history.selectState";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn select_state(s: &mut Session, p: &Value) -> Result<Value> {
    select_state_with_control(s, p, &crate::read_control_none())
}

fn select_state_with_control(s: &mut Session, p: &Value, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<Value> {
    if s.active().is_some_and(|state| state.doc.accounted().is_some()) {
        return Err(photocraft_raster::backing::BackingError::Unsupported.into());
    }
    let target = validate_request(s, p, control)?;
    select_target(s, target, control, None)
}

/// Borrowed target validation. Keep the existing numeric helper's rounding and
/// overflow semantics; the paid dispatcher owns its additional request envelope.
pub(crate) fn validate_request(s: &Session, p: &Value, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<usize> {
    control.check(1)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let past = st.history.past_len();
    let mut last = past;
    for _ in st.history.redo_labels() {
        control.check(1)?;
        last = last.checked_add(1).ok_or_else(|| bad("history state count overflow"))?;
    }
    let v = match p.get("state") {
        None | Some(Value::Null) => return Err(bad("missing `state` (the History entry to return to, 0 = oldest)")),
        Some(_) => int(p, "state").ok_or_else(|| bad("`state` must be a number"))?,
    };
    let target = usize::try_from(v).ok().filter(|i| *i <= last).ok_or_else(|| bad(format!("`state` = {v} is not a state (0..={last})")))?;
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't move while a command is still running on this document".into()));
    }
    Ok(target)
}

/// Only the genuine prepaid response owner can enter the accounted command body.
pub(crate) fn run_accounted(
    s: &mut Session,
    target: usize,
    control: &photocraft_raster::backing::ReadControl<'_>,
    response: &crate::accounted_commands::HistoryResponseAllowance,
) -> Result<Value> {
    control.check(1)?;
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't move while a command is still running on this document".into()));
    }
    if s.active().is_none_or(|state| state.doc.accounted().is_none()) {
        return Err(photocraft_raster::backing::BackingError::Unsupported.into());
    }
    select_target(s, target, control, Some(response))
}

fn select_target(
    s: &mut Session,
    target: usize,
    control: &photocraft_raster::backing::ReadControl<'_>,
    paid_response: Option<&crate::accounted_commands::HistoryResponseAllowance>,
) -> Result<Value> {
    control.check(1)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let past = st.history.past_len();
    if target == past {
        if let Some(owner) = st.doc.accounted() {
            return paid_response.ok_or(photocraft_raster::backing::BackingError::Unsupported)?.response(owner, target, false, None, control);
        }
        return Ok(json!({"state":target,"changed":false}));
    }
    let revision = st.revision.checked_add(1).ok_or(photocraft_raster::backing::BackingError::Length)?;
    let start = st.doc.clone();
    // Every checked step changes only this candidate. A late read/quota/target
    // failure leaves the actual cursor, redo, document and coalescing unchanged.
    let mut history = st.history.clone();
    let mut doc = start.clone();
    let mut layers = None;
    let mut at = past;
    while at != target {
        control.check(1)?;
        let change = if target < at { history.undo(&doc, control)? } else { history.redo(&doc, control)? };
        let Some(change) = change else {
            return Err(EngineError::Other(format!("the history stopped at state {at}")));
        };
        let (next, target_layers) = change.into_parts();
        doc = next;
        layers = Some(target_layers);
        at = if target < at { at.checked_sub(1) } else { at.checked_add(1) }.ok_or(photocraft_raster::backing::BackingError::Length)?;
    }
    let target_layers = layers.ok_or_else(|| EngineError::Other("the history can't step right now".into()))?;
    // Save As identity/title is outside history. Repair the private accounted
    // target through its paid copier before publishing the travelled cursor.
    let title_changed = if doc.accounted().is_some() { !same_accounted_title(&doc.name, &start.name, control)? } else { doc.name != start.name };
    if let Some(owner) = doc.accounted()
        && title_changed
    {
        let named = owner.try_with_document_name(&start.name, control)?;
        doc = history.try_replace_accounted_current(owner, named, control)?;
    } else if title_changed {
        doc.validate_resident()?;
        let limits = photocraft_io::NativeSaveLimits::default().preparation;
        photocraft_io::retained_native_model_bytes(&doc, limits, control.interrupt)
            .map_err(|e| if control.interrupt.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
        let mut name = String::new();
        name.try_reserve_exact(start.name.len()).map_err(|_| EngineError::Other("history title allocation failed".into()))?;
        control.check(start.name.len() as u64)?;
        name.push_str(&start.name);
        doc.try_resident_mut_with_raster_limit(photocraft_raster::MAX_CHECKED_RASTER_BYTES, control.interrupt)?.name = name;
    }
    let mut label = None;
    for current in history.entry_labels() {
        control.check(1)?;
        label = Some(current);
    }
    let response = if let Some(owner) = doc.accounted() {
        paid_response.ok_or(photocraft_raster::backing::BackingError::Unsupported)?.response(owner, target, true, label, control)?
    } else {
        json!({"state":target,"changed":true,"current":label})
    };
    if let Some(owner) = doc.accounted() {
        let values = target_layers.values();
        let view = crate::accounted_view::AccountedView::new(owner, values.active, &values.selected, values.active, control)?;
        control.check(1)?;
        let st = s.active_mut().ok_or(EngineError::NoDocument)?;
        st.active_layer = values.active;
        st.layer_anchor = values.active;
        st.accounted_view = Some(view);
        st.doc = doc;
        st.history = history;
        st.revision = revision;
        st.last_damage = None;
        st.coalesce = None;
        st._accounted_metadata_gesture = None;
        return Ok(response);
    }
    let (mut values, lease) = target_layers.into_parts();
    if lease.is_some() {
        return Err(photocraft_raster::backing::BackingError::Integrity.into());
    }
    let mut anchor = st.layer_anchor;
    restore_resident_target(&doc, &mut values, &mut anchor, control)?;
    let damage = crate::layer_multi_cmds::step_damage(&start, &doc);
    control.check(1)?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.doc = doc;
    st.history = history;
    st.active_layer = values.active;
    st.selected_layers = values.selected;
    st.layer_anchor = anchor;
    crate::channel_cmds::fix_view(st);
    st.revision = revision;
    st.last_damage = damage;
    st.coalesce = None;
    Ok(response)
}

fn same_accounted_title(a: &str, b: &str, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<bool> {
    control.check(1)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (left, right) in a.as_bytes().chunks(32768).zip(b.as_bytes().chunks(32768)) {
        control.check((left.len() as u64).checked_mul(2).ok_or(photocraft_raster::backing::BackingError::Length)?)?;
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
#[path = "history_select_accounted_title_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod accounted_title_tests;

/// Same target repair as checked Undo/Redo, prepared fallibly before publication.
fn restore_resident_target(
    doc: &crate::DocumentSnapshot,
    values: &mut photocraft_ops::LayerTarget,
    anchor: &mut Option<photocraft_doc::LayerId>,
    control: &photocraft_raster::backing::ReadControl<'_>,
) -> Result<()> {
    doc.validate_resident()?;
    if values.selected.len() > 100000 {
        return Err(photocraft_raster::backing::BackingError::Length.into());
    }
    let mut live = std::collections::HashSet::new();
    photocraft_doc::resident_structure::visit_layers_with(doc, control.interrupt, |layer| {
        live.try_reserve(1).map_err(|_| photocraft_raster::RasterInputError::AllocationFailed)?;
        live.insert(layer.id);
        Ok(true)
    })?;
    let deselected = values.active.is_none() && values.selected.is_empty();
    if !deselected && values.active.is_none_or(|id| !live.contains(&id)) {
        values.active = doc.top_layer();
    }
    let Some(active) = values.active else {
        values.selected.clear();
        return Ok(());
    };
    if !values.selected.contains(&active) {
        values.selected.try_reserve_exact(1).map_err(|_| EngineError::Other("history target allocation failed".into()))?;
        values.selected.clear();
        values.selected.push(active);
    } else {
        let mut selected = Vec::new();
        selected.try_reserve_exact(values.selected.len()).map_err(|_| EngineError::Other("history target allocation failed".into()))?;
        let mut seen = std::collections::HashSet::new();
        seen.try_reserve(values.selected.len()).map_err(|_| EngineError::Other("history target allocation failed".into()))?;
        for id in &values.selected {
            control.check(1)?;
            if live.contains(id) && seen.insert(*id) {
                selected.push(*id);
            }
        }
        values.selected = selected;
    }
    if anchor.is_some_and(|id| !values.selected.contains(&id)) {
        *anchor = Some(active);
    }
    control.check(1)?;
    Ok(())
}

fn can_select(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or_else(|| "no document open".to_string())?;
    if st.history.can_undo() || st.history.can_redo() { Ok(()) } else { Err("the history holds only one state".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Select History State",
        menu: &[],
        shortcut: None,
        params: r##"{"state":index} (History entry as the panel lists them: 0 = oldest, then the current state, then the redo states) → {state,changed,current?}: returns the document to that state in one step; the states after it stay redoable"##,
        enabled: can_select,
        run: select_state,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document with four steps after Open, each targeting different layers.
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s
    }

    type Look = (crate::DocumentSnapshot, Option<photocraft_doc::LayerId>, Vec<photocraft_doc::LayerId>, Vec<String>, Vec<String>);

    /// What the panel can see of a session: the document, the targeted layers, the entries and
    /// the redo states.
    fn look(s: &Session) -> Look {
        let st = s.active().unwrap();
        (
            st.doc.clone(),
            st.active_layer,
            st.selected_layers.clone(),
            st.history.entries(&crate::read_control_none()).unwrap().values().to_vec(),
            st.history.redo_labels().map(str::to_string).collect(),
        )
    }

    /// Step from the current state to `target` with Edit › Undo / Redo, note what that gives, and
    /// step back.
    fn stepped(s: &mut Session, target: usize) -> Look {
        let current = s.active().unwrap().history.past_len();
        let (there, back) = if target < current { ("edit.undo", "edit.redo") } else { ("edit.redo", "edit.undo") };
        let n = target.abs_diff(current);
        for _ in 0..n {
            s.execute(there, json!({})).unwrap();
        }
        let seen = look(s);
        for _ in 0..n {
            s.execute(back, json!({})).unwrap();
        }
        seen
    }

    fn same(a: &Look, b: &Look) {
        assert!(a.0.shares_state(&b.0), "the same document");
        assert_eq!((&a.1, &a.2, &a.3, &a.4), (&b.1, &b.2, &b.3, &b.4));
    }

    #[test]
    fn jumping_back_and_forward_matches_stepping() {
        for target in 0..=4usize {
            for from_back in [0usize, 2, 4] {
                // Start `from_back` states before the end, then jump to `target`.
                let mut s = session();
                for _ in 0..from_back {
                    s.execute("edit.undo", json!({})).unwrap();
                }
                let current = 4 - from_back;
                let expected = stepped(&mut s, target);
                let rev = s.active().unwrap().revision;
                let r = s.execute(CMD, json!({"state": target})).unwrap();
                assert_eq!(r["state"], json!(target));
                assert_eq!(r["changed"], json!(target != current));
                same(&look(&s), &expected);
                assert_eq!(s.active().unwrap().history.past_len(), target);
                let bumps = s.active().unwrap().revision - rev;
                assert_eq!(bumps, u64::from(target != current), "{from_back} → {target}: one revision for the jump");
            }
        }
    }

    #[test]
    fn a_jump_reports_only_the_pixels_it_changed() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let past = s.active().unwrap().history.past_len();
        s.execute(CMD, json!({"state": past - 4})).unwrap();
        let damage = s.active().unwrap().last_damage.expect("a bounded damage rect, not a full recomposite");
        assert!(damage.width() < 400 && damage.height() < 300, "{damage:?}");
        // And forward again.
        s.execute(CMD, json!({"state": past})).unwrap();
        assert!(s.active().unwrap().last_damage.is_some());
    }

    #[test]
    fn redo_states_survive_a_jump_and_go_with_the_next_edit() {
        let mut s = session();
        let all = s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec();
        s.execute(CMD, json!({"state": 1})).unwrap();
        let (_, _, _, entries, redo) = look(&s);
        assert_eq!(entries, all[..2].to_vec());
        assert_eq!(redo, all[2..].to_vec(), "the later states are dimmed, next one first");
        assert!(s.is_enabled(CMD));
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        assert!(!s.active().unwrap().history.can_redo(), "a new step discards them");
    }

    #[test]
    fn select_state_restores_the_layers_the_state_targeted() {
        let mut s = session();
        let bottom = s.active().unwrap().doc.layers.first().unwrap().id;
        // Target the bottom layer, then jump back to the first fill: its layer comes back, as
        // with Undo.
        s.select_layer(bottom).unwrap();
        let expected = stepped(&mut s, 2);
        s.select_layer(bottom).unwrap();
        s.execute(CMD, json!({"state": 2})).unwrap();
        same(&look(&s), &expected);
        assert_ne!(s.active().unwrap().active_layer, Some(bottom));
    }

    #[test]
    fn a_floating_piece_goes_back_and_the_jump_still_happens() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
        s.execute("select.float", json!({"dx": 5, "dy": 0})).unwrap();
        let all = s.active().unwrap().history.entries(&crate::read_control_none()).unwrap().values().to_vec();
        let doc = s.active().unwrap().doc.clone();
        assert!(s.active().unwrap().floating.is_some());
        s.execute(CMD, json!({"state": 1})).unwrap();
        let st = s.active().unwrap();
        assert!(st.floating.is_none(), "the piece went back, it wasn't dropped");
        assert_eq!(st.history.past_len(), 1);
        let redo: Vec<String> = st.history.redo_labels().map(str::to_string).collect();
        assert_eq!(redo, all[2..].to_vec(), "no Drop step was added");
        // Forward to the state the piece was floated on: the document as it was, without the piece.
        s.execute(CMD, json!({"state": all.len() - 1})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&doc));
    }

    #[test]
    fn select_state_refuses_bad_params_and_changes_nothing() {
        let mut s = Session::new();
        assert!(s.execute(CMD, json!({"state": 0})).is_err(), "no document");
        assert!(!s.is_enabled(CMD));
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(!s.is_enabled(CMD), "only the opening state");
        let mut s = session();
        s.execute("edit.undo", json!({})).unwrap();
        let before = look(&s);
        let rev = s.active().unwrap().revision;
        for p in [
            json!({}),
            json!({"state": null}),
            json!({"state": -1}),
            json!({"state": 5}),
            json!({"state": 99}),
            json!({"state": i64::MAX}),
            json!({"state": f64::MAX}),
            json!({"state": "x"}),
            json!({"state": "1"}),
            json!({"state": [1]}),
            json!({"state": {"i": 1}}),
            json!({"state": true}),
            json!([1]),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
            let now = look(&s);
            assert!(now.0.shares_state(&before.0) && now.3 == before.3 && now.4 == before.4, "{p} changed nothing");
        }
        assert_eq!(s.active().unwrap().revision, rev);
        // The last redo state is reachable.
        assert!(s.execute(CMD, json!({"state": 4})).is_ok());
    }
}
