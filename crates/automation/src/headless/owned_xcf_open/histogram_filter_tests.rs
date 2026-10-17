//! UNRUN transport workflows. The engine companion verifies the SHA-bound corpus;
//! these drafts exercise actual authorized readers, registered commands and saves.
use super::*;
use photocraft_doc::LayerContent;
use photocraft_engine::DocumentSnapshot;
use photocraft_raster::backing::quota::{QuotaKind, QuotaLease};

type MaskStorage = (photocraft_raster::backing::NativePlaneDescriptor, Option<(photocraft_geom::TileCoord, Vec<u8>)>);
#[derive(Debug, PartialEq, Eq)]
struct RawStorage {
    layer: photocraft_raster::backing::NativePlaneDescriptor,
    layer_full_tile: bool,
    layer_bytes: Vec<u8>,
    masks: Vec<MaskStorage>,
}
#[derive(Debug)]
struct NativeSnapshot {
    storage: RawStorage,
    // Copied payload/metadata drop before their genuine same-ledger derivative fee.
    _lease: QuotaLease,
}
impl PartialEq for NativeSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.storage == other.storage
    }
}
impl Eq for NativeSnapshot {}
fn native(snapshot: &DocumentSnapshot) -> NativeSnapshot {
    use photocraft_raster::{backing::NativePlaneDescriptor, checked_view::NativeTileBytes};
    let LayerContent::BackedRaster(layer) = &snapshot.layers[0].content else { panic!("source is not native backed") };
    assert_eq!(snapshot.layers.len(), 1);
    assert_eq!(layer.operative_bounds(), photocraft_geom::Rect::new(0, 0, 8, 6));
    let masks = &snapshot.xcf_state.as_ref().unwrap().filter_masks;
    assert!(masks.len() <= 1);
    let full_layer = 256usize.checked_mul(256).unwrap().checked_mul(layer.format().bytes_per_pixel()).unwrap();
    let mut payload = full_layer;
    for mask in masks {
        assert!(mask.surface.tile_count() <= 1);
        payload = payload.checked_add(256 * 256 * mask.surface.format().bytes_per_pixel()).unwrap();
    }
    // Double actual maximum payload admits allocator excess; 4096 prices the
    // bounded descriptor/tuple/vector headers. No native data is narrowed.
    let admitted = payload.checked_mul(2).unwrap().checked_add(4096).unwrap() as u64;
    let mut fee = layer.quota().reserve(QuotaKind::Resident, admitted).unwrap();
    fn copy(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.try_reserve_exact(bytes.len()).unwrap();
        assert!(out.capacity() <= bytes.len().checked_mul(2).unwrap());
        for chunk in bytes.chunks(64 * 1024) {
            ctl().check(chunk.len() as u64).unwrap();
            out.extend_from_slice(chunk);
        }
        out
    }
    let pin = layer.checked_view().pin_tile(photocraft_geom::TileCoord::new(0, 0), &ctl()).unwrap();
    let (layer_full_tile, layer_bytes) = match pin.encoded() {
        NativeTileBytes::FullTile(bytes) => {
            assert_eq!(bytes.len(), full_layer);
            (true, copy(bytes))
        }
        NativeTileBytes::DefaultPixel(_) => panic!("backed authoritative tile was not full native storage"),
    };
    drop(pin);
    let mut copied_masks = Vec::new();
    copied_masks.try_reserve_exact(masks.len()).unwrap();
    assert!(copied_masks.capacity() <= 2 * masks.len());
    let mut actual = layer_bytes.capacity() + 4096;
    for mask in masks {
        ctl().check(1).unwrap();
        let descriptor = NativePlaneDescriptor::new(mask.surface.format(), mask.bounds, mask.surface.default_bytes()).unwrap();
        let mut tile = None;
        for (coord, bytes) in mask.surface.tiles() {
            assert_eq!(*coord, photocraft_geom::TileCoord::new(0, 0));
            assert_eq!(bytes.bytes().len(), 256 * 256 * mask.surface.format().bytes_per_pixel());
            let copied = copy(bytes.bytes());
            actual = actual.checked_add(copied.capacity()).unwrap();
            tile = Some((*coord, copied));
        }
        copied_masks.push((descriptor, tile));
    }
    assert!(actual as u64 <= admitted);
    fee.reconcile_capacity(actual as u64).unwrap();
    NativeSnapshot { storage: RawStorage { layer: layer.descriptor(), layer_full_tile, layer_bytes, masks: copied_masks }, _lease: fee }
}

fn corpus_case(id: &str) -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("UNRUN requires pinned threshold precision evidence"))
        .join("threshold-precision-evidence");
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("SHA256SUMS.json")).unwrap()).unwrap();
    let name = format!("{id}.xcf");
    assert_eq!(manifest[&name].as_str().unwrap().len(), 64);
    assert!(std::fs::metadata(root.join(&name)).unwrap().len() <= 16 << 20);
    std::fs::read(root.join(name)).unwrap()
}
#[test]
#[ignore = "UNRUN: pinned original Histogram Threshold corpus and native host writes"]
fn owned_original_and_native_headless_registered_filter_save_reopen_preserves_raw_storage() {
    for id in ["rgb-u16_non_linear.channel0", "rgb-float_non_linear.channel0", "rgb-double_non_linear.channel0"] {
        for rooted in [false, true] {
            let dir = Directory::new();
            let original = corpus_case(id);
            std::fs::write(dir.0.join("source.xcf"), &original).unwrap();
            let path = |name: &str| if rooted { PathBuf::from(name) } else { dir.0.join(name) };
            let mut h =
                if rooted { Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), Some(&dir.0)).unwrap()) } else { Headless::trusted_local() };
            drop(h.open_owned_xcf_keep_current(&path("source.xcf"), &ctl()).unwrap());
            let before = h.session.active().unwrap().doc.clone();
            assert!(before.accounted().unwrap().is_readonly_threshold_storage());
            let raw = native(&before);
            let original_node = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
            let original_low = serde_json::to_value(original_node.arguments.iter().find(|a| a.name == "low").unwrap()).unwrap();
            let low_bits = if original_low["value"]["Number"]["Float32Bits"] == 0.875f32.to_bits() { 0.625f32.to_bits() } else { 0.875f32.to_bits() };
            let filter = original_node.id.0.to_string();
            let patch = json!({"filterId":filter,"arguments":[{"name":"low","value":{"Number":{"Float32Bits":low_bits}}},{"name":"high","value":{"Number":{"Float32Bits":0.125f32.to_bits()}}}]});
            assert_eq!(h.command_run("xcf.filter.set", patch.clone()).unwrap()["updated"], true);
            h.command_run("edit.undo", json!({})).unwrap();
            let history = h.session.active().unwrap().history.clone();
            assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":original_node.enabled})).unwrap()["updated"], false);
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            h.command_run("edit.redo", json!({})).unwrap();
            h.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":!original_node.enabled})).unwrap();
            let current = h.session.active().unwrap().doc.clone();
            assert_eq!(native(&current), raw);
            assert_eq!(current.icc_profile, before.icc_profile);
            h.save(None, Some(&path("edited.pcraft")), Some("pcraft"), &Default::default()).unwrap();
            assert_eq!(std::fs::read(dir.0.join("source.xcf")).unwrap(), original);
            let mut reopened =
                if rooted { Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&dir.0), Some(&dir.0)).unwrap()) } else { Headless::trusted_local() };
            drop(reopened.open_owned_native_keep_current(&path("edited.pcraft"), &ctl()).unwrap());
            let saved = reopened.session.active().unwrap().doc.clone();
            assert!(saved.accounted().unwrap().is_readonly_threshold_storage());
            assert_eq!(saved.name, "edited.pcraft");
            assert_eq!(saved.xcf_state, current.xcf_state);
            assert_eq!(native(&saved), raw);
            assert!(!reopened.session.active().unwrap().source_read_only);
            // The original remains protected in the original source's Session,
            // including a differently named existing native destination alias.
            std::fs::hard_link(dir.0.join("source.xcf"), dir.0.join("alias.pcraft")).unwrap();
            assert!(h.save(None, Some(&path("alias.pcraft")), Some("pcraft"), &Default::default()).is_err());
            assert_eq!(std::fs::read(dir.0.join("source.xcf")).unwrap(), original);
            let input_archive = std::fs::read(dir.0.join("edited.pcraft")).unwrap();
            assert_eq!(reopened.command_run("xcf.filter.set", patch).unwrap()["updated"], false);
            reopened.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":original_node.enabled})).unwrap();
            reopened.save(None, Some(&path("continued.pcraft")), Some("pcraft"), &Default::default()).unwrap();
            assert_eq!(std::fs::read(dir.0.join("edited.pcraft")).unwrap(), input_archive);
            assert_eq!(native(&reopened.session.active().unwrap().doc), raw);
            for command in ["edit.clear", "edit.fill", "layer.setProps"] {
                let snapshot = reopened.session.active().unwrap().doc.clone();
                assert!(reopened.command_run(command, json!({"contents":"white","name":"unsupported"})).is_err());
                assert!(reopened.session.active().unwrap().doc.shares_state(&snapshot));
            }
        }
    }
}
