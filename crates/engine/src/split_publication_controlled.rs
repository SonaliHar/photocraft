//! Atomic publication of already-prepared native Split outputs.
use crate::{DocState, DocumentSnapshot, EngineError, Result, Session};
use photocraft_raster::Interrupt;
use serde_json::{Map, Value};
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn bad(message: &str) -> EngineError {
    EngineError::Other(format!("channel.split: {message}"))
}
impl Session {
    /// All allocation/admission precedes the final fence. Once publication starts,
    /// cancellation is deferred; there is no fallible step after closing the source.
    pub(crate) fn publish_split_batch_with_interrupt(
        &mut self,
        source: &DocumentSnapshot,
        revision: u64,
        mut outputs: Vec<DocState>,
        close_original: bool,
        ctl: &Interrupt<'_>,
    ) -> Result<Value> {
        poll(ctl)?;
        if self.accounted_slots.is_some() || self.docs.len() > 4096 || outputs.is_empty() || outputs.len() > 4096 {
            return Err(bad("resident batch publication context/slot limit"));
        }
        let source_index = self.active.ok_or(EngineError::NoDocument)?;
        let current = self.docs.get(source_index).ok_or(EngineError::NoDocument)?;
        if current.doc.id != source.id
            || current.revision != revision
            || !source.downgrade().is_current(&current.doc)
            || !source.shares_state(&current.doc)
            || current.floating.is_some()
            || current.coalesce.is_some()
            || self.coalesce_request.is_some()
            || self.color_restrict.is_some()
            || self.command_has_job_on(source.id)
            || self.command_has_pending_job()
        {
            return Err(bad("source changed while preparing split"));
        }
        if self.smart_links.iter().any(|link| link.parent == source.id || link.child == source.id) {
            return Err(bad("smart contents split publication is unsupported"));
        }
        let kept = self.docs.len().checked_sub(usize::from(close_original)).ok_or_else(|| bad("source slot count"))?;
        let count = kept.checked_add(outputs.len()).filter(|n| *n <= 4096).ok_or_else(|| bad("split output slot limit"))?;
        let mut identities = std::collections::HashSet::new();
        let identity_count = self.docs.len().checked_add(outputs.len()).ok_or_else(|| bad("identity count"))?;
        identities.try_reserve(identity_count).map_err(|_| bad("identity allocation"))?;
        if identities.capacity() > identity_count.saturating_mul(4).saturating_add(64) {
            return Err(bad("identity capacity"));
        }
        for state in &self.docs {
            poll(ctl)?;
            if state.doc.id.0 == 0 || !identities.insert(state.doc.id) {
                return Err(bad("duplicate open document identity"));
            }
        }
        for output in &mut outputs {
            poll(ctl)?;
            if output.doc.id.0 == 0
                || !identities.insert(output.doc.id)
                || output.doc.accounted().is_some()
                || output.path.is_some()
                || output.floating.is_some()
                || output.coalesce.is_some()
            {
                return Err(bad("invalid prepared split output"));
            }
            output.history.max_states = self.prefs.get().performance.history_states.max(1) as usize;
            output.history.max_bytes = self.prefs.get().performance.history_budget_bytes();
        }
        let mut slots = Vec::new();
        slots.try_reserve_exact(count).map_err(|_| bad("batch slot allocation"))?;
        let mut indices = Vec::new();
        indices.try_reserve_exact(outputs.len()).map_err(|_| bad("result slot allocation"))?;
        for offset in 0..outputs.len() {
            indices.push(Value::from(u64::try_from(kept + offset).map_err(|_| bad("response index range"))?));
        }
        if slots.capacity() > count.saturating_add(64) || indices.capacity() > outputs.len().saturating_add(64) {
            return Err(bad("batch slot capacity"));
        }
        // The bounded array is reserved fallibly above; prepare the tiny object before
        // publication too. Returning this Value after commit requires no response allocation.
        let mut object = Map::new();
        object.insert("documents".into(), Value::Array(indices));
        let response = Value::Object(object);
        poll(ctl)?;
        // The smart association exclusion makes on_close's commit path unreachable here.
        if close_original {
            crate::smart_cmds::on_close(self, source_index);
            self.subject_cache.forget_doc(source.id);
            self.object_finder.forget_doc(source.id);
        }
        let old = std::mem::take(&mut self.docs);
        let mut removed = None;
        for (index, state) in old.into_iter().enumerate() {
            if close_original && index == source_index {
                removed = Some(state);
            } else {
                slots.push(state);
            }
        }
        slots.append(&mut outputs);
        self.docs = slots;
        self.active = count.checked_sub(1);
        drop(removed);
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
    fn batch_close_and_keep_publish_all_prepared_tabs_together() {
        for close in [false, true] {
            let mut session = Session::new();
            session.add_document(doc("original"), None).unwrap();
            let source = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let outputs = vec![DocState::new(doc("first"), None).unwrap(), DocState::new(doc("second"), None).unwrap()];
            let indices = session.publish_split_batch_with_interrupt(&source, revision, outputs, close, &Interrupt::NONE).unwrap();
            assert_eq!(indices, if close { serde_json::json!({"documents":[0, 1]}) } else { serde_json::json!({"documents":[1, 2]}) });
            assert_eq!(session.documents().len(), if close { 2 } else { 3 });
            assert_eq!(session.active().unwrap().doc.name, "second");
            if !close {
                assert!(source.shares_state(&session.documents()[0].doc));
            }
        }
    }
    #[test]
    fn cancellation_stale_revision_and_collision_never_close_original() {
        let mut session = Session::new();
        session.add_document(doc("original"), None).unwrap();
        let source = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let cancelled = Interrupt::cancel_only(&|| true);
        let outputs = vec![DocState::new(doc("first"), None).unwrap()];
        assert!(matches!(session.publish_split_batch_with_interrupt(&source, revision, outputs, true, &cancelled), Err(EngineError::Cancelled)));
        let outputs = vec![DocState::new(doc("first"), None).unwrap()];
        assert!(session.publish_split_batch_with_interrupt(&source, revision + 1, outputs, true, &Interrupt::NONE).is_err());
        let mut collision = doc("collision");
        collision.id = source.id;
        assert!(session.publish_split_batch_with_interrupt(&source, revision, vec![DocState::new(collision, None).unwrap()], true, &Interrupt::NONE).is_err());
        assert_eq!(session.documents().len(), 1);
        assert_eq!(session.active_index(), Some(0));
        assert!(source.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().revision, revision);
    }
}
