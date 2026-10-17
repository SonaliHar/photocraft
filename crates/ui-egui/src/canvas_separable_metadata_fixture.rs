//! UNRUN CURRENT-only metadata controls. Native/reference buffers are external
//! harness captures, not source evidence or production allocation receipts.
use super::original::{Case, ctl};
use photocraft_doc::{Document, LayerContent, item_identity::ItemRef};
use photocraft_raster::checked_view::NativeTileBytes;
use sha2::{Digest, Sha256};
pub(super) fn payload(a: &Document, b: &Document) {
    assert_eq!(a.layers.len(), b.layers.len());
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.blend, b.blend);
        assert_eq!(a.import_compositing, b.import_compositing);
        assert_eq!(a.fill_opacity.to_bits(), b.fill_opacity.to_bits());
        assert_eq!(a.locks, b.locks);
        assert!(a.mask.is_none() && b.mask.is_none());
        let mut digests = Vec::new();
        for layer in [a, b] {
            let LayerContent::BackedRaster(r) = &layer.content else { panic!("two native leaves") };
            let mut h = Sha256::new();
            h.update(format!("{:?}", r.descriptor()).as_bytes());
            h.update(r.descriptor().default_bytes());
            for coord in r.operative_bounds().tiles() {
                let pin = r.checked_view().pin_tile(coord, &ctl()).unwrap();
                h.update(format!("{coord:?}").as_bytes());
                match pin.encoded() {
                    NativeTileBytes::FullTile(v) | NativeTileBytes::DefaultPixel(v) => h.update(v),
                }
            }
            digests.push(h.finalize());
        }
        assert_eq!(digests[0], digests[1]);
    }
    assert_eq!(a.icc_profile, b.icc_profile);
    assert_eq!(a.sample_encoding, b.sample_encoding);
    assert_eq!(a.source_color_context, b.source_color_context);
    assert_eq!(a.working_profile_interpretation, b.working_profile_interpretation);
    // This external harness clone normalizes ONLY the three permitted current
    // canonical fields for comparison; no raw policy/source field is erased.
    let mut canonical = b.xcf_state.clone().unwrap();
    let before = a.xcf_state.as_ref().unwrap();
    for item in &mut canonical.items {
        let prior = before.items.iter().find(|p| p.item == item.item).unwrap();
        let ItemRef::Layer(id) = item.item else { panic!("no masks") };
        let layer = b.layer(id).unwrap();
        assert_eq!(item.name, layer.name);
        assert_eq!(item.visible, layer.visible);
        item.name = prior.name.clone();
        item.visible = prior.visible;
        item.custom_name = prior.custom_name;
    }
    assert_eq!(before, &canonical);
}
/// Existing public pure compositing kernels over independently captured source
/// native samples + actual CURRENT metadata. Not new GIMP authored edit evidence
/// and not the restrictive original-scene PreparedRenderer qualification.
pub(super) fn reference(root: &std::path::Path, case: &Case, doc: &Document) -> Vec<[f32; 4]> {
    use photocraft_compose::{gimp_blend, gimp_composite::WorkingSpace};
    let profile = photocraft_cms::Builtin::Srgb.profile();
    let mut inputs = Vec::new();
    for row in case.state["layers"].as_array().unwrap().iter().rev() {
        let raw = std::fs::read(root.join(row["native"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(super::original::hash(&raw), row["native"]["sha256"].as_str().unwrap());
        inputs.push(raw);
    }
    let mut out = Vec::new();
    for y in 0..12 {
        for x in 0..16 {
            let mut bg = [0.; 4];
            for (index, layer) in doc.layers.iter().enumerate() {
                if !layer.visible {
                    continue;
                }
                let LayerContent::BackedRaster(r) = &layer.content else { panic!("native") };
                let bounds = r.operative_bounds();
                let inside = x >= bounds.x0 && x < bounds.x1 && y >= bounds.y0 && y < bounds.y1;
                let mut pixel = [0.; 4];
                if inside {
                    let at = ((y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize) * 16;
                    for c in 0..4 {
                        pixel[c] = f64::from(f32::from_le_bytes(inputs[index][at + c * 4..at + c * 4 + 4].try_into().unwrap()));
                    }
                }
                let p = doc.xcf_state.as_ref().unwrap().items.iter().find(|p| p.item == ItemRef::Layer(layer.id)).unwrap().composition.as_ref().unwrap();
                let resolved = p.checked_resolved().unwrap();
                let space = WorkingSpace::new(profile, doc.sample_encoding.unwrap(), resolved.composite_space).unwrap();
                let blend = WorkingSpace::new(profile, doc.sample_encoding.unwrap(), resolved.blend_space).unwrap();
                let opacity = if inside { f64::from(layer.opacity) } else { 0. };
                bg = if p.source_mode == 28 {
                    space.normal(bg, pixel, opacity, resolved.operator).unwrap()
                } else {
                    space.separable(&blend, bg, pixel, opacity, resolved.operator, gimp_blend::mode(p.source_mode).unwrap()).unwrap()
                };
            }
            out.push(bg.map(|v| v as f32));
        }
    }
    out
}
