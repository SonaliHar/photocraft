//! UNRUN actual24 group sources -> registered CURRENT four-target gestures and
//! mask activation -> paid native continuation. Harness captures are external;
//! modified activity is not an independently GIMP-authored source observation.
use super::owned_isolated_normal_group_tests::{cases, ctl};
use super::*;
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_raster::checked_view::NativeTileBytes;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn ids(doc: &Document) -> Vec<photocraft_doc::LayerId> {
    let group = doc.layers.iter().find(|layer| matches!(layer.content, LayerContent::Group(_))).unwrap();
    let LayerContent::Group(content) = &group.content else { panic!("group") };
    let backdrop = doc.layers.iter().find(|layer| !matches!(layer.content, LayerContent::Group(_))).unwrap();
    vec![group.id, content.children[0].id, content.children[1].id, backdrop.id]
}
fn payload(doc: &Document) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>) {
        for layer in layers {
            let mut digest = Sha256::new();
            digest.update(layer.id.0.to_ne_bytes());
            match &layer.content {
                LayerContent::Group(group) => {
                    digest.update(format!("{:?}", (group.expanded, &group.artboard)).as_bytes());
                    assert_eq!(group.children.len(), 2);
                }
                LayerContent::BackedRaster(backed) => {
                    digest.update(format!("{:?}", backed.descriptor()).as_bytes());
                    digest.update(backed.descriptor().default_bytes());
                    for coord in backed.descriptor().bounds().tiles() {
                        let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                        match pin.encoded() {
                            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
                        }
                    }
                }
                _ => panic!("real group/native leaves only"),
            }
            if let Some(mask) = &layer.mask {
                digest.update(
                    format!("{:?}", (mask.surface.format(), mask.surface.tile_bounds(), mask.linked, mask.density.to_bits(), mask.feather.to_bits()))
                        .as_bytes(),
                );
                digest.update(mask.surface.default_bytes());
                for (coord, tile) in mask.surface.tiles() {
                    digest.update(format!("{coord:?}").as_bytes());
                    digest.update(tile.bytes());
                }
            }
            out.push(digest.finalize().iter().map(|v| format!("{v:02x}")).collect());
            if let LayerContent::Group(group) = &layer.content {
                walk(&group.children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.layers, &mut out);
    assert_eq!(out.len(), 4);
    out
}
fn retained(before: &Document, current: &Document) {
    assert_eq!(payload(before), payload(current));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.sample_encoding, before.sample_encoding);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let a = before.xcf_state.as_ref().unwrap();
    let b = current.xcf_state.as_ref().unwrap();
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.filter_stacks, b.filter_stacks);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.current_filter_output_precision, b.current_filter_output_precision);
    for id in ids(current) {
        let layer = current.layer(id).unwrap();
        let item = b.items.iter().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap();
        assert_eq!(item.name, layer.name);
        assert_eq!(item.visible, layer.visible);
        let old = a.items.iter().find(|old| old.item == item.item).unwrap();
        assert_eq!(item.raster_bounds, old.raster_bounds);
        assert_eq!(item.composition, old.composition);
        assert_eq!(item.parent, old.parent);
    }
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-group-controls-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN:24 genuine group sources; four-layer compound metadata, cross-target gesture/history, synchronous/authorizer guards, optional mask and native continuation"]
fn group_metadata_and_mask_commands_preserve_payloads_and_transport_policy() {
    for (root, row) in cases() {
        let original_path = root.join(row["source"]["file"].as_str().unwrap());
        let original = std::fs::read(&original_path).unwrap();
        let dir = Directory::new();
        let source_path = dir.0.join("original.xcf");
        std::fs::write(&source_path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&source_path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        let targets = ids(&before);
        let group = targets[0];
        let backdrop = targets[3];
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        assert!(
            matches!(h.command_start("layer.setProps",json!({"layer":group.0,"visible":false}),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
        );
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("group controls denied".into())));
        assert!(h.command_run("layer.setProps", json!({"layer":group.0,"name":"Denied"})).unwrap_err().to_string().contains("group controls denied"));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        h.session.authorize = None;
        let past = h.session.active().unwrap().history.past_len();
        assert_eq!(
            h.command_run("layer.setProps", json!({"layer":group.0,"visible":!before.layer(group).unwrap().visible,"coalesce":"group-gesture"})).unwrap(),
            Value::Null
        );
        for (index, id) in targets.iter().enumerate() {
            assert_eq!(h.command_run("layer.setProps",json!({"layer":id.0,"name":format!("Group host {index}"),"visible":index==3,"opacity":if index==3 {0.375} else {0.5},"coalesce":"group-gesture"})).unwrap(),Value::Null);
        }
        let changed = h.session.active().unwrap().doc.clone();
        retained(&before, &changed);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        for (index, id) in targets.iter().enumerate() {
            let layer = changed.layer(*id).unwrap();
            assert_eq!(layer.name, format!("Group host {index}"));
            assert_eq!(layer.visible, index == 3);
            assert_eq!(layer.opacity.to_bits(), if index == 3 { 0.375f32.to_bits() } else { 0.5f32.to_bits() });
        }
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let old = before.layer(backdrop).unwrap();
        h.command_run(
            "layer.setProps",
            json!({"layer":backdrop.0,"name":old.name,"visible":old.visible,"opacity":f64::from(old.opacity),"coalesce":"group-gesture"}),
        )
        .unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&changed));
        let mask = changed.xcf_state.as_ref().unwrap().items.iter().find_map(|item| match (&item.item, &item.mask) {
            (photocraft_doc::item_identity::ItemRef::Mask(id), Some(mask)) if mask.owner == photocraft_doc::item_identity::ItemRef::Layer(group) => {
                Some((*id, mask.enabled))
            }
            _ => None,
        });
        if let Some((mask, enabled)) = mask {
            let params = json!({"maskId":mask.0.to_string(),"enabled":!enabled});
            let revision = h.session.active().unwrap().revision;
            let journal = h.session.journal.len();
            assert!(
                matches!(h.command_start("xcf.mask.enabled.set",params.clone(),false),Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous"))
            );
            assert_eq!(h.session.active().unwrap().revision, revision);
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("group mask denied".into())));
            assert!(h.command_run("xcf.mask.enabled.set", params.clone()).unwrap_err().to_string().contains("group mask denied"));
            h.session.authorize = None;
            assert_eq!(
                h.command_run("xcf.mask.enabled.set", params).unwrap(),
                json!({"maskId":mask.0.to_string(),"layerId":group.0.to_string(),"enabled":!enabled,"changed":true})
            );
            assert_eq!(h.session.journal.len(), journal);
            retained(&before, &h.session.active().unwrap().doc);
            h.command_run("edit.undo", json!({})).unwrap();
            let history = h.session.active().unwrap().history.clone();
            assert_eq!(h.command_run("xcf.mask.enabled.set", json!({"maskId":mask.0.to_string(),"enabled":enabled})).unwrap()["changed"], false);
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            h.command_run("edit.redo", json!({})).unwrap();
            h.command_run("xcf.mask.enabled.set", json!({"maskId":mask.0.to_string(),"enabled":false})).unwrap();
            assert_eq!(h.session.journal.len(), journal);
            assert!(!h.session.active().unwrap().doc.layer(group).unwrap().mask.as_ref().unwrap().enabled);
        }
        let edited = h.session.active().unwrap().doc.clone();
        retained(&before, &edited);
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], if mask.is_some() { 5 } else { 4 });
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        assert!(h.save_owned_native_active(Some(&source_path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Group controls.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        assert!(h.session.active().unwrap().source_read_only);
        let saved_name = h.session.active().unwrap().doc.name.clone();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let state = reopened.session.active().unwrap();
        assert!(!state.source_read_only);
        assert_eq!(state.doc.name, saved_name);
        retained(&before, &state.doc);
        assert_eq!(state.doc.xcf_state, edited.xcf_state);
        for id in &targets {
            let expected = edited.layer(*id).unwrap();
            let actual = state.doc.layer(*id).unwrap();
            assert_eq!((&actual.name, actual.visible, actual.opacity.to_bits()), (&expected.name, expected.visible, expected.opacity.to_bits()));
        }
        for id in targets.iter().take(3) {
            reopened.command_run("layer.setProps", json!({"layer":id.0,"visible":true})).unwrap();
        }
        if let Some((mask, _)) = mask {
            assert!(reopened.command_run("xcf.mask.enabled.set", json!({"maskId":mask.0.to_string(),"enabled":true})).unwrap()["changed"].as_bool().unwrap());
        }
        retained(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(&source_path).unwrap(), original);
        assert_eq!(std::fs::read(original_path).unwrap(), original);
    }
}
