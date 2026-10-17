//! Actual-class anonymous pixel-selection custody. Raw Source Documents remain
//! selection-free; this module never converts a programme owner to Legacy.
use super::*;
use photocraft_raster::Surface;
use programme_native::ProgrammeNativeStorageKind as Kind;

// Private Doc-bound permission, minted only after real guard/view/ledger checks.
// Lower Doc index helpers may consume it; outside crates cannot construct one.
pub(crate) struct SelectionPermit<'a> {
    document: &'a Document,
    guard: Option<&'a AccountedSelection>,
}
impl<'a> SelectionPermit<'a> {
    pub(crate) fn matches_document(&self, doc: &Document) -> bool {
        std::ptr::eq(self.document, doc)
    }
    pub(super) fn guard(&self) -> Option<&'a AccountedSelection> {
        self.guard
    }
}
pub(crate) fn selection_disallowed(doc: &Document, permit: Option<&SelectionPermit<'_>>) -> bool {
    doc.selection.is_some() && !permit.is_some_and(|p| p.matches_document(doc))
}
fn permit<'a>(doc: &'a Document, guard: Option<&'a AccountedSelection>, q: &BackingQuota, c: &ReadControl<'_>) -> Result<SelectionPermit<'a>, BackingError> {
    c.check(1)?;
    if doc.selection_channel_id.is_some() {
        return Err(BackingError::Unsupported);
    }
    match (&doc.selection, guard) {
        (None, None) => {}
        (Some(surface), Some(guard))
            if selection_format_allowed(guard.format(), doc.depth) && doc.bounds().contains_rect(&guard.bounds()) && guard.matches(surface, q, c)? => {}
        _ => return Err(BackingError::Integrity),
    }
    Ok(SelectionPermit { document: doc, guard })
}
pub(super) fn owner_permit<'a>(owner: &'a AccountedDocument, c: &ReadControl<'_>) -> Result<SelectionPermit<'a>, BackingError> {
    programme_native::kind(owner)?;
    if owner.document().selection.is_some() != owner.owner._selection_view.is_some() {
        return Err(BackingError::Integrity);
    }
    permit(owner.document(), owner.owner._selection.as_ref(), owner.quota(), c)
}
pub(super) fn measured(doc: &Document, q: &BackingQuota, kind: Kind, p: &SelectionPermit<'_>, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    if !p.matches_document(doc) {
        return Err(BackingError::Integrity);
    }
    match kind {
        Kind::ThresholdAlpha => filter_readonly::programme_selected_measured(doc, q, p, c),
        Kind::HistogramThreshold => threshold_readonly::programme_selected_measured(doc, q, p, c),
        Kind::Point | Kind::AssignedPoint => point_readonly::programme_selected_measured(doc, q, kind == Kind::AssignedPoint, p, c),
        Kind::GaussianGroup => gaussian_group_readonly::programme_selected_measured(doc, q, p, c),
        Kind::FlatComposite => flat_composite_readonly::programme_selected_measured(doc, q, p, c),
        Kind::FilteredFlatPoint | Kind::MaskedFilteredFlatPoint => {
            filtered_flat_readonly::programme_selected_measured(doc, q, kind == Kind::MaskedFilteredFlatPoint, p, c)
        }
        Kind::NormalHierarchy => normal_hierarchy_readonly::programme_selected_measured(doc, q, p, c),
    }
}
pub(super) fn measured_owner(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    let p = owner_permit(owner, c)?;
    measured(owner.document(), owner.quota(), programme_native::kind(owner)?, &p, c)
}
/// Nonforgeable actual-owner Current witness, including full class structural
/// census. The whole programme still requires its existing semantic compiler.
pub struct CheckedProgrammeSelection<'a> {
    owner: &'a AccountedDocument,
    kind: Kind,
    _scratch: QuotaLease,
}
impl<'a> CheckedProgrammeSelection<'a> {
    pub fn owner(&self) -> &'a AccountedDocument {
        self.owner
    }
    pub fn storage_kind(&self) -> Kind {
        self.kind
    }
    pub fn matches_owner(&self, owner: &AccountedDocument) -> bool {
        self.owner.shares_model(owner)
    }
    pub fn matches_document(&self, doc: &Document) -> bool {
        std::ptr::eq(self.owner.document(), doc)
    }
    pub fn selection(&self) -> Option<&'a Surface> {
        self.owner.document().selection.as_ref()
    }
}
// A new view owns its actual map/default leases and shares only immutable paid
// tile Arcs. The caller retains the old/new guard until the resulting owner owns
// its own guard clone. View/payload drop before these charges on every failure.
pub(super) struct SelectionCopy<'a> {
    view: Option<SelectionView>,
    guard: Option<&'a AccountedSelection>,
}
impl<'a> SelectionCopy<'a> {
    pub(super) fn new(owner: &'a AccountedDocument, replacement: Option<Option<&'a AccountedSelection>>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let _old = owner_permit(owner, c)?;
        let guard = replacement.unwrap_or(owner.owner._selection.as_ref());
        if let Some(g) = guard
            && (!owner.quota().shares_ledger(g.quota())
                || !selection_format_allowed(g.format(), owner.document().depth)
                || !owner.document().bounds().contains_rect(&g.bounds()))
        {
            return Err(BackingError::Unsupported);
        }
        let view = guard.map(|g| g.clone_view(c)).transpose()?;
        Ok(Self { view, guard })
    }
    pub(super) fn install(mut self, staged: &mut ModelStaging) -> Option<&'a AccountedSelection> {
        if let Some(SelectionView { surface, charges }) = self.view.take() {
            staged.document.selection = Some(surface);
            staged.selection_view = Some(charges)
        } else {
            staged.document.selection = None;
            staged.selection_view = None
        }
        self.guard
    }
}
pub(super) fn staged_permit<'a>(
    doc: &'a Document,
    guard: Option<&'a AccountedSelection>,
    q: &BackingQuota,
    c: &ReadControl<'_>,
) -> Result<SelectionPermit<'a>, BackingError> {
    permit(doc, guard, q, c)
}
fn copy(
    owner: &AccountedDocument,
    logical: ModelCensus,
    selection: Option<&AccountedSelection>,
    c: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    match programme_native::kind(owner)? {
        Kind::ThresholdAlpha => filter_readonly::programme_selection_copy(owner, logical, selection, c),
        Kind::HistogramThreshold => threshold_readonly::programme_selection_copy(owner, logical, selection, c),
        Kind::Point | Kind::AssignedPoint => point_readonly::programme_selection_copy(owner, logical, selection, c),
        Kind::GaussianGroup => gaussian_group_readonly::programme_selection_copy(owner, logical, selection, c),
        Kind::FlatComposite => flat_composite_readonly::programme_selection_copy(owner, logical, selection, c),
        Kind::FilteredFlatPoint | Kind::MaskedFilteredFlatPoint => filtered_flat_readonly::programme_selection_copy(owner, logical, selection, c),
        Kind::NormalHierarchy => normal_hierarchy_readonly::programme_selection_copy(owner, logical, selection, c),
    }
}
impl AccountedDocument {
    pub fn checked_programme_selection(&self, c: &ReadControl<'_>) -> Result<CheckedProgrammeSelection<'_>, BackingError> {
        let kind = programme_native::kind(self)?;
        let fixed = (std::mem::size_of::<CheckedProgrammeSelection<'_>>() + 512) as u64;
        let scratch = self.quota().reserve(QuotaKind::Resident, fixed).map_err(|_| BackingError::Quota)?;
        c.check(fixed)?;
        measured_owner(self, c)?;
        Ok(CheckedProgrammeSelection { owner: self, kind, _scratch: scratch })
    }
    pub fn validate_programme_selection_replacement(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        measured_owner(self, c)?;
        c.check(1)
    }
    pub fn try_replace_programme_selection(&self, selection: Option<&AccountedSelection>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let logical = measured_owner(self, c)?;
        if let Some(g) = selection
            && (!self.quota().shares_ledger(g.quota())
                || !selection_format_allowed(g.format(), self.document().depth)
                || !self.document().bounds().contains_rect(&g.bounds()))
        {
            return Err(BackingError::Unsupported);
        }
        let noop = match (selection, &self.document().selection) {
            (None, None) => true,
            (Some(g), Some(s)) => g.matches(s, self.quota(), c)?,
            _ => false,
        };
        c.check(1)?;
        if noop {
            return Ok(self.clone());
        }
        copy(self, logical, selection, c)
    }
    /// Trusted codec-only ownership contract, like from_owned: retention must
    /// already own all incoming physical model/native/selection bytes on quota.
    /// Some selection is required; None callers retain their old typed factories.
    pub fn from_owned_programme_rehydrating_selection(
        kind: Kind,
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            selection: Option<AccountedSelection>,
            view: Option<SelectionViewCharges>,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let mut input = Incoming { document, selection: None, view: None, retention };
        if input.document.selection_channel_id.is_some() {
            return Err(BackingError::Unsupported);
        }
        let source = input.document.selection.as_ref().ok_or(BackingError::Unsupported)?;
        if !selection_format_allowed(source.format(), input.document.depth) {
            return Err(BackingError::Unsupported);
        }
        // Independent prospective scalar decode/unit validation work for the
        // actual codec adoption below; no earlier parser/Source phase credit.
        let bpp = source.format().bytes_per_pixel() as u64;
        let bytes = (source.tile_count() as u64)
            .checked_mul(65536)
            .and_then(|v| v.checked_mul(bpp))
            .and_then(|v| v.checked_add(source.default_bytes().len() as u64))
            .ok_or(BackingError::Length)?;
        if bpp == 0 {
            return Err(BackingError::Length);
        }
        let mut work = (bytes / bpp).checked_mul(96).and_then(|v| v.checked_add(bytes)).ok_or(BackingError::Length)?;
        while work != 0 {
            let n = work.min(4096);
            c.check(n)?;
            work -= n;
        }
        let surface = input.document.selection.take().ok_or(BackingError::Integrity)?;
        input.selection = Some(AccountedSelection::adopt_codec_surface(surface, input.document.bounds(), input.retention.clone(), &quota, c)?);
        let SelectionView { surface, charges } = input.selection.as_ref().ok_or(BackingError::Integrity)?.clone_view(c)?;
        input.document.selection = Some(surface);
        input.view = Some(charges);
        let p = permit(&input.document, input.selection.as_ref(), &quota, c)?;
        measured(&input.document, &quota, kind, &p, c)?;
        let allocation = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        let Incoming { document, selection, view, retention } = input;
        let class = match kind {
            Kind::ThresholdAlpha => StorageClass::ReadonlyThresholdAlpha,
            Kind::HistogramThreshold => StorageClass::ReadonlyHistogramThreshold,
            Kind::Point => StorageClass::ReadonlyPoint,
            Kind::AssignedPoint => StorageClass::ReadonlyAssignedPoint,
            Kind::GaussianGroup => StorageClass::ReadonlyGaussianGroup,
            Kind::FlatComposite => StorageClass::ReadonlyFlatComposite,
            Kind::FilteredFlatPoint => StorageClass::ReadonlyFilteredFlatPoint,
            Kind::MaskedFilteredFlatPoint => StorageClass::ReadonlyMaskedFilteredFlatPoint,
            Kind::NormalHierarchy => StorageClass::ReadonlyNormalHierarchy,
        };
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: class,
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
