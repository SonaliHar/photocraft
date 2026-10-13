#![cfg(feature = "corpus")]
//! Actual GIMP NoHalo LinkLayers: current placement and independently observed matrices.
use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource, xcf_resources::ResourceKind};
use photocraft_engine::{
    Session,
    xcf_resources::{RecoveryPolicy, ServiceResourceBytes, recover_authorized_bytes},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn read(root: &std::path::Path, entry: &Value) -> Vec<u8> {
    let bytes = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), entry["sha256"].as_str().unwrap());
    bytes
}
fn bounds(native: &Value) -> photocraft_doc::Rect {
    let x = native["offset"][0].as_i64().unwrap() as i32;
    let y = native["offset"][1].as_i64().unwrap() as i32;
    photocraft_doc::Rect::new(x, y, x + native["size"][0].as_u64().unwrap() as i32, y + native["size"][1].as_u64().unwrap() as i32)
}
fn assert_native(doc: &photocraft_doc::Document, owner: photocraft_doc::LayerId, extent: photocraft_doc::Rect, oracle: &[u8], id: &str) {
    let actual = doc.layer(owner).unwrap().surface().unwrap().try_read_interleaved(extent).unwrap();
    assert_eq!(actual.len(), oracle.len());
    let maximum = actual.iter().zip(oracle).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(maximum <= 2, "{id} native maximum{maximum}/255");
    eprintln!("{id} geometry native maximum{maximum}/255");
}
#[test]
#[ignore = "requires private independently authored NoHalo LinkLayer packet; PHOTOCRAFT_XCF_NOHALO_LINK_PACKET"]
fn actual_nohalo_current_placement_and_transform_preserve_source_native_and_history() {
    let root = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_NOHALO_LINK_PACKET").unwrap());
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let records = packet["records"].as_array().unwrap();
    let mut count = 0;
    for record in records {
        let id = record["id"].as_str().unwrap();
        let target_id = if id.ends_with("upscale") { id.replace("upscale", "translation") } else { id.replace("translation", "upscale") };
        let target = records.iter().find(|target| target["id"] == target_id).unwrap();
        let material = read(&root, &record["resource"]);
        assert_eq!(material, read(&root, &target["resource"]), "independent matrix pair must use identical source bytes");
        assert_eq!(record["untransformed"]["offset"], target["untransformed"]["offset"]);
        let source_bytes = read(&root, &record["source"]);
        let original = photocraft_io::import("own-nohalo-geometry.xcf", &source_bytes).unwrap().document;
        let source = original.xcf_state.as_ref().unwrap().source_items.iter().find(|source| matches!(source.source, EditableSource::Linked { .. })).unwrap();
        let ItemRef::Layer(owner) = source.owner else { panic!("own linked owner") };
        let EditableSource::Linked { resource, .. } = source.source else { panic!("own linked source") };
        let mut session = Session::new();
        session.open_document(original.clone(), None).unwrap();
        recover_authorized_bytes(
            &mut session,
            resource,
            ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, material, true).unwrap(),
            RecoveryPolicy::AllowSubstitution,
            &Interrupt::NONE,
        )
        .unwrap();
        session.execute("xcf.link.refresh", json!({"layerId":owner.0.to_string()})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let placed_bounds = bounds(&record["fresh_edit"]);
        let params = json!({"layerId":owner.0.to_string(),"originPx":[placed_bounds.x0,placed_bounds.y0]});
        assert_eq!(session.execute("xcf.link.setPlacement", params.clone()).unwrap()["changed"], true);
        let placed = session.active().unwrap().doc.clone();
        assert_native(&placed, owner, placed_bounds, &read(&root, &record["fresh_edit"]["native"]), id);
        assert_eq!(session.execute("xcf.link.setPlacement", params).unwrap()["changed"], false);
        assert!(session.active().unwrap().doc.shares_state(&placed));
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&placed));
        let target_bounds = bounds(&target["fresh_source"]);
        let matrix: Vec<String> = target["matrix"].as_array().unwrap().iter().map(|value| format!("{:016x}", value.as_f64().unwrap().to_bits())).collect();
        let params = json!({"layerId":owner.0.to_string(),"matrixF64Bits":matrix,"interpolation":"nohalo","originPx":[target_bounds.x0,target_bounds.y0]});
        assert_eq!(session.execute("xcf.link.setTransform", params.clone()).unwrap()["changed"], true);
        let transformed = session.active().unwrap().doc.clone();
        assert_native(&transformed, owner, target_bounds, &read(&root, &target["fresh_source"]["native"]), &format!("{id}->{target_id}"));
        assert_eq!(session.execute("xcf.link.setTransform", params).unwrap()["changed"], false);
        assert!(session.active().unwrap().doc.shares_state(&transformed));
        let current_source = transformed.xcf_state.as_ref().unwrap().source_items.iter().find(|current| current.owner == source.owner).unwrap();
        assert_eq!(current_source.transform, source.transform);
        assert_eq!(current_source.cache, source.cache);
        assert_eq!(transformed.xcf_state.as_ref().unwrap().provenance, original.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(transformed.xcf_state.as_ref().unwrap().resources, before.xcf_state.as_ref().unwrap().resources);
        let archive = photocraft_format::save_to_bytes(&transformed, &Default::default()).unwrap();
        assert_eq!(photocraft_format::read_manifest(&archive).unwrap().minimum_reader_version, 7);
        assert_eq!(photocraft_format::load_from_bytes(&archive).unwrap(), *transformed);
        assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&placed));
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&transformed));
        assert_eq!(read(&root, &record["source"]), source_bytes);
        count += 1;
    }
    assert_eq!(count, 4);
}
