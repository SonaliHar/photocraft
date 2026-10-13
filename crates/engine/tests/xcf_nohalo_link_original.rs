#![cfg(feature = "corpus")]
//! Independent GIMP-authored LinkLayer import, not a synthesized linked state.
use photocraft_doc::{item_identity::ItemRef, xcf_items::EditableSource, xcf_resources::ResourceKind};
use photocraft_engine::{
    Session,
    xcf_resources::{RecoveryPolicy, ServiceResourceBytes, recover_authorized_bytes},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[test]
#[ignore = "requires isolated GIMP-authored modern NoHalo LinkLayer packet; set PHOTOCRAFT_XCF_NOHALO_LINK_PACKET"]
fn real_nohalo_link_import_substitution_refresh_native_and_history() {
    let root = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_XCF_NOHALO_LINK_PACKET").unwrap());
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let read = |entry: &Value| {
        let bytes = std::fs::read(root.join(entry["file"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), entry["sha256"].as_str().unwrap());
        bytes
    };
    let mut count = 0;
    for record in packet["records"].as_array().unwrap() {
        let bytes = read(&record["source"]);
        let resource_bytes = read(&record["resource"]);
        for (entry, native) in [(&record["source"], &record["fresh_source"]), (&record["edit"], &record["fresh_edit"])] {
            let source_bytes = read(entry);
            let parsed = photocraft_xcf::decode(&source_bytes, &Default::default()).unwrap();
            let drawable = &parsed.layers[0];
            assert_eq!(drawable.link.as_ref().unwrap().flags, 0, "default monitored/naming source bits must be retained");
            let flag_headers = source_bytes
                .windows(12)
                .enumerate()
                .filter(|(_, value)| value[..4] == [0, 0, 0, 48] && value[8..12] == [0, 0, 0, 0])
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            assert_eq!(flag_headers.len(), 1);
            let mut unknown_flags = source_bytes.clone();
            unknown_flags[flag_headers[0] + 8..flag_headers[0] + 12].copy_from_slice(&4u32.to_be_bytes());
            assert_eq!(photocraft_xcf::decode(&unknown_flags, &Default::default()).unwrap().layers[0].link.as_ref().unwrap().flags, 4);
            assert!(photocraft_io::import("own-unknown-link-flags.xcf", &unknown_flags).is_err());
            eprintln!(
                "{} version={} precision={:?} size={}x{} offset={:?} link={:?} transform={:?} mode={} composition={}/{}/{} unsupported={:?}",
                record["id"],
                parsed.version,
                parsed.precision,
                drawable.width,
                drawable.height,
                drawable.offset,
                drawable.link.as_ref().map(|link| (link.flags, link.width, link.height)),
                drawable.transform,
                drawable.mode,
                drawable.composite_mode,
                drawable.composite_space,
                drawable.blend_space,
                drawable.unsupported
            );
            // Import gets bytes only: the stored locator supplies no read authority.
            let imported = photocraft_io::import("own-nohalo-link.xcf", &source_bytes).unwrap();
            assert_eq!(imported.assessment.as_ref().unwrap().fidelity, photocraft_io::xcf_assessment::Fidelity::DependencyLimited);
            let original = imported.document;
            let state = original.xcf_state.as_ref().unwrap();
            let linked = state.source_items.iter().find(|entry| matches!(entry.source, EditableSource::Linked { .. })).unwrap();
            let ItemRef::Layer(owner) = linked.owner else { panic!("expected layer owner") };
            let EditableSource::Linked { resource, .. } = linked.source else { panic!("expected link") };
            assert_eq!(linked.transform.as_ref().unwrap().interpolation, 3);
            let requested = state.resources.iter().find(|value| value.id == resource).unwrap();
            assert!(requested.owned_bytes.is_none() && requested.resolution.is_none() && requested.content_hash.is_none());
            let bounds = state.items.iter().find(|item| item.item == linked.owner).unwrap().raster_bounds.unwrap();
            let oracle = read(&native["native"]);
            assert_eq!(original.layer(owner).unwrap().surface().unwrap().try_read_interleaved(bounds).unwrap(), oracle);
            let mut session = Session::new();
            session.open_document(original.clone(), None).unwrap();
            let snapshot = session.active().unwrap().doc.clone();
            assert!(
                recover_authorized_bytes(
                    &mut session,
                    resource,
                    ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, resource_bytes.clone(), true).unwrap(),
                    RecoveryPolicy::Exact,
                    &Interrupt::NONE
                )
                .is_err()
            );
            assert!(session.active().unwrap().doc.shares_state(&snapshot));
            recover_authorized_bytes(
                &mut session,
                resource,
                ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, resource_bytes.clone(), true).unwrap(),
                RecoveryPolicy::AllowSubstitution,
                &Interrupt::NONE,
            )
            .unwrap();
            let pending = session.active().unwrap().doc.clone();
            let mut locked = (*pending).clone();
            locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == linked.owner).unwrap().locks.content = true;
            let mut denied = Session::new();
            denied.open_document(locked.clone(), None).unwrap();
            assert!(denied.execute("xcf.link.refresh", json!({"layerId":owner.0.to_string()})).is_err());
            assert_eq!(*denied.active().unwrap().doc, locked);
            assert!(
                photocraft_engine::xcf_link_cmds::refresh_with_interrupt(
                    &mut session,
                    &json!({"layerId":owner.0.to_string()}),
                    &Interrupt::cancel_only(&|| true)
                )
                .is_err()
            );
            assert!(session.active().unwrap().doc.shares_state(&pending));
            session.execute("xcf.link.refresh", json!({"layerId":owner.0.to_string()})).unwrap();
            let current = session.active().unwrap().doc.clone();
            assert_eq!(session.execute("xcf.link.refresh", json!({"layerId":owner.0.to_string()})).unwrap()["refreshed"], false);
            assert!(session.active().unwrap().doc.shares_state(&current));
            assert_eq!(session.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
            assert_eq!(session.execute("xcf.resource.inspect", json!({"resource":resource.0.to_string()})).unwrap()["resources"][0]["activated"], true);
            let pixels = current.layer(owner).unwrap().surface().unwrap().try_read_interleaved(bounds).unwrap();
            assert_eq!(pixels.len(), oracle.len());
            let maximum_error = pixels.iter().zip(&oracle).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(maximum_error <= 2, "{} {} native {maximum_error}/255", record["id"], entry["file"]);
            eprintln!("{} {} refreshed native maximum {maximum_error}/255", record["id"], entry["file"]);
            let current_source = current.xcf_state.as_ref().unwrap().source_items.iter().find(|item| item.owner == linked.owner).unwrap();
            assert_eq!(current_source.cache, linked.cache);
            assert_eq!(current_source.transform, linked.transform);
            assert_eq!(current.xcf_state.as_ref().unwrap().provenance, state.provenance);
            let current_resource = current.xcf_state.as_ref().unwrap().resources.iter().find(|value| value.id == resource).unwrap();
            assert_eq!(current_resource.locator, requested.locator);
            assert_eq!(current_resource.content_hash, requested.content_hash);
            let saved = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened, *current);
            assert_eq!(
                photocraft_io::xcf_assessment::assess_xcf_document(&reopened, &Interrupt::NONE).unwrap().unwrap().fidelity,
                photocraft_io::xcf_assessment::Fidelity::Reduced
            );
            let mut assigned = Session::new();
            assigned.open_document((*current).clone(), None).unwrap();
            assigned.execute("edit.assignProfile", json!({"profile":"linear-srgb"})).unwrap();
            let profile_pending = assigned.active().unwrap().doc.clone();
            assert!(assigned.execute("xcf.assess", json!({})).is_err());
            assigned.execute("xcf.link.refresh", json!({"layerId":owner.0.to_string()})).unwrap();
            let profile_current = assigned.active().unwrap().doc.clone();
            assert_eq!(assigned.execute("xcf.assess", json!({})).unwrap()["fidelity"], "reduced");
            assert_ne!(profile_current.layer(owner).unwrap().surface(), current.layer(owner).unwrap().surface());
            assert_eq!(profile_current.xcf_state.as_ref().unwrap().source_items, current.xcf_state.as_ref().unwrap().source_items);
            assert_eq!(profile_current.xcf_state.as_ref().unwrap().provenance, current.xcf_state.as_ref().unwrap().provenance);
            let saved = photocraft_format::save_to_bytes(&profile_current, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), *profile_current);
            assigned.execute("edit.undo", json!({})).unwrap();
            assert!(assigned.active().unwrap().doc.shares_state(&profile_pending));
            assigned.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*assigned.active().unwrap().doc, *current);
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&pending));
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&snapshot));
            session.execute("edit.redo", json!({})).unwrap();
            session.execute("edit.redo", json!({})).unwrap();
            assert!(session.active().unwrap().doc.shares_state(&current));
            assert_eq!(read(entry), source_bytes);
        }
        assert_eq!(read(&record["source"]), bytes);
        count += 1;
    }
    assert_eq!(count, 4);
}
