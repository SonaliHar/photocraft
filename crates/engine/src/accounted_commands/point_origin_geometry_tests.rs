//! UNRUN synthetic larger CURRENT continuation, not larger producer evidence.
//! Input/archive/expected Vecs are external harness captures. Real model/native
//! headers, cache tiles and cloned metadata are held under the same quota.
use super::*;
use photocraft_doc::{BackedLayer, LayerContent, Size, xcf_filters::FilterNode};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

const INPUTS: [[(&str, &str); 2]; 3] = [
    [
        (
            "temp/sol1-semi-flatten-gimp-20261016/rgb-f32-author-v1/outputs/rgb-f32-background.source.xcf",
            "e6d705f52c19ac30c4e3d5f92eefe22ee6baab791ce42fdb879397cb140c61da",
        ),
        (
            "temp/sol1-semi-flatten-gimp-20261016/rgb-f64-author-v1/outputs/rgb-f64-background.source.xcf",
            "63278093e370c2e7d0bcea850c1fa3ef494f6975b91746ea71594d7b9bec5b71",
        ),
    ],
    [
        (
            "temp/sol1-hue-chroma-owned-gimp-20261016/f32-author-v1/outputs/f32-neutral-to-source.source.xcf",
            "0f63702cdd76521303d869ca8ac952103c75a6abf69961555201dc635c615edf",
        ),
        (
            "temp/sol1-hue-chroma-owned-gimp-20261016/f64-author-v1/outputs/f64-neutral-to-source.source.xcf",
            "3514e59d95223345ec4dc73d0c53374af13e25a92ff6b4277a97284528b574f9",
        ),
    ],
    [
        (
            "temp/sol1-vibrance-owned-gimp-20261016/f32-author-v1/outputs/f32-neutral-rounding.source.xcf",
            "4e239a92719be5b66fb040b9d815295e5fb2cb9744ff55c8cc18980e80ec311c",
        ),
        (
            "temp/sol1-vibrance-owned-gimp-20261016/f64-author-v1/outputs/f64-neutral-rounding.source.xcf",
            "120c3e4bf2bb0655b9af3637f6bd9eccc5ca272bf6a960ce86e86a22489fc156",
        ),
    ],
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct GeneratedStore {
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _header: QuotaLease,
}
impl NativePlaneStore for GeneratedStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> std::result::Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|end| end > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        let word = self.descriptor.format().sample.bytes();
        for (i, byte) in out.iter_mut().enumerate() {
            if i % 512 == 0 {
                c.check(0)?;
            }
            let at = offset + i as u64;
            let scalar = at / word as u64;
            let pixel = scalar / 4;
            let component = scalar % 4;
            let value = if component == 3 { [0.0, 0.5, 1.0][pixel as usize % 3] } else { ((pixel + component * 17) % 127) as f64 / 128.0 };
            *byte = if word == 4 { (value as f32).to_ne_bytes()[at as usize % word] } else { value.to_ne_bytes()[at as usize % word] };
        }
        Ok(())
    }
}
struct FixtureRetention {
    _original: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
fn fixture_owner(family: usize, depth: usize, size: Size) -> AccountedDocument {
    let (path, hash) = INPUTS[family][depth];
    let bytes = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)).unwrap();
    assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), hash);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let photocraft_io::OwnedXcfImport::Point(source) = photocraft_io::import_owned_xcf_from_reader_in_quota(
        "Pinned original",
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap() else {
        panic!("Point source")
    };
    let mut original = Session::new();
    original.add_owned_point_xcf_import(&source, "Pinned original", None, &ctl()).unwrap();
    let old = original.active().unwrap().doc.accounted().unwrap().clone();
    // Before clone/map growth: real same-quota metadata/default/map capacities.
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut doc = old.document().clone();
    doc.size = size;
    let bounds = doc.bounds();
    let format = doc.pixel_format();
    let default = vec![0; format.bytes_per_pixel()];
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let header = q.reserve(QuotaKind::Resident, (std::mem::size_of::<GeneratedStore>() + 128) as u64).unwrap();
    let store = GeneratedStore { descriptor, quota: q.clone(), _header: header };
    let raster = BackedRaster::new(Arc::new(store), q.clone(), 2, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let state = doc.xcf_state.as_mut().unwrap();
    let layer_id = doc.layers[0].id;
    state.items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(layer_id)).unwrap().raster_bounds = Some(bounds);
    let mask = &mut state.filter_masks[0];
    mask.bounds = bounds;
    // Share the genuinely retained all-zero physical tile; only map headers grow.
    let zero = mask.surface.tiles().next().map(|(_, t)| Arc::clone(t));
    if let Some(zero) = zero {
        for coord in bounds.tiles() {
            mask.surface.put_tiles(std::iter::once((coord, Arc::clone(&zero))));
        }
    }
    let item = state.items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Mask(mask.id)).unwrap();
    item.raster_bounds = Some(bounds);
    let metadata = item.mask.as_mut().unwrap();
    metadata.bounds = bounds;
    metadata.offset = [0, 0];
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(actual.retained_bytes() <= 8 << 20);
    AccountedDocument::new_readonly_point_storage(doc, Arc::new(FixtureRetention { _original: old, _index: index, _resident: resident }), q, &ctl()).unwrap()
}
fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let bytes = photocraft_format::owned_document::save_current_document_archive(owner, Cursor::new(Vec::new()), Default::default(), owner.quota(), &ctl())
        .unwrap()
        .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        Default::default(),
        owner.quota().clone(),
        &ctl(),
    )
    .unwrap()
}
fn adopt(owner: &AccountedDocument) -> Session {
    let mut s = Session::new();
    s.add_owned_native_point_archive(archive(owner), "Synthetic larger CURRENT", None, &ctl()).unwrap();
    s
}
fn node(s: &Session) -> &FilterNode {
    &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
}
fn patch(s: &Session, family: usize) -> Value {
    let value = if family == 0 {
        json!({"name":"color","value":{"Color":{"components":[0.25f32,0.5,0.75,0.375].map(|v| json!({"Float32Bits":v.to_bits()})),"encoding":"nonlinear_profile","profile":null}}})
    } else {
        json!({"name":if family == 1 {"hue"} else {"vibrance"},"value":{"Number":{"Float32Bits":15f32.to_bits()}}})
    };
    json!({"filterId":node(s).id.0.to_string(),"arguments":[value],"enabled":false})
}
fn raw_hash(s: &Session) -> Vec<u8> {
    let LayerContent::BackedRaster(layer) = &s.active().unwrap().doc.layers[0].content else { panic!("backed") };
    let mut h = Sha256::new();
    h.update(layer.default_bytes());
    for coord in layer.operative_bounds().tiles() {
        match layer.checked_view().pin_tile(coord, &ctl()).unwrap().encoded() {
            photocraft_raster::checked_view::NativeTileBytes::FullTile(bytes) | photocraft_raster::checked_view::NativeTileBytes::DefaultPixel(bytes) => {
                h.update(bytes)
            }
        }
    }
    h.finalize().to_vec()
}
#[test]
#[ignore = "UNRUN: six larger synthetic CURRENT roles, parameter+enable once and supported metadata; no larger producer or numerical oracle"]
fn six_roles_atomic_filter_and_supported_properties() {
    for family in 0..3 {
        for depth in 0..2 {
            let owner = fixture_owner(family, depth, Size::new(257, 259));
            let mut s = adopt(&owner);
            let before = s.active().unwrap().doc.clone();
            let raw = raw_hash(&s);
            let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
            let p = patch(&s, family);
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            assert!(!node(&s).enabled);
            assert_eq!(node(&s).id, stack.nodes[0].id);
            assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].revision, stack.revision + 1);
            let changed = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.clone();
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            assert!(s.active().unwrap().doc.shares_state(&changed));
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            let id = node(&s).id.0.to_string();
            assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
            s.execute("layer.setProps", json!({"name":"Larger current", "visible":false, "opacity":0.375})).unwrap();
            let d = &s.active().unwrap().doc;
            assert_eq!(d.layers[0].name, "Larger current");
            assert!(!d.layers[0].visible);
            assert_eq!(d.layers[0].opacity.to_bits(), 0.375f32.to_bits());
            assert_eq!(d.xcf_state.as_ref().unwrap().items[0].name, "Larger current");
            assert_eq!(d.icc_profile, before.icc_profile);
            assert_eq!(d.source_color_context, before.source_color_context);
            assert_eq!(raw_hash(&s), raw);
            let filter = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":node(&s).id.0.to_string()}), &ctl()).unwrap();
            drop(filter);
            let project = crate::accounted_xcf_inspect::prepare(&s, &Value::Null, &ctl()).unwrap();
            assert_eq!(project.value()["filters"]["total"], 1);
        }
    }
}
#[test]
#[ignore = "UNRUN: skinny tile crossing removal to empty CURRENT/native and undo/redo preserves raw backing/profile/mask lifecycle"]
fn remove_empty_native_history() {
    for family in 0..3 {
        for depth in 0..2 {
            let owner = fixture_owner(family, depth, Size::new(1, 257));
            let mut s = adopt(&owner);
            let before = s.active().unwrap().doc.clone();
            let raw = raw_hash(&s);
            let id = node(&s).id.0.to_string();
            assert_eq!(s.execute("xcf.filter.remove", json!({"filterId":id})).unwrap()["updated"], true);
            let empty = s.active().unwrap().doc.clone();
            let x = empty.xcf_state.as_ref().unwrap();
            assert!(x.filter_stacks[0].nodes.is_empty());
            assert!(x.filter_masks.is_empty());
            assert_eq!(x.items.len(), 1);
            assert_eq!(raw_hash(&s), raw);
            assert_eq!(empty.icc_profile, before.icc_profile);
            let r = adopt(empty.accounted().unwrap());
            assert!(r.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
            assert_eq!(raw_hash(&r), raw);
            assert!(s.undo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(s.redo_with_control(&ctl()).unwrap());
            assert!(s.active().unwrap().doc.shares_state(&empty));
        }
    }
}
#[test]
#[ignore = "UNRUN: typed native admission rejects offset/current-profile and canvas-drawable mismatch without publishing"]
fn wrong_geometry_profile_refuses_before_publication() {
    for family in 0..3 {
        for variant in 0..3 {
            let owner = fixture_owner(family, 0, Size::new(257, 1));
            let q = owner.quota().clone();
            let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
            let resident = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
            let mut d = owner.document().clone();
            if variant == 0 {
                let LayerContent::BackedRaster(layer) = &d.layers[0].content else { panic!("native") };
                let shifted = layer.try_translated(1, 0, &ctl()).unwrap();
                let bounds = shifted.operative_bounds();
                d.layers[0].content = LayerContent::BackedRaster(shifted);
                d.xcf_state.as_mut().unwrap().items[0].raster_bounds = Some(bounds);
            } else if variant == 1 {
                d.icc_profile = Some(Arc::new(vec![0; 132]));
            } else {
                d.size = Size::new(258, 1);
                // Keep the mask structurally canvas-bound while the actual
                // native drawable and its canonical row remain 257x1. This
                // isolates CURRENT canvas/drawable refusal after L1 custody.
                let canvas = d.bounds();
                let state = d.xcf_state.as_mut().unwrap();
                let mask = &mut state.filter_masks[0];
                mask.bounds = canvas;
                let id = mask.id;
                let item = state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Mask(id)).unwrap();
                item.raster_bounds = Some(canvas);
                item.mask.as_mut().unwrap().bounds = canvas;
            }
            let malformed = AccountedDocument::new_readonly_point_storage(
                d,
                Arc::new(FixtureRetention { _original: owner, _index: index, _resident: resident }),
                q,
                &ctl(),
            )
            .unwrap();
            let mut s = Session::new();
            // A malformed profile may be refused by the native writer/loader
            // invariants before typed adoption; every phase must fail closed.
            let saved = photocraft_format::owned_document::save_current_document_archive(
                &malformed,
                Cursor::new(Vec::new()),
                Default::default(),
                malformed.quota(),
                &ctl(),
            );
            if let Ok(saved) = saved {
                let bytes = saved.into_inner();
                if let Ok(archive) = photocraft_io::accounted_native::open_accounted_native_from_reader(
                    &mut Cursor::new(bytes.as_slice()),
                    bytes.len() as u64,
                    Default::default(),
                    malformed.quota().clone(),
                    &ctl(),
                ) {
                    assert!(s.add_owned_native_point_archive(archive, "Refuse synthetic", None, &ctl()).is_err());
                }
            }
            assert!(s.active().is_none());
        }
    }
}
#[test]
#[ignore = "UNRUN: one cumulative control across parse/current/staged/execute and late cancellation/publication fences; no work reset"]
fn cumulative_control_and_fences_leave_state_atomic() {
    let owner = fixture_owner(2, 1, Size::new(257, 3));
    let mut s = adopt(&owner);
    let p = patch(&s, 2);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let used = AtomicU64::new(0);
    let limit = AtomicU64::new(u64::MAX);
    let stop = AtomicBool::new(false);
    let debit = |n| {
        let next = used.fetch_add(n, Ordering::Relaxed).saturating_add(n);
        if next > limit.load(Ordering::Relaxed) { Err(BackingError::Work) } else { Ok(()) }
    };
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &debit };
    let prepared = Request::parse(&s, "xcf.filter.set", &p, &c).unwrap().prepare(&s, &c).unwrap();
    assert!(used.load(Ordering::Relaxed) > 0);
    limit.store(used.load(Ordering::Relaxed), Ordering::Relaxed);
    assert!(prepared.execute(&mut s, &c).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
    // Separate operation/control; the failed operation's cumulative meter was never reset/retried.
    let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
    stop.store(true, Ordering::Relaxed);
    let cancel_control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(prepared.execute(&mut s, &cancel_control).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal, journal);
    let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
