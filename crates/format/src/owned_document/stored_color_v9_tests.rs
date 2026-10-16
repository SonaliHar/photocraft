//! UNRUN owned archive versioning/custody regressions; no semantic filter grant.
use super::*;
use photocraft_doc::{Color, ColorMode, SampleEncoding, SampleType, XcfState, item_identity::*, xcf_filters::*, xcf_state::ItemState};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaKind, QuotaLease, QuotaLimits},
};
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn raw() -> FilterArgument {
    FilterArgument {
        name: "raw".into(),
        value: FilterValue::StoredColor(ExactColorStorage {
            layout: "HSL float".into(),
            samples: vec![0, 0, 0, 128, 0, 0, 128, 63, 0, 0, 0, 0],
            opaque_tail: vec![0x5a; 257],
        }),
    }
}
fn fixture(q: &BackingQuota, arguments: Vec<FilterArgument>) -> (Document, QuotaLease) {
    let guard = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut doc = Document::with_background("owned version fixture", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let layer = &doc.layers[0];
    let owner = ItemRef::Layer(layer.id);
    let mut state = XcfState::default();
    state.items.push(ItemState {
        item: owner,
        name: layer.name.clone(),
        custom_name: false,
        visible: layer.visible,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(Rect::new(0, 0, 1, 1)),
        composition: None,
        mask: None,
    });
    state.filter_stacks.push(FilterStack {
        owner,
        revision: 7,
        nodes: vec![FilterNode {
            id: FilterId(91),
            operation: OperationIdentity { name: "own:inert-storage".into(), stored_version: "0:0".into(), schema_epoch: "own-storage-only".into() },
            source_display_name: None,
            source_icon: None,
            arguments,
            enabled: false,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    doc.xcf_state = Some(state);
    doc.validate_xcf_state_with(&Interrupt::NONE).unwrap();
    (doc, guard)
}
fn nested(kind: u8) -> FilterArgument {
    match kind {
        0 => raw(),
        1 => FilterArgument { name: "configuration".into(), value: FilterValue::Config(vec![raw()]) },
        _ => FilterArgument {
            name: "graph".into(),
            value: FilterValue::Graph(FilterGraph {
                input: None,
                nodes: vec![GraphNode {
                    id: GraphNodeId(77),
                    operation: OperationIdentity { name: "own:graph-storage".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
                    version_origin: GraphVersionOrigin::Stored,
                    arguments: vec![raw()],
                }],
                edges: vec![],
                output: GraphNodeId(77),
                original_pipeline: None,
            }),
        },
    }
}
struct Writer {
    cursor: Cursor<Vec<u8>>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl Writer {
    fn new(q: &BackingQuota) -> Self {
        let max = 16usize << 20;
        let header = (std::mem::size_of::<Self>() + std::mem::size_of::<Memory>() + 256) as u64;
        let mut fee = q.reserve(QuotaKind::Resident, max as u64 + header).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(max).unwrap();
        assert!(bytes.capacity() <= max);
        fee.reconcile_capacity(bytes.capacity() as u64 + header).unwrap();
        Self { cursor: Cursor::new(bytes), quota: q.clone(), _fee: fee }
    }
    fn finish(self) -> Arc<Memory> {
        Arc::new(Memory { bytes: self.cursor.into_inner(), quota: self.quota, _fee: self._fee })
    }
}
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.cursor.position().checked_add(b.len() as u64).is_none_or(|n| n > self.cursor.get_ref().capacity() as u64) {
            return Err(std::io::Error::other("paid output capacity"));
        }
        self.cursor.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for Writer {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(p)
    }
}
impl ImmutableCanonicalArchive for Memory {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if self.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let a = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let b = a.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(a..b).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}

fn manifest(memory: &Memory) -> crate::Manifest {
    let mut reader = crate::zip::stream::Reader::new(Cursor::new(memory.bytes.as_slice()), Default::default(), &Interrupt::NONE).unwrap();
    serde_json::from_slice(&reader.read_by_name("manifest.json", 1 << 20, &Interrupt::NONE).unwrap()).unwrap()
}
fn save(doc: &Document, q: &BackingQuota) -> Arc<Memory> {
    save_current_document_archive(doc, Writer::new(q), Default::default(), q, &ctl()).unwrap().finish()
}
fn lowered_header(memory: &Memory, q: &BackingQuota) -> Arc<Memory> {
    // Source/output/manifest copies belong to this external harness; never claimed
    // as a production codec receipt. The returned writer has a distinct paid fee.
    let _harness = q.reserve(QuotaKind::Resident, 32 << 20).unwrap();
    let mut input = crate::zip::stream::Reader::new(Cursor::new(memory.bytes.as_slice()), Default::default(), &Interrupt::NONE).unwrap();
    let original = input.read_by_name("manifest.json", 1 << 20, &Interrupt::NONE).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["format_version"] = 8.into();
    value["minimum_reader_version"] = 8.into();
    fn collect(value: &serde_json::Value, hashes: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::String(s) if s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()) => {
                hashes.insert(s.clone());
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect(value, hashes)
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    collect(value, hashes)
                }
            }
            _ => {}
        }
    }
    let mut hashes = std::collections::BTreeSet::new();
    collect(&value, &mut hashes);
    let changed = serde_json::to_vec(&value).unwrap();
    let mut writer = crate::zip::stream::Writer::new_accounted(Writer::new(q), Default::default(), &ctl(), q).unwrap();
    writer.add_accounted("manifest.json", changed.len() as u64, &mut changed.as_slice(), &ctl()).unwrap();
    for hash in hashes {
        for prefix in ["blobs", "tiles"] {
            let path = format!("{prefix}/{hash}.zst");
            if input.contains(&path) {
                let bytes = input.read_by_name(&path, 16 << 20, &Interrupt::NONE).unwrap();
                writer.add_accounted(&path, bytes.len() as u64, &mut bytes.as_slice(), &ctl()).unwrap();
            }
        }
    }
    writer.finish_accounted(&ctl()).unwrap().finish()
}
#[test]
#[ignore = "UNRUN: same-quota owned writer selects reader9 for exact StoredColor in direct/Config/Graph data and preserves ordinary Color/Number minimums"]
fn owned_writer_selects_nine_and_retains_exact_stored_bits() {
    for kind in 0..3 {
        let q = quota();
        let _harness = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
        let expected = nested(kind);
        let (doc, _model) = fixture(&q, vec![expected.clone()]);
        let memory = save(&doc, &q);
        let header = manifest(&memory);
        assert_eq!((header.format_version, header.minimum_reader_version), (9, 9));
        let loaded = load_document_from_owned_archive(memory, &Default::default(), Default::default(), q.clone(), &ctl()).unwrap();
        let kept = &loaded.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0];
        assert_eq!(kept, &expected);
    }
    for kind in 0..2 {
        let q = quota();
        let _harness = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
        let value = if kind == 0 {
            FilterValue::Number(ExactNumber::Float32Bits((-0f32).to_bits()))
        } else {
            FilterValue::Color { components: vec![ExactNumber::Float32Bits(0f32.to_bits()); 4], encoding: SampleEncoding::NonlinearProfile, profile: None }
        };
        let (doc, _model) = fixture(&q, vec![FilterArgument { name: "old".into(), value }]);
        assert!(!requires_reader_v9_after_validation(&doc, &q, &ctl()).unwrap());
        let header = manifest(&save(&doc, &q));
        assert_eq!((header.format_version, header.minimum_reader_version), (2, 2));
    }
}
#[test]
#[ignore = "UNRUN: valid-hash owned archives with StoredColor reject underdeclared minimum8 after decode, while declared9 retains bytes"]
fn owned_loader_rejects_underdeclared_stored_color() {
    for kind in 0..3 {
        let q = quota();
        let _harness = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
        let (doc, _model) = fixture(&q, vec![nested(kind)]);
        let source = save(&doc, &q);
        let lowered = lowered_header(&source, &q);
        let before = q.usage().unwrap();
        let result = load_document_from_owned_archive(lowered, &Default::default(), Default::default(), q.clone(), &ctl());
        assert!(matches!(result,Err(FormatError::Corrupt(ref reason))if reason=="raw stored color requires minimum reader version 9"));
        // Lowered archive owner was consumed/dropped; source/model remain held.
        assert!(q.usage().unwrap().resident <= before.resident);
        let declared = load_document_from_owned_archive(source, &Default::default(), Default::default(), q.clone(), &ctl()).unwrap();
        assert_eq!(
            declared.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments,
            doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments
        );
    }
}
#[test]
#[ignore = "UNRUN: postvalidated owned v9 scan prepays original work, admits32KiB scratch, checks late failures and leaves generic limits unchanged"]
fn shared_control_and_phase_scratch_are_effective() {
    let q = quota();
    // Caller prehold precedes nested argument Vecs and later external clones.
    let _harness = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let (doc, _model) = fixture(&q, vec![nested(2)]);
    let before = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let debit = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    assert!(requires_reader_v9_after_validation(&doc, &q, &c).unwrap());
    let once = used.load(Ordering::Relaxed);
    assert!(once > 1_600_000);
    assert_eq!(q.usage().unwrap(), before);
    requires_reader_v9_after_validation(&doc, &q, &c).unwrap();
    assert_eq!(used.load(Ordering::Relaxed), once * 2);
    let denied = |n| if n > 1 { Err(BackingError::Work) } else { Ok(()) };
    let failure = requires_reader_v9_after_validation(&doc, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &denied });
    assert!(matches!(failure,Err(FormatError::Io(ref e))if e.get_ref().and_then(|e|e.downcast_ref::<BackingError>())==Some(&BackingError::Work)));
    assert_eq!(q.usage().unwrap(), before);
    let calls = AtomicUsize::new(0);
    let late = |_| {
        if calls.fetch_add(1, Ordering::Relaxed) + 1 == 3 { Err(BackingError::Work) } else { Ok(()) }
    };
    assert!(requires_reader_v9_after_validation(&doc, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &late }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let poll = || true;
    assert!(matches!(
        requires_reader_v9_after_validation(&doc, &q, &ReadControl { interrupt: &Interrupt::cancel_only(&poll), work: &|_| Ok(()) }),
        Err(FormatError::Cancelled)
    ));
    assert_eq!(q.usage().unwrap(), before);
    let room = (256u64 << 20) - q.usage().unwrap().resident;
    let block = q.reserve(QuotaKind::Resident, room - (32 << 10) + 1).unwrap();
    assert!(requires_reader_v9_after_validation(&doc, &q, &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), before);
    let (mut empty, _emptyfee) = fixture(&q, vec![]);
    empty.xcf_state.as_mut().unwrap().filter_stacks.clear();
    let emptyused = AtomicU64::new(0);
    let emptydebit = |n| {
        emptyused.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    assert!(!requires_reader_v9_after_validation(&empty, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &emptydebit }).unwrap());
    assert_eq!(emptyused.load(Ordering::Relaxed), 1);
    // Direct upstream malformed-depth coverage remains its own admission domain.
    let mut deep = raw();
    for _ in 0..33 {
        deep = FilterArgument { name: "nested".into(), value: FilterValue::Config(vec![deep]) };
    }
    let mut invalid = doc.clone();
    invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments = vec![deep];
    assert!(crate::convert::requires_reader_v9_state_with(&invalid, Interrupt::NONE).is_err());
}
