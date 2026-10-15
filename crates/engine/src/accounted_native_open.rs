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
    CurrentHistogramThreshold,
    CurrentPoint,
    CurrentAnalyticGeometry,
}

/// Allocation-free route hint selecting the stricter current Point endpoint.
/// Version, arguments, profile and storage still require the typed handoff;
/// a refusal must never retry through the generic archive endpoint.
pub fn is_current_point_candidate(document: &photocraft_doc::Document, control: &ReadControl<'_>) -> std::result::Result<bool, BackingError> {
    if let Some(state) = &document.xcf_state {
        // A retained empty stack has no operation name. Select the same strict
        // current endpoint; only its complete storage/profile/native recipe
        // can admit it. This never changes effectless XCF source selection.
        if let [stack] = state.filter_stacks.as_slice() && stack.nodes.is_empty() {
            control.check(1)?;
            return Ok(true);
        }
        for stack in &state.filter_stacks {
            control.check(1)?;
            for node in &stack.nodes {
                control.check(1)?;
                if matches!(
                    node.operation.name.as_str(),
                    "gegl:brightness-contrast"
                        | "gegl:levels"
                        | "gegl:exposure"
                        | "gegl:color-temperature"
                        | "gegl:sepia"
                        | "gegl:alien-map"
                        | "gegl:invert-linear"
                        | "gegl:invert-gamma"
                        | "gegl:absolute"
                        | "gegl:color-exchange"
                        | "gegl:color-overlay"
                        | "gegl:channel-mixer"
                        | "gegl:mono-mixer"
                        | "gegl:red-eye-removal"
                        | "gegl:color-rotate"
                        | "gegl:video-degradation"
                        | "gegl:color-to-alpha"
                        | "gegl:component-extract"
                        | "gegl:alpha-clip"
                        | "gimp:desaturate"
                        | "gegl:saturation"
                        | "gegl:value-invert"
                        | "gimp:posterize"
                        | "gimp:color-balance"
                        | "gimp:levels"
                        | "gimp:curves"
                        | "gegl:rgb-clip"
                        | "gegl:unpremultiply"
                        | "gegl:box-blur"
                        | "gegl:median-blur"
                        | "gegl:motion-blur-linear"
                        | "gegl:pixelize"
                        | "gegl:gaussian-blur"
                ) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Exact retained geometry epochs select the stricter current endpoint only.
/// No class, source/profile authority or locator access follows from this hint;
/// even malformed matching metadata must undergo the full typed validation.
pub fn is_current_analytic_geometry_candidate(document: &photocraft_doc::Document, control: &ReadControl<'_>) -> std::result::Result<bool, BackingError> {
    use photocraft_doc::import_compositing::{
        XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
        XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
    };
    if let Some(state) = &document.xcf_state {
        for item in &state.items {
            control.check(1)?;
            if let Some(policy) = &item.composition {
                control.check(policy.compatibility_epoch.len() as u64)?;
                if matches!(
                    policy.compatibility_epoch.as_str(),
                    XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
                        | XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
                        | XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH
                        | XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH
                ) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn point_error(error: photocraft_compose::backed_normal::BackedNormalError, control: &ReadControl<'_>) -> EngineError {
    use photocraft_compose::backed_normal::BackedNormalError;
    use photocraft_raster::checked_view::RasterViewError;
    match error {
        BackedNormalError::Backing(error)
        | BackedNormalError::View(RasterViewError::Backed(error) | RasterViewError::Control(error) | RasterViewError::Descriptor(error)) => error.into(),
        BackedNormalError::Quota(_) => BackingError::Quota.into(),
        BackedNormalError::Allocation => BackingError::Allocation.into(),
        _ if control.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("owned stock Point current recipe: {other}")),
    }
}

/// Independently qualify and visit the complete current operative drawable,
/// using its genuine model quota and the caller's cumulative control.
pub(crate) fn validate_current_stock_point_storage(owner: &photocraft_doc::accounted_document::AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    validate_current_owned_filter_storage(owner, control)
}

/// Shared current-filter recipe validation. The exact neighborhood name hint
/// selects a stricter semantic context only; no failure retries another family.
/// Both contexts retain the real model quota and visit the full operative input.
pub(crate) fn validate_current_owned_filter_storage(owner: &photocraft_doc::accounted_document::AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    control.check(1)?;
    let profile = match owner.document().mode {
        photocraft_doc::ColorMode::Rgb => photocraft_cms::Builtin::Srgb.profile(),
        photocraft_doc::ColorMode::Grayscale => photocraft_cms::Builtin::SGray.profile(),
        _ => return Err(BackingError::Unsupported.into()),
    };
    control.check(1)?;
    // Nonlinear builtin basis only. The context independently binds and owns
    // the actual stored-encoding effective current profile on the same quota.
    if photocraft_compose::backed_neighborhood_context::is_candidate(owner.document(), control)? {
        let prepared = photocraft_compose::backed_neighborhood_context::PreparedBackedNeighborhoodContext::new(owner, profile, Default::default(), control)
            .map_err(|error| point_error(error, control))?;
        prepared.validate_operative(control).map_err(|error| point_error(error, control))?;
        drop(prepared);
    } else {
        let prepared = photocraft_compose::backed_stock_point_context::PreparedBackedStockPointContext::new(owner, profile, Default::default(), control)
            .map_err(|error| point_error(error, control))?;
        prepared.validate_operative(control).map_err(|error| point_error(error, control))?;
        drop(prepared);
    }
    control.check(1)?;
    Ok(())
}

/// Stored operation names select a stricter current-storage validator only.
/// This hint grants no runtime storage class, original-source proof or read
/// access. Typed Session handoff must still validate; refusal must not retry.
pub fn is_current_threshold_alpha_candidate(document: &photocraft_doc::Document, control: &ReadControl<'_>) -> std::result::Result<bool, BackingError> {
    if let Some(state) = &document.xcf_state {
        for stack in &state.filter_stacks {
            control.check(1)?;
            for node in &stack.nodes {
                control.check(1)?;
                if node.operation.name == "gimp:threshold-alpha" {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Select the separate strict Histogram Threshold handoff, without granting
/// storage, profile or source eligibility. A refusal is final for this route.
pub fn is_current_threshold_candidate(document: &photocraft_doc::Document, control: &ReadControl<'_>) -> std::result::Result<bool, BackingError> {
    if let Some(state) = &document.xcf_state {
        for stack in &state.filter_stacks {
            control.check(1)?;
            for node in &stack.nodes {
                control.check(1)?;
                if node.operation.name == "gimp:threshold" {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Validate the entire current operative recipe before any Session publication.
/// This does not establish original-source correspondence or retain a render grant.
pub(crate) fn validate_current_threshold_storage(owner: &photocraft_doc::accounted_document::AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    use photocraft_compose::{backed_normal::BackedNormalError, backed_threshold::BackedThresholdError};
    use photocraft_raster::checked_view::RasterViewError;
    control.check(1)?;
    let profile = photocraft_cms::Builtin::Srgb.profile();
    control.check(1)?;
    let prepared =
        photocraft_compose::backed_threshold_context::PreparedBackedThresholdContext::new(owner, profile, Default::default(), control).map_err(|error| {
            match error {
                BackedNormalError::Backing(error)
                | BackedNormalError::View(RasterViewError::Backed(error) | RasterViewError::Control(error) | RasterViewError::Descriptor(error)) => {
                    error.into()
                }
                BackedNormalError::Quota(_) => BackingError::Quota.into(),
                BackedNormalError::Allocation => BackingError::Allocation.into(),
                _ if control.interrupt.cancelled() => EngineError::Cancelled,
                other => EngineError::Other(format!("owned Histogram Threshold current recipe: {other}")),
            }
        })?;
    prepared.validate_operative(control).map_err(|error| match error {
        BackedThresholdError::Backing(error)
        | BackedThresholdError::View(RasterViewError::Backed(error) | RasterViewError::Control(error) | RasterViewError::Descriptor(error)) => error.into(),
        _ if control.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("owned Histogram Threshold operative band: {other}")),
    })?;
    drop(prepared);
    control.check(1)?;
    Ok(())
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

    /// Explicit Histogram Threshold native continuation. Saved locators remain
    /// inert and the current recipe must pass on its actual owned storage.
    pub fn add_owned_native_threshold_archive(
        &mut self,
        archive: OwnedDocumentArchive,
        label: &str,
        authorized_native_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        self.add_owned_native_with(archive, label, authorized_native_path, control, NativeHandoff::CurrentHistogramThreshold)
    }

    /// Strict current Point native continuation. Stored source locators remain
    /// inert; the full current recipe must succeed before publishing any state.
    pub fn add_owned_native_point_archive(
        &mut self,
        archive: OwnedDocumentArchive,
        label: &str,
        authorized_native_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        self.add_owned_native_with(archive, label, authorized_native_path, control, NativeHandoff::CurrentPoint)
    }

    /// Strict assigned-ICC analytic masked geometry continuation. Native current
    /// storage is genuinely retained; profile and full canvas validation precede
    /// history/view/tab publication. Saved XCF locators remain inert evidence.
    pub fn add_owned_native_analytic_geometry_archive(
        &mut self,
        archive: OwnedDocumentArchive,
        label: &str,
        authorized_native_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        self.add_owned_native_with(archive, label, authorized_native_path, control, NativeHandoff::CurrentAnalyticGeometry)
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
        if matches!(handoff, NativeHandoff::Document)
            && (is_current_point_candidate(archive.document(), control)? || is_current_analytic_geometry_candidate(archive.document(), control)?)
        {
            return Err(BackingError::Unsupported.into());
        }
        let document = match handoff {
            NativeHandoff::Document | NativeHandoff::CurrentAnalyticGeometry => archive.into_accounted_document(control),
            NativeHandoff::CurrentThresholdAlpha => archive.into_accounted_readonly_threshold_alpha_storage(control),
            NativeHandoff::CurrentHistogramThreshold => archive.into_accounted_readonly_threshold_storage(control),
            NativeHandoff::CurrentPoint => archive.into_accounted_readonly_point_storage(control),
        }
        .map_err(|error| {
            if matches!(handoff, NativeHandoff::CurrentPoint | NativeHandoff::CurrentAnalyticGeometry) {
                if let photocraft_format::FormatError::Io(io) = &error {
                    if let Some(backing) = io.get_ref().and_then(|source| source.downcast_ref::<BackingError>()) {
                        return EngineError::from(*backing);
                    }
                }
            }
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
            let prepared =
                photocraft_compose::backed_threshold_alpha_context::PreparedBackedThresholdAlphaContext::new(&document, profile, Default::default(), control)
                    .map_err(|error| match error {
                    photocraft_compose::backed_normal::BackedNormalError::Backing(error) => EngineError::from(error),
                    _ if control.interrupt.cancelled() => EngineError::Cancelled,
                    other => EngineError::Other(format!("owned native current Threshold Alpha: {other}")),
                })?;
            // Validate the complete supported canvas in bounded operative bands on
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
        if let NativeHandoff::CurrentHistogramThreshold = handoff {
            validate_current_threshold_storage(&document, control)?;
        }
        if let NativeHandoff::CurrentPoint = handoff {
            validate_current_stock_point_storage(&document, control)?;
        }
        if let NativeHandoff::CurrentAnalyticGeometry = handoff {
            crate::accounted_xcf_open::analytic_geometry::validate_current_analytic_geometry_storage(&document, control)?;
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

#[cfg(test)]
#[path = "accounted_native_open_hint_tests.rs"]
mod hint_tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "accounted_histogram_threshold_open_tests.rs"]
mod histogram_threshold_integration_tests;

#[cfg(test)]
#[path = "accounted_point_handoff_tests.rs"]
mod point_handoff_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_residual_point_workflow_tests.rs"]
mod residual_point_workflow_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_scalar_point_workflow_tests.rs"]
mod scalar_point_workflow_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_neighborhood_workflow_tests.rs"]
mod neighborhood_workflow_tests;

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_analytic_geometry_workflow_tests.rs"]
mod analytic_geometry_workflow_tests;
