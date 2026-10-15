//! CURRENT Move controls from independent original native inputs. Synthetic view,
//! effect-reference and lock edits below are not GIMP translation observations.
//! Captured vectors/documents/writer buffers are external harness allocations.
use photocraft_doc::{Document, LayerContent, LayerId, Rect, item_identity::ItemRef};
use photocraft_raster::backing::quota::QuotaKind;
use std::sync::Arc;
#[path = "canvas_original_flat_fixture.rs"]
#[allow(dead_code)]
mod original;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    original::ctl()
}
pub(super) struct Case {
    pub input: Vec<u8>,
    pub path: std::path::PathBuf,
    inner: original::Case,
}
pub(super) fn cases() -> Vec<Case> {
    original::cases().into_iter().map(|inner| Case { input: inner.input.clone(), path: inner.path.clone(), inner }).collect()
}
pub(super) fn import(case: &Case) -> photocraft_io::OwnedXcfImport {
    original::import(&case.inner)
}
pub(super) fn observed(case: &Case, doc: &Document) {
    original::observed(&case.inner, doc)
}
fn backed(layer: &photocraft_doc::Layer) -> &photocraft_doc::BackedLayer {
    let LayerContent::BackedRaster(b) = &layer.content else { panic!("native leaf") };
    b
}
pub(super) fn native(layer: &photocraft_doc::Layer) -> Vec<u8> {
    let b = backed(layer);
    let r = b.operative_bounds();
    let mut out = vec![0; r.width() as usize * r.height() as usize * 16];
    for coord in r.tiles() {
        let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
        let part = coord.rect().intersect(&r);
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * 16;
                pin.read_pixel_native_into(x, y, &mut out[at..at + 16], &ctl()).unwrap();
            }
        }
    }
    out
}
pub(super) fn archive(owner: &photocraft_doc::accounted_document::AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    original::archive(owner)
}
pub(super) fn seed(owner: &photocraft_doc::accounted_document::AccountedDocument, locked: bool) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let q = owner.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = owner.document().clone();
    let bottom = doc.layers[0].id;
    let top = doc.layers.last().unwrap().id;
    for (i, layer) in doc.layers.iter_mut().enumerate() {
        layer.effects.reference = Some((1.25 + i as f64, -0.0));
    }
    if locked {
        doc.layers.last_mut().unwrap().locks.position = true;
    }
    let selection = &mut doc.xcf_state.as_mut().unwrap().selection;
    selection.selected = vec![ItemRef::Layer(top), ItemRef::Layer(bottom)];
    selection.active = Some(ItemRef::Layer(bottom));
    let retained =
        photocraft_doc::accounted_document::AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner.clone(), fee)), q, &ctl()).unwrap();
    archive(&retained)
}
pub(super) fn exact(before: &Document, after: &Document, deltas: &[(LayerId, i32, i32)]) {
    assert_eq!(before.layers.len(), after.layers.len());
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(before.sample_encoding, after.sample_encoding);
    let mut expected = before.xcf_state.clone().unwrap();
    for old in &before.layers {
        let now = after.layer(old.id).unwrap();
        let (_, dx, dy) = deltas.iter().copied().find(|(id, _, _)| *id == old.id).unwrap_or((old.id, 0, 0));
        let r = backed(old).operative_bounds();
        let shifted = Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
        assert_eq!(backed(now).operative_bounds(), shifted);
        assert_eq!(backed(old).format(), backed(now).format());
        assert_eq!(backed(old).default_bytes(), backed(now).default_bytes());
        assert_eq!(native(old), native(now));
        assert_eq!(old.opacity.to_bits(), now.opacity.to_bits());
        assert_eq!(old.fill_opacity.to_bits(), now.fill_opacity.to_bits());
        assert_eq!(old.visible, now.visible);
        assert_eq!(old.locks, now.locks);
        assert_eq!(old.blend, now.blend);
        let refs = old
            .effects
            .reference
            .map(|(x, y)| if (dx, dy) == (0, 0) { (x.to_bits(), y.to_bits()) } else { ((x + f64::from(dx)).to_bits(), (y + f64::from(dy)).to_bits()) });
        assert_eq!(refs, now.effects.reference.map(|(x, y)| (x.to_bits(), y.to_bits())));
        let item = expected.items.iter_mut().find(|i| i.item == ItemRef::Layer(old.id)).unwrap();
        item.raster_bounds = Some(shifted);
        if item.name != now.name {
            item.name = now.name.clone();
            item.custom_name = true;
        }
    }
    assert_eq!(Some(expected), after.xcf_state);
}
pub(super) fn reference(before: &Document, current: &Document) -> photocraft_compose::Buffer {
    use photocraft_compose::{gimp_blend, gimp_composite::WorkingSpace};
    let raw: Vec<_> = before.layers.iter().map(native).collect();
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let mut px = Vec::new();
    for y in 0..current.size.height as i32 {
        for x in 0..current.size.width as i32 {
            let mut bg = [0.; 4];
            for (index, layer) in current.layers.iter().enumerate() {
                if !layer.visible {
                    continue;
                }
                let r = backed(layer).operative_bounds();
                let inside = r.contains(x, y);
                let pixel = if inside {
                    let at = ((y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize) * 16;
                    std::array::from_fn(|c| photocraft_color::try_read_sample_f64(&raw[index][at..at + 16], photocraft_doc::SampleType::F32, c).unwrap())
                } else {
                    [0.; 4]
                };
                let p = current.xcf_state.as_ref().unwrap().items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap().composition.as_ref().unwrap();
                let resolved = p.checked_resolved().unwrap();
                let working = WorkingSpace::new(profile, current.sample_encoding.unwrap(), resolved.composite_space).unwrap();
                let blend = WorkingSpace::new(profile, current.sample_encoding.unwrap(), resolved.blend_space).unwrap();
                let opacity = if inside { f64::from(layer.opacity) } else { 0. };
                bg = if p.source_mode == 28 {
                    working.normal(bg, pixel, opacity, resolved.operator).unwrap()
                } else {
                    working.separable(&blend, bg, pixel, opacity, resolved.operator, gimp_blend::mode(p.source_mode).unwrap()).unwrap()
                };
            }
            px.push(bg.map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: current.bounds(), px }
}
