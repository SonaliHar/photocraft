//! Synthetic CURRENT-only native fixture. No source-XCF producer/provenance or
//! independent GIMP geometry claim. Procedural stores own real same-ledger fees;
//! model/profile holds precede allocation, reference/output files are harness data.
use photocraft_doc::{
    BackedLayer, ColorMode, Document, ImportCompositing, Layer, LayerContent, Rect, SampleEncoding, SampleType, Size, SourceColorContext,
    WorkingProfileInterpretation, accounted_document::AccountedDocument, import_compositing::*, item_identity::ItemRef, xcf_state::*,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    pixel: [u8; 16],
    quota: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.pixel[((offset + i as u64) % 16) as usize];
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub(super) struct Literal {
    pub bounds: Rect,
    pub pixel: [f32; 4],
}
fn encoding(s: GimpSpace) -> SampleEncoding {
    match s {
        GimpSpace::LinearRgb => SampleEncoding::Linear,
        GimpSpace::ProfileRgb => SampleEncoding::NonlinearProfile,
        GimpSpace::PerceptualRgb => SampleEncoding::Perceptual,
        _ => panic!("fixture excludes Lab"),
    }
}
pub(super) fn fixture() -> (AccountedDocument, Vec<Literal>) {
    build(false, false)
}
pub(super) fn normal_fixture() -> (AccountedDocument, Vec<Literal>) {
    build(true, false)
}
pub(super) fn malformed_fixture() -> (AccountedDocument, Vec<Literal>) {
    build(true, true)
}
fn build(all_normal: bool, malformed: bool) -> (AccountedDocument, Vec<Literal>) {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let model = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let profile = q.reserve(QuotaKind::Resident, 65536).unwrap();
    let mut doc = Document::new("Synthetic current five-layer flat", Size::new(263, 19), ColorMode::Rgb, SampleType::F32);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
    doc.source_color_context = Some(SourceColorContext { source_precision: 650, original_icc: None });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 23;
    let specs = [
        (28, Rect::new(-5, -3, 270, 22), GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::Union, true, 0.8),
        (31, Rect::new(-7, 1, 260, 14), GimpSpace::PerceptualRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop, true, 0.6),
        (30, Rect::new(250, -4, 278, 23), GimpSpace::LinearRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToLayer, true, 0.4),
        (23, Rect::new(-2, 7, 264, 28), GimpSpace::ProfileRgb, GimpSpace::PerceptualRgb, GimpOperator::Intersection, false, 0.5),
        (33, Rect::new(4, 2, 260, 18), GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::Union, true, 0.0),
    ];
    let mut literals = Vec::new();
    for (index, (mode, bounds, blend, composite, operator, visible, opacity)) in specs.into_iter().enumerate() {
        let mode = if all_normal { 28 } else { mode };
        let pixel = [0.2 + index as f32 * 0.05, 0.35, 0.65 - index as f32 * 0.04, 0.625];
        let raw: Vec<u8> = pixel.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let default: Vec<u8> = [0.03125f32, 0.0625, 0.125, 0.0].into_iter().flat_map(f32::to_ne_bytes).collect();
        let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), bounds, &default).unwrap();
        let fee = q.reserve(QuotaKind::Index, (std::mem::size_of::<Store>() + 128) as u64).unwrap();
        let store = Arc::new(Store { descriptor, pixel: raw.try_into().unwrap(), quota: q.clone(), _fee: fee });
        let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
        let backed = BackedLayer::new(raster, descriptor, &ctl()).unwrap();
        let mut layer = Layer::new(format!("Current leaf {index}"), LayerContent::BackedRaster(backed));
        layer.visible = visible;
        layer.opacity = opacity;
        layer.import_compositing = Some(ImportCompositing { blend_encoding: encoding(blend), composite_encoding: encoding(composite), pass_through: false });
        let rawspace = |s| match s {
            GimpSpace::LinearRgb => 1,
            GimpSpace::ProfileRgb => 2,
            GimpSpace::PerceptualRgb => 4,
            _ => panic!("RGB"),
        };
        let rawop = if malformed && index == 0 {
            0
        } else {
            match operator {
                GimpOperator::Union => 1,
                GimpOperator::ClipToBackdrop => 2,
                GimpOperator::ClipToLayer => 3,
                GimpOperator::Intersection => 4,
            }
        };
        state.items.insert(
            0,
            ItemState {
                item: ItemRef::Layer(layer.id),
                name: layer.name.clone(),
                custom_name: false,
                visible,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(bounds),
                mask: None,
                composition: Some(GimpComposition {
                    compatibility_epoch: "synthetic-current-explicit-composition".into(),
                    source_mode: mode,
                    legacy: false,
                    stored_blend_space: rawspace(blend),
                    stored_composite_space: rawspace(composite),
                    stored_operator: rawop,
                    resolved: Some(ResolvedGimpComposition { blend_space: blend, composite_space: composite, operator }),
                    pass_through: false,
                }),
            },
        );
        doc.layers.push(layer);
        literals.push(Literal { bounds, pixel });
    }
    state.selection.selected = vec![ItemRef::Layer(doc.layers[4].id), ItemRef::Layer(doc.layers[2].id), ItemRef::Layer(doc.layers[0].id)];
    state.selection.active = Some(ItemRef::Layer(doc.layers[2].id));
    doc.xcf_state = Some(state);
    let census = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(census.retained_bytes() as u64 <= model.bytes());
    assert!(doc.icc_profile.as_ref().unwrap().capacity() as u64 <= profile.bytes());
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((model, profile)), q, &ctl()).unwrap();
    (owner, literals)
}
pub(super) fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let q = owner.quota();
    let encoded =
        photocraft_format::owned_document::save_current_document_archive(owner.document(), std::io::Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
    photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(encoded.as_slice()),
        encoded.len() as u64,
        Default::default(),
        q.clone(),
        &ctl(),
    )
    .unwrap()
}
pub(super) fn same_quota(owner: &AccountedDocument) {
    owner.validate_readonly_flat_composite_storage(&ctl()).unwrap();
    for layer in &owner.document().layers {
        let LayerContent::BackedRaster(b) = &layer.content else { panic!("native") };
        assert!(owner.quota().shares_ledger(b.quota()));
    }
}
pub(super) fn payload(a: &Document, b: &Document) {
    for (a, b) in a.layers.iter().zip(&b.layers) {
        let mut hashes = Vec::new();
        for l in [a, b] {
            let LayerContent::BackedRaster(r) = &l.content else { panic!("native") };
            let mut h = Sha256::new();
            h.update(format!("{:?}", r.descriptor()).as_bytes());
            h.update(r.descriptor().default_bytes());
            for c in r.operative_bounds().tiles() {
                let p = r.checked_view().pin_tile(c, &ctl()).unwrap();
                h.update(format!("{c:?}").as_bytes());
                match p.encoded() {
                    NativeTileBytes::FullTile(v) | NativeTileBytes::DefaultPixel(v) => h.update(v),
                }
            }
            hashes.push(h.finalize());
        }
        assert_eq!(hashes[0], hashes[1]);
    }
    assert_eq!(a.layers.len(), b.layers.len());
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    assert_eq!(a.source_color_context, b.source_color_context);
}
pub(super) fn reference(doc: &Document, literals: &[Literal]) -> Vec<[f32; 4]> {
    use photocraft_compose::{gimp_blend, gimp_composite::WorkingSpace};
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let mut out = Vec::new();
    for y in 0..doc.size.height as i32 {
        for x in 0..doc.size.width as i32 {
            let mut bg = [0.; 4];
            for (layer, lit) in doc.layers.iter().zip(literals) {
                if !layer.visible {
                    continue;
                }
                let p = doc.xcf_state.as_ref().unwrap().items.iter().find(|p| p.item == ItemRef::Layer(layer.id)).unwrap().composition.as_ref().unwrap();
                let r = p.checked_resolved().unwrap();
                let inside = x >= lit.bounds.x0 && x < lit.bounds.x1 && y >= lit.bounds.y0 && y < lit.bounds.y1;
                let pixel = if inside { lit.pixel.map(f64::from) } else { [0.; 4] };
                let opacity = if inside { f64::from(layer.opacity) } else { 0. };
                let working = WorkingSpace::new(profile, doc.sample_encoding.unwrap(), r.composite_space).unwrap();
                let blend = WorkingSpace::new(profile, doc.sample_encoding.unwrap(), r.blend_space).unwrap();
                bg = if p.source_mode == 28 {
                    working.normal(bg, pixel, opacity, r.operator).unwrap()
                } else {
                    working.separable(&blend, bg, pixel, opacity, r.operator, gimp_blend::mode(p.source_mode).unwrap()).unwrap()
                };
            }
            out.push(bg.map(|v| v as f32));
        }
    }
    out
}

pub(super) fn page(value: &serde_json::Value, doc: &Document, offset: usize, limit: usize) {
    use serde_json::json;
    let s = doc.xcf_state.as_ref().unwrap();
    assert_eq!(value["items"]["total"], s.items.len());
    assert_eq!(value["items"]["offset"], offset);
    assert_eq!(value["items"]["limit"], limit);
    let rows = value["items"]["entries"].as_array().unwrap();
    assert_eq!(rows.len(), s.items.len().saturating_sub(offset).min(limit));
    for (row, item) in rows.iter().zip(s.items.iter().skip(offset).take(limit)) {
        assert_eq!(row["item"], json!({"kind":"layer","id":item.item.raw().to_string()}));
        assert_eq!(row["name"], item.name);
        assert_eq!(row["visible"], item.visible);
        assert_eq!(row["composition"]["storedBlendSpace"], item.composition.as_ref().unwrap().stored_blend_space);
        assert_eq!(row["composition"]["storedCompositeSpace"], item.composition.as_ref().unwrap().stored_composite_space);
        assert_eq!(row["composition"]["storedOperator"], item.composition.as_ref().unwrap().stored_operator);
    }
    let selected = &value["selection"]["selected"];
    assert_eq!(selected["total"], s.selection.selected.len());
    assert_eq!(selected["offset"], offset);
    assert_eq!(selected["limit"], limit);
    let expected: Vec<_> = s.selection.selected.iter().skip(offset).take(limit).map(|i| json!({"kind":"layer","id":i.raw().to_string()})).collect();
    assert_eq!(selected["entries"], json!(expected));
    assert_eq!(value["selection"]["active"], json!({"kind":"layer","id":s.selection.active.unwrap().raw().to_string()}));
    assert_eq!(value["filters"]["total"], 0);
    assert_eq!(value["filterStacks"]["total"], 0);
}
