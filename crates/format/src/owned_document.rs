//! Optional owned archive codec. This does not replace the standard loader. Successful model/object envelopes remain charged
//! until this borrowed-document owner is dropped. No deep Clone/extraction.
use crate::{
    FormatError, LoadOptions, Result,
    canonical_archive::ImmutableCanonicalArchive,
    convert::{Fetch, Loader},
};
use photocraft_doc::Document;
use photocraft_raster::backing::{BackingError, ReadControl, quota::BackingQuota};
use std::{cell::RefCell, collections::HashMap, sync::Arc};
mod accounting;
mod decode;
mod fetch;
mod finite_catalog;
mod save;
mod source;
use accounting::RetainedReservations;
pub use save::save_current_document_archive;

#[derive(Clone, Copy, Debug)]
pub struct OwnedDocumentLimits {
    pub zip: crate::zip::stream::Limits,
    pub max_manifest_bytes: usize,
    pub max_object_bytes: usize,
    pub max_layers: usize,
    pub max_reservations: usize,
    pub decoder_window: u64,
}
impl Default for OwnedDocumentLimits {
    fn default() -> Self {
        Self {
            zip: crate::zip::stream::Limits::default(),
            max_manifest_bytes: 1 << 20,
            max_object_bytes: 16 << 20,
            max_layers: 4096,
            max_reservations: 4096,
            decoder_window: 4 << 20,
        }
    }
}
impl OwnedDocumentLimits {
    /// Check the supported owned-codec profile before preparing input storage.
    /// This performs no resource admission and grants no source or IO authority.
    pub fn validate(self) -> Result<()> {
        let max = Self::default();
        if self.max_manifest_bytes > max.max_manifest_bytes
            || self.max_object_bytes > max.max_object_bytes
            || self.max_layers > max.max_layers
            || self.decoder_window == 0
            || self.decoder_window > 16 << 20
            || self.max_reservations == 0
            || self.max_reservations > 4096
        {
            return Err(FormatError::LimitExceeded("owned document profile".into()));
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct OwnedDocumentArchive {
    document: Document,
    _archive: Arc<dyn ImmutableCanonicalArchive>,
    _model: RetainedReservations,
}
// This is a stack-only choice. Neither native manifest data nor an archive
// loader selects the immutable storage class on the caller's behalf.
enum AccountedHandoff {
    Document,
    ReadonlyThresholdAlphaStorage,
    ReadonlyHistogramThresholdStorage,
    ReadonlyPointStorage,
    ReadonlyGaussianGroupStorage,
}
impl OwnedDocumentArchive {
    /// The actual ledger retaining this archive's model and encoded storage.
    /// Borrow it to prepay related handoff metadata on the same owner; equal
    /// limits or a separately constructed quota do not establish that identity.
    pub fn quota(&self) -> &BackingQuota {
        self._model.quota()
    }

    /// Move the native model and every retained archive/model lease into the
    /// application lifecycle owner. No model clone or unaccounted extraction.
    pub fn into_accounted_document(self, control: &ReadControl<'_>) -> Result<photocraft_doc::accounted_document::AccountedDocument> {
        self.into_accounted_with(control, AccountedHandoff::Document)
    }

    /// Explicit consuming handoff of the closed immutable Threshold Alpha
    /// storage class. Retains the actual archive/model/native leases on their
    /// original quota; no whole-document clone or sample conversion occurs.
    ///
    /// This validates current storage only. The caller must establish any source,
    /// profile, rendering or application recipe separately; persisted epochs and
    /// locators restore none of those authorities. Saved selection is refused by
    /// the closed document constructor. The normal archive handoff is unchanged
    /// and never automatically selects this endpoint from serialized fields.
    pub fn into_accounted_readonly_threshold_alpha_storage(self, control: &ReadControl<'_>) -> Result<photocraft_doc::accounted_document::AccountedDocument> {
        self.into_accounted_with(control, AccountedHandoff::ReadonlyThresholdAlphaStorage)
    }

    /// Explicit current Histogram Threshold storage handoff. No serialized flag,
    /// locator, profile or original-source/render authority is restored.
    pub fn into_accounted_readonly_threshold_storage(self, control: &ReadControl<'_>) -> Result<photocraft_doc::accounted_document::AccountedDocument> {
        self.into_accounted_with(control, AccountedHandoff::ReadonlyHistogramThresholdStorage)
    }

    /// Consume into structural Point custody on the actual archive/model quota.
    /// This confers no operation, profile, source or rendering eligibility; the
    /// application must validate its complete current recipe before publication.
    pub fn into_accounted_readonly_point_storage(self, control: &ReadControl<'_>) -> Result<photocraft_doc::accounted_document::AccountedDocument> {
        self.into_accounted_with(control, AccountedHandoff::ReadonlyPointStorage)
    }

    /// Explicit consuming custody for the closed Gaussian group. The actual
    /// archive/model/native leases remain on their original ledger; source and
    /// complete current profile/render admission belong to the caller.
    pub fn into_accounted_readonly_gaussian_group_storage(self, control: &ReadControl<'_>) -> Result<photocraft_doc::accounted_document::AccountedDocument> {
        self.into_accounted_with(control, AccountedHandoff::ReadonlyGaussianGroupStorage)
    }

    fn into_accounted_with(self, control: &ReadControl<'_>, handoff: AccountedHandoff) -> Result<photocraft_doc::accounted_document::AccountedDocument> {
        struct NativeRetention {
            _archive: Arc<dyn ImmutableCanonicalArchive>,
            _model: RetainedReservations,
            _owner: photocraft_raster::backing::quota::QuotaLease,
        }
        let quota = self._model.quota().clone();
        control.check(0).map_err(error)?;
        let bytes = (std::mem::size_of::<NativeRetention>() as u64).checked_add(64).ok_or_else(|| error(BackingError::Length))?;
        let lease = quota.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, bytes).map_err(|_| error(BackingError::Quota))?;
        // All fallible preparation precedes destruction of the original wrapper,
        // whose field order guarantees model payload destruction before refunds.
        control.check(0).map_err(error)?;
        let Self { document, _archive, _model } = self;
        let retention = Arc::new(NativeRetention { _archive, _model, _owner: lease });
        match handoff {
            AccountedHandoff::Document => photocraft_doc::accounted_document::AccountedDocument::from_owned_rehydrating_selection(document, retention, quota, control),
            AccountedHandoff::ReadonlyThresholdAlphaStorage => photocraft_doc::accounted_document::AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, retention, quota, control),
            AccountedHandoff::ReadonlyHistogramThresholdStorage => photocraft_doc::accounted_document::AccountedDocument::new_readonly_threshold_storage(document, retention, quota, control),
            AccountedHandoff::ReadonlyPointStorage => photocraft_doc::accounted_document::AccountedDocument::new_readonly_point_storage(document, retention, quota, control),
            AccountedHandoff::ReadonlyGaussianGroupStorage => photocraft_doc::accounted_document::AccountedDocument::from_owned_readonly_gaussian_group(document, retention, quota, control),
        }.map_err(error)
    }

    pub fn document(&self) -> &Document {
        &self.document
    }
}

#[cfg(test)]
#[path = "owned_document/threshold_alpha_storage_tests.rs"]
mod threshold_alpha_storage_tests;
#[cfg(test)]
#[path = "owned_document/threshold_storage_tests.rs"]
mod threshold_storage_tests;
#[cfg(test)]
#[path = "owned_document/translated_storage_tests.rs"]
mod translated_storage_tests;
pub(crate) fn error(error: BackingError) -> FormatError {
    if error == BackingError::Cancelled { FormatError::Cancelled } else { FormatError::Io(std::io::Error::other(error)) }
}

/// Optional whole resident document load through the existing field mapper.
/// Ordinary compressed tile/blob lookup remains supported under explicit caps.
/// FiniteRaster uses exact finite current backing; ordinary objects retain the existing mapper.
pub fn load_document_from_owned_archive(
    archive: Arc<dyn ImmutableCanonicalArchive>,
    options: &LoadOptions,
    limits: OwnedDocumentLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedDocumentArchive> {
    limits.validate()?;
    control.check(0).map_err(error)?;
    archive.validate_quota(&quota).map_err(error)?;
    let ownership = RefCell::new(RetainedReservations::new(limits.max_reservations, quota.clone(), control).map_err(error)?);
    let source = source::OwnedArchiveSource::new(archive.clone(), quota.clone(), limits.zip, limits.max_object_bytes, control, &ownership)?;
    let options = LoadOptions {
        max_manifest_bytes: options.max_manifest_bytes.min(limits.max_manifest_bytes),
        max_blob_bytes: options.max_blob_bytes.min(limits.max_object_bytes),
        ..*options
    };
    let manifest = crate::store::read_manifest(&source, &options)?;
    fn layers(values: &[crate::manifest::LayerM], depth: usize, control: &ReadControl<'_>) -> Result<usize> {
        if depth > crate::MAX_GROUP_DEPTH {
            return Err(FormatError::LimitExceeded("owned document layer depth".into()));
        }
        let mut total = values.len();
        for layer in values {
            control.check(1).map_err(error)?;
            if let crate::manifest::ContentM::Group { children, .. } = &layer.content {
                total = total.checked_add(layers(children, depth + 1, control)?).ok_or_else(|| FormatError::LimitExceeded("owned layer count".into()))?;
            }
        }
        Ok(total)
    }
    let count = layers(&manifest.document.layers, 0, control)?;
    if count > limits.max_layers {
        return Err(FormatError::LimitExceeded("owned document layer count".into()));
    }
    let mut fetch = fetch::OwnedFetch::new(archive.clone(), &source, &ownership, quota.clone(), control, options, limits)?;
    let maps = count
        .checked_add(manifest.document.channels.len())
        .and_then(|n| n.checked_add(manifest.document.paths.len()))
        .and_then(|n| n.checked_mul(512))
        .ok_or_else(|| FormatError::LimitExceeded("owned identity index".into()))?;
    fetch.model_reserve(maps as u64)?;
    let mut loader = Loader {
        fetch: &mut fetch,
        preserve_ids: options.preserve_ids,
        max_id: 0,
        id_map: HashMap::new(),
        channel_map: HashMap::new(),
        path_map: HashMap::new(),
        contents_id_map: HashMap::new(),
        legacy_linked_contents: HashMap::new(),
        tiles: HashMap::new(),
    };
    let document = loader.document_with_interrupt(&manifest.document, control.interrupt)?;
    let max_id = loader.max_id;
    drop(loader);
    if crate::convert::requires_reader_v3_state(&document) && manifest.minimum_reader_version < 3 {
        return Err(FormatError::Corrupt("required source/working state requires minimum reader version 3".into()));
    }
    if crate::convert::requires_reader_v4_state_with(&document, *control.interrupt)? && manifest.minimum_reader_version < 4 {
        return Err(FormatError::Corrupt("operative text placement requires minimum reader version 4".into()));
    }
    if crate::convert::requires_reader_v6_state_with(&document, *control.interrupt)? && manifest.minimum_reader_version < 6 {
        return Err(FormatError::Corrupt("current resource resolution requires minimum reader version 6".into()));
    }
    if crate::convert::requires_reader_v8_state_with(&document, *control.interrupt)? && manifest.minimum_reader_version < 8 {
        return Err(FormatError::Corrupt("Brown layer label requires minimum reader version 8".into()));
    }
    if crate::convert::requires_reader_v7_state_with(&document, *control.interrupt)? && manifest.minimum_reader_version < 7 {
        return Err(FormatError::Corrupt("current linked geometry requires minimum reader version 7".into()));
    }
    if options.preserve_ids && !crate::convert::reserve_ids_through(max_id) {
        drop(document);
        drop(fetch);
        drop(source);
        drop(ownership);
        return load_document_from_owned_archive(archive, &LoadOptions { preserve_ids: false, ..options }, limits, quota, control);
    }
    drop(fetch);
    drop(source);
    drop(manifest);
    let candidate = OwnedDocumentArchive { document, _archive: archive, _model: ownership.into_inner() };
    control.check(0).map_err(error)?;
    Ok(candidate)
}
