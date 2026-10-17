//! Paid synthetic CURRENT owners and controlled provider debit. The injected
//! units test meter plumbing, not measured work, performance or producer proof.
#[path = "pure_point_geometry_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod current;
use photocraft_doc::{Document, LayerContent, accounted_document::AccountedDocument};
use photocraft_format::canonical_archive::ImmutableCanonicalArchive;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
pub(super) fn ctl() -> ReadControl<'static> {
    current::ctl()
}
pub(super) fn owner(index: usize, empty: bool) -> AccountedDocument {
    owner_variant(index, empty, 1)
}
pub(super) fn cold_owner(index: usize, empty: bool) -> AccountedDocument {
    owner_variant(index, empty, 0)
}
fn owner_variant(index: usize, empty: bool, variant: usize) -> AccountedDocument {
    let family = match index % 3 {
        0 => current::Family::Semi,
        1 => current::Family::Hue,
        _ => current::Family::Vibrance,
    };
    let sample = if index < 3 { photocraft_doc::SampleType::F32 } else { photocraft_doc::SampleType::F64 };
    let owner = current::fixture(family, sample, variant);
    let mut session = photocraft_engine::Session::new();
    session.add_owned_native_point_archive(current::archive(&owner), "synthetic CURRENT", None, &ctl()).unwrap();
    if empty {
        let id = session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        session.execute("xcf.filter.remove", serde_json::json!({"filterId":id})).unwrap();
    }
    session.active().unwrap().doc.accounted().unwrap().clone()
}
pub(super) fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    current::archive(owner)
}
pub(super) fn same(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(current::raw(a), current::raw(b));
    assert_eq!(a.xcf_state, b.xcf_state);
    let (LayerContent::BackedRaster(x), LayerContent::BackedRaster(y)) = (&a.layers[0].content, &b.layers[0].content) else { panic!("native") };
    assert_eq!(x.default_bytes(), y.default_bytes());
    assert_eq!(x.operative_bounds(), y.operative_bounds());
    assert_eq!(x.format(), y.format());
}
#[derive(Debug)]
pub(super) struct Trigger {
    units: AtomicU64,
    pub hits: AtomicU64,
    _fee: QuotaLease,
}
impl Trigger {
    pub fn arm(&self, units: u64) {
        self.units.store(units, Ordering::Relaxed);
    }
}
#[derive(Debug)]
struct Archive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    trigger: Arc<Trigger>,
    _payload: QuotaLease,
    _header: QuotaLease,
}
impl ImmutableCanonicalArchive for Archive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Length)?;
        let input = self.bytes.get(start..end).ok_or(BackingError::Bounds)?;
        let units = self.trigger.units.swap(0, Ordering::Relaxed);
        if units != 0 {
            self.trigger.hits.fetch_add(1, Ordering::Relaxed);
            c.check(units)?;
        }
        c.check(out.len() as u64)?;
        out.copy_from_slice(input);
        Ok(())
    }
}
pub(super) fn triggered_archive(owner: &AccountedDocument) -> (photocraft_format::owned_document::OwnedDocumentArchive, Arc<Trigger>) {
    let raw = current::encoded(owner); // External harness writer capture, not a fee receipt.
    let q = owner.quota().clone();
    let mut payload = q.reserve(QuotaKind::Resident, (raw.len() * 2) as u64).unwrap();
    let header = q.reserve(QuotaKind::Index, 4096).unwrap();
    let trigger_fee = q.reserve(QuotaKind::Index, 4096).unwrap();
    let trigger = Arc::new(Trigger { units: AtomicU64::new(0), hits: AtomicU64::new(0), _fee: trigger_fee });
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(raw.len()).unwrap();
    assert!(bytes.capacity() <= raw.len() * 2);
    payload.reconcile_capacity(bytes.capacity() as u64).unwrap();
    bytes.extend_from_slice(&raw);
    let source = Arc::new(Archive { bytes, quota: q.clone(), trigger: trigger.clone(), _payload: payload, _header: header });
    let archive = photocraft_format::owned_document::load_document_from_owned_archive(source, &Default::default(), Default::default(), q, &ctl()).unwrap();
    (archive, trigger)
}
