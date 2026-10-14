//! Scoped host authority for registered exact stored-channel Load Selection.
//! The offer is runtime-only. A journal request without the original host
//! binding cannot reconstruct it, and no filtered appearance is evaluated.
use crate::xcf_channel_stored_load::{self as stored, Limits, PreparedStoredLoad};
use crate::{EngineError, Result, Session, jobs::JobCtx, xcf_channel_consumer::SourceBinding};
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const CMD: &str = "select.loadSelection";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
fn exact(value: Option<&Value>, nonzero: bool) -> Result<u64> {
    let text = value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("stored source target must use canonical decimal strings"))?;
    let number = text.parse::<u64>().map_err(|_| bad("stored source target exceeds range"))?;
    if nonzero && number == 0 {
        return Err(bad("stored source target must be nonzero"));
    }
    Ok(number)
}
struct Work {
    used: AtomicU64,
    exceeded: AtomicBool,
    limit: u64,
}
impl Work {
    fn poll(&self) -> bool {
        if self.used.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(1).filter(|next| *next <= self.limit)).is_err() {
            self.exceeded.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
    fn finish<R>(&self, result: Result<R>) -> Result<R> {
        if self.exceeded.load(Ordering::Relaxed) { Err(bad("stored publication work limit")) } else { result }
    }
}
/// Only the trusted preparation function constructs this capability. Its
/// prepared snapshot, private COW output and original JobCtx stay held through
/// registered dispatch. JobCtx's shared backing is already caller-held; its
/// cloned header/Arc and this offer are inside PUBLICATION_BYTES.
pub(crate) struct StoredLoadOffer {
    prepared: PreparedStoredLoad,
    output: Option<Surface>,
    unchanged: bool,
    response: Value,
    context: JobCtx,
    work: Work,
}
impl StoredLoadOffer {
    fn with_control<R>(&self, lo: f32, hi: f32, f: impl FnOnce(&Interrupt<'_>) -> Result<R>) -> Result<R> {
        let result = self.context.stage(lo, hi, "Load Stored Selection", |control| {
            let cancelled = || control.cancelled() || self.work.poll();
            let progress = |fraction| control.progress(fraction);
            f(&Interrupt::new(&cancelled, &progress))
        });
        self.work.finish(result)
    }
    fn check_request(&self, session: &Session, params: &Value) -> Result<()> {
        let object = params
            .as_object()
            .filter(|object| {
                object.len() == 5
                    && object.keys().all(|key| matches!(key.as_str(), "channelId" | "documentId" | "expectedRevision" | "rawStorage" | "storedSource"))
            })
            .ok_or_else(|| bad("stored source offer requires its exact scoped request"))?;
        if object.get("rawStorage").and_then(Value::as_bool) != Some(true)
            || object.get("storedSource").and_then(Value::as_bool) != Some(true)
            || exact(object.get("channelId"), true)? != self.prepared.source_channel().0
            || exact(object.get("documentId"), true)? != self.prepared.snapshot().id.0
            || exact(object.get("expectedRevision"), false)? != self.prepared.revision()
            || !self.prepared.publication_reserved()
        {
            return Err(bad("stored source offer does not match this request"));
        }
        self.with_control(0.9, 0.95, |control| self.prepared.validate(session, control))
    }
}
pub(crate) fn preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    if background || session.command_is_background() {
        return Err(bad("stored source publication requires synchronous registered dispatch"));
    }
    session.stored_load_offer.as_ref().ok_or_else(|| bad("stored source requires a fresh host-held original binding"))?.check_request(session, params)
}

/// Prepare the genuine original binding under the same owned cancellation
/// context, offer it once, then execute the existing registered command. This
/// never reads a locator or accepts caller-forged proof fields.
pub fn load_stored_selection_with_binding(session: &mut Session, binding: &SourceBinding<'_>, limits: Limits, context: &JobCtx) -> Result<Value> {
    if session.stored_load_offer.is_some() {
        return Err(bad("another stored selection offer is active"));
    }
    let prepared =
        context.stage(0.0, 0.7, "Prepare Stored Selection", |control| stored::prepare_for_publication_with_limits(session, binding, limits, control))?;
    let used = prepared.requirements().work_units;
    if used > limits.work_units {
        return Err(bad("stored preparation exhausted publication work"));
    }
    let mut offer = StoredLoadOffer {
        prepared,
        output: None,
        unchanged: false,
        response: json!({"selected":true}),
        context: context.clone(),
        work: Work { used: AtomicU64::new(used), exceeded: AtomicBool::new(false), limit: limits.work_units },
    };
    let (output, unchanged) = offer.with_control(0.7, 0.9, |control| {
        offer.prepared.validate(session, control)?;
        let source = offer.prepared.stored_surface()?;
        // The closed stored reader proves <= one complete native tile. Identity
        // conversion preserves raw words/defaults and shares each original Arc.
        if source.tile_count() > 1 {
            return Err(bad("stored output exceeds its admitted map"));
        }
        let output = source
            .try_convert_with(source.format(), control)
            .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(&error.to_string()) })?;
        let unchanged = offer.prepared.source_bounds() == offer.prepared.previous_selection_bounds()
            && offer.prepared.snapshot().selection.as_ref().is_some_and(|selection| selection.tile_count() == output.tile_count());
        let unchanged = if unchanged {
            let selection = offer.prepared.snapshot().selection.as_ref().ok_or_else(|| bad("stored selection disappeared"))?;
            super::super::selection_storage_controlled::equal_storage(selection, &output, control)?
        } else {
            false
        };
        control.check().map_err(|_| EngineError::Cancelled)?;
        Ok((output, unchanged))
    })?;
    offer.output = Some(output);
    offer.unchanged = unchanged;
    let params = json!({"channelId":offer.prepared.source_channel().0.to_string(), "documentId":offer.prepared.snapshot().id.0.to_string(), "expectedRevision":offer.prepared.revision().to_string(), "rawStorage":true,"storedSource":true});
    context.check()?;
    session.stored_load_offer = Some(offer);
    let result = session.execute(CMD, params);
    // Shared dispatch also clears the slot on every attempt. Clearing here
    // covers early authorization/dispatch failures without retaining a grant.
    session.stored_load_offer = None;
    result
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    preflight(session, params, false)?;
    let mut offer = session.stored_load_offer.take().ok_or_else(|| bad("stored source offer was consumed"))?;
    let output = offer.output.take().ok_or_else(|| bad("stored source offer has no prepared output"))?;
    let response = std::mem::take(&mut offer.response);
    offer.with_control(0.95, 1.0, |control| {
        offer.prepared.validate(session, control)?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        if offer.unchanged {
            return Ok(response);
        }
        let edit = |doc: &mut photocraft_doc::Document, _: &mut Option<photocraft_doc::LayerId>| {
            offer.prepared.apply_selection_bounds(doc, control)?;
            doc.selection = Some(output);
            control.check().map_err(|_| EngineError::Cancelled)?;
            Ok(())
        };
        if let Some(limits) = offer.prepared.resident_history_limits() {
            session.edit_with_controlled_resident_history_with_interrupt("Load Stored Selection", limits, control, edit)?;
        } else {
            session.edit_with_interrupt("Load Stored Selection", control, edit)?;
        }
        // No cancellation or other fallible operation after publication.
        Ok(response)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_without_original_host_offer_never_grants_stored_access() {
        let session = Session::new();
        for params in [json!({"storedSource":true}), json!({"channelId":"1","documentId":"1","expectedRevision":"0","rawStorage":true,"storedSource":true})] {
            assert!(preflight(&session, &params, false).is_err());
        }
    }
    #[test]
    fn exact_target_strings_and_prepaid_work_do_not_wrap() {
        for value in [json!(1), json!("01"), json!("18446744073709551616"), json!("-1")] {
            assert!(exact(Some(&value), true).is_err());
        }
        assert_eq!(exact(Some(&json!("0")), false).unwrap(), 0);
        let work = Work { used: AtomicU64::new(u64::MAX), exceeded: AtomicBool::new(false), limit: u64::MAX };
        assert!(work.poll());
        assert!(work.finish(Ok(())).is_err());
    }
    fn transaction_session() -> (Session, photocraft_doc::item_identity::ChannelId) {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_doc::{
            AlphaChannel, Document, Size,
            item_identity::{ChannelId, ItemRef},
        };
        use photocraft_geom::Rect;
        let mut doc = Document::new("Own stored publication", Size::new(32, 32), ColorMode::Rgb, SampleType::F64);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut surface = Surface::try_with_default_bytes(format, &(-0f64).to_ne_bytes()).unwrap();
        surface.try_write_interleaved(Rect::new(0, 0, 1, 1), &f64::from_bits(0.5f64.to_bits() + 1).to_ne_bytes()).unwrap();
        let mut channel = AlphaChannel::new("Own saved source", surface);
        channel.id = ChannelId(19);
        doc.channels.push(channel);
        doc.selection = Some(Surface::try_with_default_bytes(format, &0f64.to_ne_bytes()).unwrap());
        doc.selection_channel_id = Some(ChannelId(17));
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        for id in [ChannelId(17), ChannelId(19)] {
            state.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().raster_bounds = Some(Rect::new(0, 0, 32, 32));
        }
        doc.xcf_state = Some(state);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, ChannelId(19))
    }
    #[test]
    fn admitted_byte_budgeted_resident_history_restores_exact_selection() {
        let (mut session, source) = transaction_session();
        session.active_mut().unwrap().history.max_bytes = 1 << 20;
        assert!(session.active().unwrap().history.plain_clone_census_work().is_err());
        let before = session.active().unwrap().doc.selection.as_ref().unwrap().clone();
        let output = session.active().unwrap().doc.channels.iter().find(|channel| channel.id == source).unwrap().surface.clone();
        let expected = output.clone();
        session
            .edit_with_controlled_resident_history_with_interrupt(
                "Load Stored Selection",
                photocraft_ops::ResidentHistoryLimits::default(),
                &Interrupt::NONE,
                |document, _| {
                    document.selection = Some(output);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(session.active().unwrap().history.max_bytes, 1 << 20);
        assert!(session.undo());
        assert!(
            super::super::super::selection_storage_controlled::equal_storage(
                session.active().unwrap().doc.selection.as_ref().unwrap(),
                &before,
                &Interrupt::NONE,
            )
            .unwrap()
        );
        assert!(session.redo());
        assert!(
            super::super::super::selection_storage_controlled::equal_storage(
                session.active().unwrap().doc.selection.as_ref().unwrap(),
                &expected,
                &Interrupt::NONE,
            )
            .unwrap()
        );
    }
    #[test]
    fn cancelled_controlled_history_transaction_keeps_selection_and_history() {
        let (mut session, _) = transaction_session();
        session.active_mut().unwrap().history.max_bytes = 1 << 20;
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let cancel = AtomicBool::new(false);
        let check = || cancel.load(Ordering::Relaxed);
        let control = Interrupt::cancel_only(&check);
        let result = session.edit_with_controlled_resident_history_with_interrupt(
            "Load Stored Selection",
            photocraft_ops::ResidentHistoryLimits::default(),
            &control,
            |document, _| {
                document.selection = None;
                cancel.store(true, Ordering::Relaxed);
                Ok(())
            },
        );
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(!session.active().unwrap().history.can_undo());
    }
    fn offer_after_binding(session: &Session, source: photocraft_doc::item_identity::ChannelId, ctx: JobCtx) -> (StoredLoadOffer, Value) {
        // This isolated draft verifies registered publication AFTER the genuine
        // source-binding boundary, not acceptance of a synthetic original XCF.
        let prepared = stored::draft_publication_prepared(session, source, photocraft_geom::Rect::new(0, 0, 32, 32));
        let output = prepared.stored_surface().unwrap().try_convert_with(prepared.stored_surface().unwrap().format(), &Interrupt::NONE).unwrap();
        let unchanged =
            super::super::super::selection_storage_controlled::equal_storage(prepared.snapshot().selection.as_ref().unwrap(), &output, &Interrupt::NONE)
                .unwrap();
        let params = json!({"channelId":source.0.to_string(),"documentId":prepared.snapshot().id.0.to_string(),"expectedRevision":prepared.revision().to_string(),"rawStorage":true,"storedSource":true});
        (
            StoredLoadOffer {
                prepared,
                output: Some(output),
                unchanged,
                response: json!({"selected":true}),
                context: ctx,
                work: Work { used: AtomicU64::new(0), exceeded: AtomicBool::new(false), limit: Limits::default().work_units },
            },
            params,
        )
    }
    #[test]
    fn registered_post_binding_load_preserves_raw_source_and_one_undo_noop_redo() {
        let (mut session, source) = transaction_session();
        let original = session.active().unwrap().doc.clone();
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.stored_load_offer = Some(offer);
        session.execute(CMD, params).unwrap();
        assert!(session.stored_load_offer.is_none());
        let after = session.active().unwrap().doc.clone();
        assert_eq!(after.channels, original.channels);
        assert_eq!(after.xcf_state, original.xcf_state);
        assert_eq!(after.selection.as_ref().unwrap().default_bytes(), &(-0f64).to_ne_bytes());
        assert_eq!(
            after.selection.as_ref().unwrap().try_read_interleaved(photocraft_geom::Rect::new(0, 0, 1, 1)).unwrap(),
            f64::from_bits(0.5f64.to_bits() + 1).to_ne_bytes()
        );
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc.selection, original.selection);
        assert!(session.redo());
        session
            .edit("Owned redo seed", |doc, _| {
                doc.name.push('!');
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let revision = session.active().unwrap().revision;
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.stored_load_offer = Some(offer);
        session.execute(CMD, params).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
    }
    #[test]
    fn registered_post_binding_cancel_stale_and_background_refusals_keep_storage() {
        let (mut session, source) = transaction_session();
        let original = session.active().unwrap().doc.clone();
        let ctx = JobCtx::new();
        let (offer, params) = offer_after_binding(&session, source, ctx.clone());
        ctx.cancel();
        session.stored_load_offer = Some(offer);
        assert!(session.execute(CMD, params).is_err());
        assert_eq!(session.active().unwrap().doc, original);
        assert!(session.stored_load_offer.is_none());
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.stored_load_offer = Some(offer);
        assert!(preflight(&session, &params, true).is_err());
        session.stored_load_offer = None;
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session
            .edit("Change captured target", |doc, _| {
                doc.name.push('!');
                Ok(())
            })
            .unwrap();
        let changed = session.active().unwrap().doc.clone();
        session.stored_load_offer = Some(offer);
        assert!(session.execute(CMD, params).is_err());
        assert_eq!(session.active().unwrap().doc, changed);
        assert!(session.stored_load_offer.is_none());
    }
    #[test]
    fn denied_authorization_and_stale_channel_view_consume_offer_without_history() {
        let (mut session, source) = transaction_session();
        let original = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.stored_load_offer = Some(offer);
        session.authorize = Some(|_, _| Err(EngineError::Other("Owned denied host policy".into())));
        assert!(session.execute(CMD, params).is_err());
        assert!(session.stored_load_offer.is_none());
        assert_eq!(session.active().unwrap().doc, original);
        assert_eq!(session.active().unwrap().history.past_len(), history);
        session.authorize = None;
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.active_mut().unwrap().channel_view.quick_mask_hidden = true;
        let changed_view = session.active().unwrap().channel_view.clone();
        session.stored_load_offer = Some(offer);
        assert!(session.execute(CMD, params).is_err());
        assert!(session.stored_load_offer.is_none());
        assert_eq!(session.active().unwrap().channel_view, changed_view);
        assert_eq!(session.active().unwrap().doc, original);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
    #[test]
    fn changed_history_policy_refuses_before_private_clone_or_publication() {
        let (mut session, source) = transaction_session();
        let original = session.active().unwrap().doc.clone();
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.active_mut().unwrap().history.max_bytes = 1;
        session.stored_load_offer = Some(offer);
        let error = session.execute(CMD, params).unwrap_err();
        assert!(error.to_string().contains("controlled history publication"));
        assert!(session.stored_load_offer.is_none());
        assert_eq!(session.active().unwrap().doc, original);
        session.active_mut().unwrap().history.max_bytes = 0;
        let (offer, params) = offer_after_binding(&session, source, JobCtx::new());
        session.active_mut().unwrap().history.max_states += 1;
        session.stored_load_offer = Some(offer);
        assert!(session.execute(CMD, params).is_err());
        assert!(session.stored_load_offer.is_none());
        assert_eq!(session.active().unwrap().doc, original);
    }
}
