//! Immutable Session/job snapshots. Accounted snapshots never expose a detached
//! raw Arc<Document>; their clone and weak upgrade retain the physical model guard.
use photocraft_doc::{
    Document,
    accounted_document::{AccountedDocument, AccountedEdit, WeakAccountedDocument},
};
use photocraft_raster::backing::{BackingError, ReadControl, quota::BackingQuota};
use std::{
    ops::Deref,
    sync::{Arc, Weak},
};

#[path = "document_snapshot_accounting.rs"]
mod accounting;

/// Raster-only conservative footprint, not all Document metadata/blob storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidentRasterFootprint {
    pub retained_bytes: u64,
    pub clone_metadata_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    inner: Snapshot,
}
#[derive(Clone, Debug)]
enum Snapshot {
    Resident(Arc<Document>),
    Accounted(AccountedDocument),
}
impl DocumentSnapshot {
    /// Opt-in checked resident adoption. The original constructor's behavior is
    /// unchanged; callers choose a lower-only raster cap for their operation.
    pub fn from_resident_with_raster_limit(document: Arc<Document>, limit: u64, control: &photocraft_raster::Interrupt<'_>) -> Result<Self, BackingError> {
        accounting::resident(&document, limit.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES), control)?;
        Self::from_resident_with_interrupt(document, control)
    }

    /// Includes hidden/disabled raster caches and defaults, without sample reads.
    /// Accounted snapshots retain their own physical ledger and are not detached.
    pub fn resident_raster_footprint_with(&self, control: &photocraft_raster::Interrupt<'_>) -> Result<ResidentRasterFootprint, BackingError> {
        let doc = self.resident_arc().ok_or(BackingError::Unsupported)?;
        Self::resident_raster_footprint_of(doc, control)
    }

    /// Same bounded raster census for a private resident document during editing.
    /// This does not adopt it or account non-raster metadata and blobs.
    pub fn resident_raster_footprint_of(doc: &Document, control: &photocraft_raster::Interrupt<'_>) -> Result<ResidentRasterFootprint, BackingError> {
        let size = accounting::resident(doc, photocraft_raster::MAX_CHECKED_RASTER_BYTES, control)?;
        Ok(ResidentRasterFootprint { retained_bytes: size.retained, clone_metadata_bytes: size.clone_metadata })
    }

    /// Preflight the current raster retention plus COW sparse-map/default clone
    /// cost before Arc::make_mut. Document strings/blobs/other collections are
    /// outside this raster-only guard and need separate aggregate admission.
    pub fn try_resident_mut_with_raster_limit(&mut self, limit: u64, control: &photocraft_raster::Interrupt<'_>) -> Result<&mut Document, BackingError> {
        let doc = self.resident_arc().ok_or(BackingError::Unsupported)?;
        let limit = limit.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES);
        let size = accounting::resident(doc, limit, control)?;
        // Always reserve the possible clone: concurrent weak upgrades must not
        // invalidate a strong-count observation made before Arc::make_mut.
        let extra = size.clone_metadata;
        if size.retained.checked_add(extra).ok_or(BackingError::Quota)? > limit {
            return Err(BackingError::Quota);
        }
        control.check().map_err(|_| BackingError::Cancelled)?;
        self.try_resident_mut()
    }

    pub fn from_resident(document: Arc<Document>) -> Result<Self, BackingError> {
        Self::from_resident_with_interrupt(document, &photocraft_raster::Interrupt::NONE)
    }
    /// Controlled structural adoption only; no sample validation or ambient source access.
    pub fn from_resident_with_interrupt(document: Arc<Document>, control: &photocraft_raster::Interrupt<'_>) -> Result<Self, BackingError> {
        control.check().map_err(|_| BackingError::Cancelled)?;
        let backed = photocraft_doc::resident_structure::has_backed_rasters_with(&document, control)
            .map_err(|error| if error == photocraft_raster::RasterInputError::Cancelled { BackingError::Cancelled } else { BackingError::Bounds })?;
        if backed {
            return Err(BackingError::Unsupported);
        }
        control.check().map_err(|_| BackingError::Cancelled)?;
        Ok(Self { inner: Snapshot::Resident(document) })
    }
    pub fn from_accounted(document: AccountedDocument) -> Result<Self, BackingError> {
        Ok(Self { inner: Snapshot::Accounted(document) })
    }
    pub fn resident_arc(&self) -> Option<&Arc<Document>> {
        match &self.inner {
            Snapshot::Resident(doc) => Some(doc),
            Snapshot::Accounted(_) => None,
        }
    }
    pub fn accounted(&self) -> Option<&AccountedDocument> {
        match &self.inner {
            Snapshot::Accounted(doc) => Some(doc),
            Snapshot::Resident(_) => None,
        }
    }
    pub fn quota(&self) -> Option<&BackingQuota> {
        self.accounted().map(AccountedDocument::quota)
    }
    /// Legacy mutable operations remain resident-only. Publication must validate
    /// the resulting resident model again; arbitrary callbacks cannot be metered.
    pub fn try_resident_mut(&mut self) -> Result<&mut Document, BackingError> {
        match &mut self.inner {
            Snapshot::Resident(doc) => {
                if doc.has_backed_rasters().map_err(|_| BackingError::Bounds)? {
                    return Err(BackingError::Unsupported);
                }
                Ok(Arc::make_mut(doc))
            }
            Snapshot::Accounted(_) => Err(BackingError::Unsupported),
        }
    }
    pub fn validate_resident(&self) -> Result<(), BackingError> {
        match &self.inner {
            Snapshot::Resident(doc) if !doc.has_backed_rasters().map_err(|_| BackingError::Bounds)? => Ok(()),
            _ => Err(BackingError::Unsupported),
        }
    }
    pub fn try_accounted_edit(&self, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        let doc = self.accounted().ok_or(BackingError::Unsupported)?;
        Self::from_accounted(doc.try_edit(edit, control)?)
    }
    pub fn shares_state(&self, other: &Self) -> bool {
        match (&self.inner, &other.inner) {
            (Snapshot::Resident(a), Snapshot::Resident(b)) => Arc::ptr_eq(a, b),
            (Snapshot::Accounted(a), Snapshot::Accounted(b)) => a.shares_model(b),
            _ => false,
        }
    }
    pub fn downgrade(&self) -> WeakDocumentSnapshot {
        WeakDocumentSnapshot {
            inner: match &self.inner {
                Snapshot::Resident(doc) => WeakSnapshot::Resident(Arc::downgrade(doc)),
                Snapshot::Accounted(doc) => WeakSnapshot::Accounted(doc.downgrade()),
            },
        }
    }
}
/// Structural comparison of immutable current models, matching resident Arc semantics.
/// BackedLayer equality compares verified shared state and never performs native IO.
impl PartialEq for DocumentSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
impl Deref for DocumentSnapshot {
    type Target = Document;
    fn deref(&self) -> &Document {
        match &self.inner {
            Snapshot::Resident(doc) => doc,
            Snapshot::Accounted(doc) => doc.document(),
        }
    }
}
impl AsRef<Document> for DocumentSnapshot {
    fn as_ref(&self) -> &Document {
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct WeakDocumentSnapshot {
    inner: WeakSnapshot,
}
#[derive(Clone, Debug, Default)]
enum WeakSnapshot {
    #[default]
    Empty,
    Resident(Weak<Document>),
    Accounted(WeakAccountedDocument),
}
impl WeakDocumentSnapshot {
    pub fn new() -> Self {
        Self::default()
    }
    /// Weak keeps only the charged control block for accounted owners, never
    /// the native archive/model payload. Upgrade retains the entire live guard.
    pub fn upgrade(&self) -> Option<DocumentSnapshot> {
        let inner = match &self.inner {
            WeakSnapshot::Empty => return None,
            WeakSnapshot::Resident(doc) => Snapshot::Resident(doc.upgrade()?),
            WeakSnapshot::Accounted(doc) => Snapshot::Accounted(doc.upgrade()?),
        };
        Some(DocumentSnapshot { inner })
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.inner, &other.inner) {
            (WeakSnapshot::Empty, WeakSnapshot::Empty) => true,
            (WeakSnapshot::Resident(a), WeakSnapshot::Resident(b)) => Weak::ptr_eq(a, b),
            (WeakSnapshot::Accounted(a), WeakSnapshot::Accounted(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
    pub fn is_current(&self, document: &DocumentSnapshot) -> bool {
        self.ptr_eq(&document.downgrade())
    }
}
