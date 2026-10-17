//! UNRUN host-only CURRENT metadata fixture support; no source eligibility grant.
use photocraft_doc::{Document, Layer, LayerContent, LayerId, item_identity::ItemRef};
use photocraft_raster::checked_view::NativeTileBytes;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
#[path = "canvas_gaussian_group_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, clippy::panic, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod original;
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    original::ctl()
}
pub(super) fn assets() -> PathBuf {
    original::assets()
}
#[allow(dead_code)] // UI resolves this through its actual guarded mask capture.
pub(super) fn physical(doc: &Document) -> photocraft_doc::item_identity::MaskId {
    let group = ids(doc)[0];
    let state = doc.xcf_state.as_ref().unwrap();
    state
        .items
        .iter()
        .find_map(|item| {
            let ItemRef::Mask(id) = item.item else { return None };
            let mask = item.mask.as_ref()?;
            (mask.owner == ItemRef::Layer(group) && !state.filter_masks.iter().any(|m| m.id == id)).then_some(id)
        })
        .unwrap()
}
pub(super) fn ids(doc: &Document) -> [LayerId; 4] {
    let group = &doc.layers[1];
    let LayerContent::Group(g) = &group.content else { panic!("retained group") };
    assert_eq!(g.children.len(), 2);
    [group.id, g.children[0].id, g.children[1].id, doc.layers[0].id]
}
fn words(layers: &[Layer], out: &mut Vec<String>, depth: usize) {
    assert!(depth <= 1);
    for layer in layers {
        let mut d = Sha256::new();
        d.update(format!("{:?}", (layer.id, layer.blend, layer.fill_opacity.to_bits(), &layer.locks, layer.clipped, layer.excluded_channels)).as_bytes());
        match &layer.content {
            LayerContent::Group(g) => {
                assert_eq!(depth, 0);
                assert_eq!(g.children.len(), 2);
                d.update(format!("{:?}", (g.expanded, &g.artboard)).as_bytes());
            }
            LayerContent::BackedRaster(b) => {
                d.update(format!("{:?}", b.descriptor()).as_bytes());
                d.update(b.descriptor().default_bytes());
                for coord in b.operative_bounds().tiles() {
                    let pin = b.checked_view().pin_tile(coord, &ctl()).unwrap();
                    match pin.encoded() {
                        NativeTileBytes::FullTile(v) | NativeTileBytes::DefaultPixel(v) => d.update(v),
                    }
                }
            }
            _ => panic!("three actual native leaves"),
        }
        if let Some(mask) = &layer.mask {
            d.update(
                format!("{:?}", (mask.surface.format(), mask.surface.tile_bounds(), mask.linked, mask.density.to_bits(), mask.feather.to_bits())).as_bytes(),
            );
            d.update(mask.surface.default_bytes());
            for (coord, tile) in mask.surface.tiles() {
                d.update(format!("{coord:?}").as_bytes());
                d.update(tile.bytes());
            }
        }
        out.push(d.finalize().iter().map(|v| format!("{v:02x}")).collect());
        if let LayerContent::Group(g) = &layer.content {
            words(&g.children, out, depth + 1);
        }
    }
}
pub(super) fn payload(before: &Document, current: &Document) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    words(&before.layers, &mut a, 0);
    words(&current.layers, &mut b, 0);
    assert_eq!(a, b);
    let mut expected = before.xcf_state.as_ref().unwrap().clone();
    let actual = current.xcf_state.as_ref().unwrap();
    for item in &mut expected.items {
        match item.item {
            ItemRef::Layer(id) => {
                let physical = current.layer(id).unwrap();
                let row = actual.items.iter().find(|i| i.item == item.item).unwrap();
                assert_eq!(row.name, physical.name);
                assert_eq!(row.visible, physical.visible);
                item.name = physical.name.clone();
                item.visible = physical.visible;
                item.custom_name = row.custom_name;
            }
            ItemRef::Mask(id) if !expected.filter_masks.iter().any(|m| m.id == id) => {
                let metadata = item.mask.as_mut().unwrap();
                let ItemRef::Layer(owner) = metadata.owner else { panic!("physical owner") };
                metadata.enabled = current.layer(owner).unwrap().mask.as_ref().unwrap().enabled;
            }
            _ => {}
        }
    }
    assert_eq!(Some(expected), current.xcf_state);
    assert_eq!(before.icc_profile, current.icc_profile);
    assert_eq!(before.source_color_context, current.source_color_context);
    assert_eq!(before.working_profile_interpretation, current.working_profile_interpretation);
    assert_eq!(before.sample_encoding, current.sample_encoding);
}
