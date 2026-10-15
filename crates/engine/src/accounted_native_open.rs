//! Explicit owned native reopen. This endpoint preserves current interpretation;
//! the normal open service still owns preference/profile questions and conversion.
use crate::{DocState, EngineError, Result, Session, source_policy::SourceIdentity};
use photocraft_format::owned_document::OwnedDocumentArchive;
use photocraft_raster::backing::{BackingError, ReadControl};

/// Choosing this endpoint is an explicit interpretation decision, not a new
/// default open policy or authority derived from persisted source locators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountedNativeOpenPolicy {
    KeepCurrentInterpretation,
}
#[derive(Clone, Copy)]
enum NativeHandoff {
    Document,
    CurrentThresholdAlpha,
}

fn text(value: &str, c: &ReadControl<'_>) -> Result<String> {
    let mut owned = String::new();
    owned.try_reserve_exact(value.len()).map_err(|_| BackingError::Allocation)?;
    if owned.capacity() > value.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    for ch in value.chars() {
        c.check(ch.len_utf8() as u64)?;
        owned.push(ch);
    }
    Ok(owned)
}

impl Session {
    /// Consume a genuinely owned decoded native archive. The supplied path is
    /// the actual authorized native input identity, never a persisted XCF path.
    /// No filesystem access, color conversion, or locator resolution occurs.
    /// Archive content proves native format independently of filename suffix;
    /// writable destinations remain subject to normal native-save path guards.
    pub fn add_owned_native_archive(
        &mut self,
        archive: OwnedDocumentArchive,
        policy: AccountedNativeOpenPolicy,
        label: &str,
        authorized_native_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        match policy {
            AccountedNativeOpenPolicy::KeepCurrentInterpretation => {}
        }
        self.add_owned_native_with(archive, label, authorized_native_path, control, NativeHandoff::Document)
    }

    /// Explicit current Threshold Alpha reopen over the closed immutable native
    /// storage class. Current encoded ICC, implicit marker, composition and mask
    /// recipe are checked on their actual retained owner before publication.
    /// This supplies no original-XCF correspondence or restored locator grant.
    /// The ordinary owned native endpoint never selects this class itself.
    pub fn add_owned_native_threshold_alpha_archive(
        &mut self,
        archive: OwnedDocumentArchive,
        label: &str,
        authorized_native_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        self.add_owned_native_with(archive, label, authorized_native_path, control, NativeHandoff::CurrentThresholdAlpha)
    }

    fn add_owned_native_with(
        &mut self,
        archive: OwnedDocumentArchive,
        label: &str,
        authorized_native_path: Option<&str>,
        control: &ReadControl<'_>,
        handoff: NativeHandoff,
    ) -> Result<usize> {
        control.check(1)?;
        let path_len = authorized_native_path.map_or(0, str::len);
        if label.len() > 4096 || path_len > 4096 || self.docs.len() >= 4096 {
            return Err(BackingError::Length.into());
        }
        let input = label.len().checked_add(path_len).ok_or(BackingError::Length)?;
        control.check(input as u64)?;
        if label.contains('\0') || authorized_native_path.is_some_and(|path| path.contains('\0')) {
            return Err(BackingError::Length.into());
        }
        for states in self.docs.chunks(64) {
            control.check(states.len() as u64)?;
            if states.iter().any(|state| state.doc.id == archive.document().id) {
                return Err(EngineError::Other("an open document already owns this accounted identity".into()));
            }
        }
        let document = match handoff {
            NativeHandoff::Document => archive.into_accounted_document(control),
            NativeHandoff::CurrentThresholdAlpha => archive.into_accounted_readonly_threshold_alpha_storage(control),
        }.map_err(|error| {
            if matches!(error, photocraft_format::FormatError::Cancelled) {
                EngineError::Cancelled
            } else {
                EngineError::Other(format!("owned native reopen: {error}"))
            }
        })?;
        if let NativeHandoff::CurrentThresholdAlpha = handoff {
            // Builtin initialization is the existing outer-polled provider
            // boundary. No current Profile parsing, cloning, or encoding is
            // requested here: the context requires already-retained encoding
            // and compares it exactly with actual saved current ICC bytes.
            control.check(1)?;
            let profile = match document.document().mode {
                photocraft_doc::ColorMode::Rgb => photocraft_cms::Builtin::Srgb.profile(),
                photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile(),
                _ => return Err(BackingError::Unsupported.into()),
            };
            control.check(1)?;
            let prepared = photocraft_compose::backed_threshold_alpha_context::PreparedBackedThresholdAlphaContext::new(
                &document, profile, Default::default(), control,
            ).map_err(|error| match error {
                photocraft_compose::backed_normal::BackedNormalError::Backing(error) => EngineError::from(error),
                _ if control.interrupt.cancelled() => EngineError::Cancelled,
                other => EngineError::Other(format!("owned native current Threshold Alpha: {other}")),
            })?;
            // The closed canvas remains 8x6. Validate bounded operative bands on
            // the same quota/control before publishing a tab; physical backing
            // padding and original-source correspondence remain separate.
            prepared.validate_operative(control).map_err(|error| match error {
                photocraft_compose::backed_point_filter::BackedPointError::Backing(error) => EngineError::from(error),
                _ if control.interrupt.cancelled() => EngineError::Cancelled,
                other => EngineError::Other(format!("owned native current Threshold Alpha band: {other}")),
            })?;
            // This borrowed preparation is not a persistent rendering grant.
            // A future render job must prepare/pay against its own held snapshot.
            drop(prepared);
            control.check(1)?;
        }
        let history_states = self.prefs.get().performance.history_states.max(1) as usize;
        let mut state = DocState::new_accounted(document, None, history_states, control)?;
        // Inline SourceIdentity/path fields were priced by new_accounted. Pay
        // BOTH path allocations plus label before copying any incoming text.
        let metadata = state._accounted_metadata.as_mut().ok_or(BackingError::Integrity)?;
        let previous = metadata.bytes();
        let strings = path_len.checked_mul(2).and_then(|n| n.checked_add(label.len())).ok_or(BackingError::Length)?;
        let envelope = (strings as u64).checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        metadata.try_resize(previous.checked_add(envelope).ok_or(BackingError::Length)?).map_err(|_| BackingError::Quota)?;
        let label = text(label, control)?;
        let source_path = authorized_native_path.map(|p| text(p, control)).transpose()?;
        let writable_path = authorized_native_path.map(|p| text(p, control)).transpose()?;
        let actual = label
            .capacity()
            .checked_add(source_path.as_ref().map_or(0, String::capacity))
            .and_then(|n| n.checked_add(writable_path.as_ref().map_or(0, String::capacity)))
            .ok_or(BackingError::Length)?;
        metadata
            .reconcile_capacity(previous.checked_add(actual as u64).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?)
            .map_err(|_| BackingError::Quota)?;
        state.source = Some(SourceIdentity::new(label, source_path, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft)));
        state.source_read_only = false;
        state.path = writable_path;
        // XCF provenance/fingerprints remain in the genuine current document for
        // inert destination exclusion; they are never restored into source.
        control.check(1)?;
        self.publish_document_state(state, control)
    }
}

#[cfg(test)]
#[path = "accounted_native_open_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "accounted_native_threshold_alpha_open_tests.rs"]
mod threshold_alpha_tests;
