//! Trusted native codec transfer of an already charged selection Surface.
use super::*;

impl AccountedDocument {
    /// Consume a codec-owned model and its actual archive/model retention guard.
    /// As with `from_owned`, the guard must own the model's existing physical
    /// allocations on `quota`. A loaded anonymous selection is additionally
    /// validated and moved into a real selection owner; only its new owner and
    /// document view allocations are charged. No native bytes are re-encoded.
    /// The initially supported selection lane is the existing plain-raster
    /// model-copy subset. Unsupported selections fail without being discarded.
    pub fn from_owned_rehydrating_selection(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            selection: Option<AccountedSelection>,
            view: Option<SelectionViewCharges>,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let mut incoming = Incoming { document, selection: None, view: None, retention };
        if incoming.document.selection.is_none() {
            let Incoming { document, retention, .. } = incoming;
            return Self::from_owned(document, retention, quota, control);
        }
        validate_backing(&incoming.document, &quota, control)?;
        imported_selection::validate(&incoming.document, &quota, control)?;
        let format = incoming.document.selection.as_ref().ok_or(BackingError::Integrity)?.format();
        if !selection_format_allowed(format, incoming.document.depth) {
            return Err(BackingError::Unsupported);
        }
        let canvas = incoming.document.bounds();
        let surface = incoming.document.selection.take().ok_or(BackingError::Integrity)?;
        let selection = AccountedSelection::adopt_codec_surface(surface, canvas, incoming.retention.clone(), &quota, control)?;
        // Keep the adopted owner in Incoming before any fallible view creation.
        incoming.selection = Some(selection);
        let SelectionView { surface, charges } = incoming.selection.as_ref().ok_or(BackingError::Integrity)?.clone_view(control)?;
        incoming.document.selection = Some(surface);
        incoming.view = Some(charges);
        selection_census(&incoming.document, true, incoming.selection.as_ref(), &quota, control)?;
        let allocation = quota.reserve(QuotaKind::Index, (std::mem::size_of::<ModelOwner>() + 128) as u64).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { document, selection, view, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::Legacy,
                document,
                _selection: selection,
                _selection_view: view,
                retention,
                quota,
                _model: None,
                _allocation: Arc::new(allocation),
            }),
        })
    }
}

#[cfg(test)]
#[path = "accounted_selection_rehydrate_tests.rs"]
mod tests;
