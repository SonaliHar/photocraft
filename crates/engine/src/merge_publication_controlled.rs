//! Atomic all-source replacement for already-prepared native channel assembly.
use crate::{DocState, DocumentSnapshot, EngineError, Result, Session};
use photocraft_raster::Interrupt;
use serde_json::{Map, Value};
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn bad(message: &str) -> EngineError {
    EngineError::Other(format!("channel.merge: {message}"))
}
impl Session {
    /// The output, tab vector and response are prepared before the final fence.
    /// Cancellation is deferred after publication begins; no sequential close/add.
    pub(crate) fn publish_merge_batch_with_interrupt(
        &mut self,
        anchor: &DocumentSnapshot,
        anchor_revision: u64,
        sources: &[(DocumentSnapshot, u64)],
        mut output: DocState,
        ctl: &Interrupt<'_>,
    ) -> Result<Value> {
        poll(ctl)?;
        if self.accounted_slots.is_some()
            || self.docs.len() > 4096
            || sources.len() < 3
            || sources.len() > 4096
            || self.coalesce_request.is_some()
            || self.color_restrict.is_some()
            || self.command_has_pending_job()
        {
            return Err(bad("batch publication context/slot limit"));
        }
        let active = self.active().ok_or(EngineError::NoDocument)?;
        if active.doc.id != anchor.id
            || active.revision != anchor_revision
            || !anchor.shares_state(&active.doc)
            || active.floating.is_some()
            || active.coalesce.is_some()
            || self.command_has_job_on(anchor.id)
        {
            return Err(bad("active document changed during preparation"));
        }
        let mut current = std::collections::HashMap::new();
        current.try_reserve(self.docs.len()).map_err(|_| bad("document index allocation"))?;
        if current.capacity() > self.docs.len().saturating_mul(4).saturating_add(64) {
            return Err(bad("document index capacity"));
        }
        for (index, state) in self.docs.iter().enumerate() {
            poll(ctl)?;
            if state.doc.id.0 == 0 || current.insert(state.doc.id, index).is_some() {
                return Err(bad("ambiguous open document identity"));
            }
        }
        let mut selected = std::collections::HashSet::new();
        selected.try_reserve(sources.len()).map_err(|_| bad("source set allocation"))?;
        if selected.capacity() > sources.len().saturating_mul(4).saturating_add(64) {
            return Err(bad("source set capacity"));
        }
        for (source, revision) in sources {
            poll(ctl)?;
            let index = *current.get(&source.id).ok_or_else(|| bad("source document closed"))?;
            let state = self.docs.get(index).ok_or_else(|| bad("source document index"))?;
            if !selected.insert(source.id)
                || state.revision != *revision
                || !source.downgrade().is_current(&state.doc)
                || !source.shares_state(&state.doc)
                || state.doc.accounted().is_some()
                || state.floating.is_some()
                || state.coalesce.is_some()
                || self.command_has_job_on(source.id)
            {
                return Err(bad("source changed during preparation"));
            }
            if self.smart_links.iter().any(|link| link.parent == source.id || link.child == source.id) {
                return Err(bad("smart contents publication unsupported"));
            }
        }
        if output.doc.id.0 == 0
            || current.contains_key(&output.doc.id)
            || output.doc.accounted().is_some()
            || output.path.is_some()
            || output.floating.is_some()
            || output.coalesce.is_some()
        {
            return Err(bad("invalid prepared assembly output"));
        }
        let kept = self.docs.len().checked_sub(selected.len()).ok_or_else(|| bad("selected source count"))?;
        let count = kept.checked_add(1).filter(|n| *n <= 4096).ok_or_else(|| bad("result tab count"))?;
        let mut slots = Vec::new();
        slots.try_reserve_exact(count).map_err(|_| bad("batch tab allocation"))?;
        if slots.capacity() > count.saturating_add(64) {
            return Err(bad("batch tab capacity"));
        }
        output.history.max_states = self.prefs.get().performance.history_states.max(1) as usize;
        output.history.max_bytes = self.prefs.get().performance.history_budget_bytes();
        let mut object = Map::new();
        object.insert("document".into(), Value::from(u64::try_from(kept).map_err(|_| bad("response index range"))?));
        let response = Value::Object(object);
        poll(ctl)?;
        // No source has a smart association, so on_close cannot perform a fallible child commit.
        // No source job remains. All subsequent operations move existing owners or use prepaid slots.
        for state in self.docs.iter().rev() {
            if selected.contains(&state.doc.id) {
                self.subject_cache.forget_doc(state.doc.id);
                self.object_finder.forget_doc(state.doc.id);
                // smart_links is unchanged: all selected identities were excluded above.
            }
        }
        let old = std::mem::take(&mut self.docs);
        for state in old {
            if !selected.contains(&state.doc.id) {
                slots.push(state);
            }
        }
        slots.push(output);
        self.docs = slots;
        self.active = Some(kept);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(name: &str) -> photocraft_doc::Document {
        photocraft_doc::Document::new(name, photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U32)
    }
    #[test]
    fn all_sources_replaced_while_unselected_tabs_keep_identity_and_order() {
        let mut session = Session::new();
        for name in ["one", "unselected", "two", "three"] {
            session.add_document(doc(name), None).unwrap();
        }
        session.set_active(0);
        let anchor = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let retained = session.documents()[1].doc.clone();
        let sources = [3usize, 0, 2].map(|index| (session.documents()[index].doc.clone(), session.documents()[index].revision));
        let output = DocState::new(doc("merged"), None).unwrap();
        let response = session.publish_merge_batch_with_interrupt(&anchor, revision, &sources, output, &Interrupt::NONE).unwrap();
        assert_eq!(response, serde_json::json!({"document":1}));
        assert_eq!(session.documents().len(), 2);
        assert!(retained.shares_state(&session.documents()[0].doc));
        assert_eq!(session.active().unwrap().doc.name, "merged");
    }
    #[test]
    fn cancellation_or_stale_member_never_closes_any_source() {
        let mut session = Session::new();
        for name in ["one", "two", "three"] {
            session.add_document(doc(name), None).unwrap();
        }
        let anchor = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let mut sources = session.documents().iter().map(|state| (state.doc.clone(), state.revision)).collect::<Vec<_>>();
        let output = DocState::new(doc("merged"), None).unwrap();
        assert!(matches!(
            session.publish_merge_batch_with_interrupt(&anchor, revision, &sources, output, &Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        sources[0].1 += 1;
        let output = DocState::new(doc("merged"), None).unwrap();
        assert!(session.publish_merge_batch_with_interrupt(&anchor, revision, &sources, output, &Interrupt::NONE).is_err());
        assert_eq!(session.documents().len(), 3);
        assert_eq!(session.active_index(), Some(2));
        assert!(anchor.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().revision, revision);
    }
}
