//! Test-only real authored mixed-root input custody plus explicitly synthetic
//! larger CURRENT storage. Producer projection/profile/Float HOLDs are not oracles.
use photocraft_doc::{BackedLayer, Document, Layer, LayerContent, Size, accounted_document::AccountedDocument, item_identity::ItemRef};
use photocraft_raster::backing::{
    BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, clippy::module_inception, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod native;
pub(super) fn ctl() -> ReadControl<'static> {
    native::ctl()
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn archive(o: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(o)
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn encoded(o: &AccountedDocument) -> Vec<u8> {
    native::encoded(o)
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn workspace() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn path(index: usize) -> std::path::PathBuf {
    workspace().join(format!("temp/root-filtered-flat-gimp-20261016/author-v1/outputs/f{}-plain-semi-hue-vibrance.xcf", if index == 0 { 32 } else { 64 }))
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn import(index: usize) -> photocraft_io::OwnedXcfImport {
    let bytes = std::fs::read(path(index)).unwrap();
    let (length, expected) = if index == 0 {
        (4930, "25b2e44cff477af09f4677db4777416f324af0432aceb69341155782619667da")
    } else {
        (4928, "14f2563bbb37ecd6fdaabb5d6e00bf5dd6863f5456ab485913c54fae2e330c66")
    };
    assert_eq!(bytes.len(), length);
    assert_eq!(hash(&bytes), expected);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut options = photocraft_xcf::DecodeOptions::default();
    options.limits.resident_bytes = 64 << 20;
    options.limits.input_bytes = 16 << 20;
    let imported = photocraft_io::import_owned_xcf_bytes_in_quota("author-mixed.xcf", Arc::new(bytes), &options, &limits, q, &ctl()).unwrap();
    assert!(matches!(imported, photocraft_io::OwnedXcfImport::FilteredFlat(_)));
    assert!(imported.document().is_readonly_filtered_flat_point_storage());
    observed(imported.document().document(), index);
    imported
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn raw(layer: &Layer) -> Vec<u8> {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("backed") };
    let r = b.operative_bounds();
    let bpp = b.format().bytes_per_pixel();
    let mut bytes = vec![0; r.width() as usize * r.height() as usize * bpp];
    for coord in r.tiles() {
        let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
        let p = coord.rect().intersect(&r);
        for y in p.y0..p.y1 {
            for x in p.x0..p.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * bpp;
                pin.read_pixel_native_into(x, y, &mut bytes[at..at + bpp], &ctl()).unwrap();
            }
        }
    }
    bytes
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn observed(doc: &Document, index: usize) {
    assert_eq!(doc.size, Size::new(8, 6));
    assert_eq!(doc.layers.len(), 4);
    assert_eq!(doc.mode, photocraft_doc::ColorMode::Rgb);
    assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, if index == 0 { 650 } else { 750 });
    assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
    assert!(doc.icc_profile.is_some());
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    // Generated CURRENT builtin relation only; never producer effective/Babl ICC equality.
    assert_eq!(doc.icc_profile.as_ref().unwrap().as_slice(), photocraft_cms::Builtin::Srgb.profile().to_bytes().as_slice());
    assert_eq!(doc.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap()));
    let case = path(index).with_extension("case.json");
    let bytes = std::fs::read(&case).unwrap();
    assert_eq!(
        hash(&bytes),
        if index == 0 {
            "431acd5d59e29b406f3faae3c20d561e264a1cad3d485d154d1a66c9e49e24b6"
        } else {
            "80ee21c48a85e8cf300c5ad86e7baf8e0bf1793c55e6ea191973a1f481ba761a"
        }
    );
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let roots = value["raw"]["roots"].as_array().unwrap();
    assert_eq!(doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), roots.iter().rev().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>());
    for layer in &doc.layers {
        let row = roots.iter().find(|r| r["name"] == layer.name).unwrap();
        let blob = &row["stored_source"];
        let bytes = std::fs::read(case.parent().unwrap().join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), blob["sha256"].as_str().unwrap());
        assert_eq!(bytes.len() as u64, blob["bytes"].as_u64().unwrap());
        assert_eq!(raw(layer), bytes);
    }
    let state = doc.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_stacks.len(), 3);
    assert_eq!(state.filter_masks.len(), 3);
    assert_eq!(state.items.len(), 7);
    assert_eq!(state.selection.selected.len(), 1);
    for stack in &state.filter_stacks {
        assert_eq!(stack.nodes.len(), 1);
        let ItemRef::Layer(id) = stack.owner else { panic!("root owner") };
        let name = &doc.layers.iter().find(|l| l.id == id).unwrap().name;
        let op = match name.as_str() {
            "Own root 1" => "gimp:semi-flatten",
            "Own root 2" => "gegl:hue-chroma",
            "Own root 3" => "gegl:vibrance",
            _ => panic!("filtered owner"),
        };
        assert_eq!(stack.nodes[0].operation.name, op);
        let observed = roots.iter().find(|r| r["name"] == *name).unwrap();
        let blob = &observed["effects"][0]["mask"]["stored_native"];
        let expected = std::fs::read(case.parent().unwrap().join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&expected), blob["sha256"].as_str().unwrap());
        let mask = state.filter_masks.iter().find(|m| m.owner == stack.nodes[0].id).unwrap();
        assert_eq!(mask.surface.try_read_interleaved(doc.bounds()).unwrap(), expected);
    }
}
#[derive(Debug)]
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
struct Pattern {
    bytes: Vec<u8>,
    descriptor: NativePlaneDescriptor,
    width: u32,
    bpp: usize,
    quota: BackingQuota,
    _payload: QuotaLease,
    _header: QuotaLease,
}
impl NativePlaneStore for Pattern {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        if at.checked_add(out.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        c.check(out.len() as u64)?;
        for (i, v) in out.iter_mut().enumerate() {
            if i % 512 == 0 {
                c.check(0)?
            }
            let at = at as usize + i;
            let p = at / self.bpp;
            let x = p % self.width as usize;
            let y = p / self.width as usize;
            *v = *self.bytes.get(((y % 6) * 8 + x % 8) * self.bpp + at % self.bpp).ok_or(BackingError::Bounds)?;
        }
        Ok(())
    }
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
struct Guard {
    _source: Arc<photocraft_io::OwnedXcfImport>,
    _index: QuotaLease,
    _resident: QuotaLease,
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn larger(source: Arc<photocraft_io::OwnedXcfImport>) -> AccountedDocument {
    let old = source.document().document();
    let q = source.document().quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut doc = old.clone();
    let size = Size::new(257, 259);
    let bounds = photocraft_doc::Rect::from_xywh(0, 0, size.width, size.height);
    doc.size = size;
    for (layer, original) in doc.layers.iter_mut().zip(&old.layers) {
        let LayerContent::BackedRaster(b) = &original.content else { panic!("backed") };
        let length = 48 * b.format().bytes_per_pixel();
        let mut fee = q.reserve(QuotaKind::Resident, (length * 2) as u64).unwrap();
        let bytes = raw(original);
        assert_eq!(bytes.len(), length);
        assert!(bytes.capacity() <= length * 2);
        fee.reconcile_capacity(bytes.capacity() as u64).unwrap();
        let descriptor = NativePlaneDescriptor::new(b.format(), bounds, b.default_bytes()).unwrap();
        let header = q.reserve(QuotaKind::Index, (std::mem::size_of::<Pattern>() + 128) as u64).unwrap();
        let raster = BackedRaster::new(
            Arc::new(Pattern { bytes, descriptor, width: size.width, bpp: b.format().bytes_per_pixel(), quota: q.clone(), _payload: fee, _header: header }),
            q.clone(),
            2,
            &ctl(),
        )
        .unwrap();
        layer.content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    }
    let state = doc.xcf_state.as_mut().unwrap();
    for mask in &mut state.filter_masks {
        mask.bounds = bounds;
        let tile = mask.surface.tiles().next().map(|(_, t)| Arc::clone(t));
        if let Some(tile) = tile {
            for coord in bounds.tiles() {
                mask.surface.put_tiles(std::iter::once((coord, tile.clone())));
            }
        }
    }
    for item in &mut state.items {
        item.raster_bounds = Some(bounds);
        if let Some(mask) = &mut item.mask {
            mask.bounds = bounds;
            mask.offset = [0, 0];
        }
    }
    AccountedDocument::new_readonly_filtered_flat_point_storage(doc, Arc::new(Guard { _source: source, _index: index, _resident: resident }), q, &ctl())
        .unwrap()
}
struct CopyGuard {
    _source: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
pub(super) fn current(owner: &AccountedDocument, change: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 16 << 20).unwrap();
    let mut doc = owner.document().clone();
    change(&mut doc);
    AccountedDocument::new_readonly_filtered_flat_point_storage(
        doc,
        Arc::new(CopyGuard { _source: owner.clone(), _index: index, _resident: resident }),
        q,
        &ctl(),
    )
    .unwrap()
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn same(a: &Document, b: &Document) {
    assert_eq!(a.name, b.name);
    storage(a, b);
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn storage(a: &Document, b: &Document) {
    assert_eq!(a.size, b.size);
    assert_eq!(a.mode, b.mode);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.xcf_state, b.xcf_state);
    assert_eq!(a.layers.len(), b.layers.len());
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.name, b.name);
        assert_eq!(a.visible, b.visible);
        assert_eq!(a.opacity.to_bits(), b.opacity.to_bits());
        assert_eq!(raw(a), raw(b));
        let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&a.content, &b.content) else { panic!("backed") };
        assert_eq!(a.descriptor(), b.descriptor());
        assert_eq!(a.default_bytes(), b.default_bytes());
    }
}
#[allow(dead_code, reason = "Shared cfg(test) fixture retains this item for other independent consumers.")]
pub(super) fn activity(owner: &AccountedDocument) -> AccountedDocument {
    current(owner, |doc| {
        doc.layers[1].visible = false;
        doc.layers[2].opacity = 0.0;
        let hidden = doc.layers[1].id;
        let s = doc.xcf_state.as_mut().unwrap();
        s.items.iter_mut().find(|i| i.item == ItemRef::Layer(hidden)).unwrap().visible = false;
        s.filter_stacks[0].nodes[0].enabled = false;
    })
}
