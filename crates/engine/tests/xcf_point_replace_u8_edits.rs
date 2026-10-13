#![cfg(feature = "corpus")]
//! Real U8 seven-family point edits retain native source and canvas masks.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
#[test]
#[ignore = "local unpublished independently authored seven-family U8 Linear and Nonlinear originals"]
fn original_point_replace_u8_edits_native_undo_and_atomic_rejection() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-replace-u8-evidence");
    let manifest = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "e472b64e19df16ce7fa5999785f9859c4a553c233bb341c47d6aa0c98d1816b5");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    let obs = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&obs), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap()["sha256"]);
    let obs: Value = serde_json::from_slice(&obs).unwrap();
    let mut count = 0;
    for row in obs["records"].as_array().unwrap().iter().filter(|r| r["id"].as_str().unwrap().ends_with(".source")) {
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let source = photocraft_io::import("source.xcf", &bytes).unwrap().document;
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        for args in [
            json!({"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":f32::NAN.to_bits()}}}]}),
            json!({"arguments":[{"name":"black-level","value":{"Number":{"Float64Bits":"0x0000000000000000"}}}]}),
            json!({"arguments":[]}),
            json!({"unexpected":true}),
        ] {
            let mut params = args;
            params["filterId"] = json!(node.id.0.to_string());
            let mut s = Session::new();
            s.add_document(source.clone(), None).unwrap();
            assert!(s.execute("xcf.filter.set", params).is_err());
            assert_eq!(*s.active().unwrap().doc, source);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
        }
        if family.ends_with("small") {
            for mutation in 0..10 {
                let mut changed = source.clone();
                let owner = changed.layers[0].id;
                let state = changed.xcf_state.as_mut().unwrap();
                match mutation {
                    0 => state.filter_masks[0].surface.try_write_pixel_f64(7, 5, &[0.5]).unwrap(),
                    1 => state.filter_masks[0].bounds.x0 = -13,
                    2 => state.items.iter_mut().find(|item| item.mask.is_some()).unwrap().mask.as_mut().unwrap().offset = [-13, 13],
                    3 => state.filter_stacks[0].nodes[0].mask = None,
                    4 => state.filter_stacks[0].nodes[0].operation.schema_epoch = "unqualified".into(),
                    5 => state.items.iter_mut().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().locks.content = true,
                    6 => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                    7 => state.filter_masks[0].owner = photocraft_doc::item_identity::FilterId(u64::MAX),
                    8 => state.items.iter_mut().find(|item| item.mask.is_some()).unwrap().mask.as_mut().unwrap().shown = true,
                    _ => state.items.iter_mut().find(|item| item.mask.is_some()).unwrap().visible = false,
                }
                let mut rejected = Session::new();
                rejected.add_document(changed.clone(), None).unwrap();
                assert!(rejected.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false})).is_err(), "{family} {mutation}");
                assert_eq!(*rejected.active().unwrap().doc, changed);
                assert_eq!(rejected.active().unwrap().history.past_len(), 0);
            }
        }
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":1.5f32.to_bits()}}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            )
            .is_err()
        );
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        for stage in ["edit", "disabled"] {
            let targetrow = obs["records"].as_array().unwrap().iter().find(|r| r["id"] == format!("{family}.{stage}")).unwrap();
            let targetbytes = fs::read(root.join(targetrow["file"].as_str().unwrap())).unwrap();
            assert_eq!(hash(&targetbytes), targetrow["sha256"]);
            let target = photocraft_io::import("reference.xcf", &targetbytes).unwrap().document;
            let targetnode = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut s = Session::new();
            s.add_document(source.clone(), None).unwrap();
            let params = if stage == "edit" {
                json!({"filterId":node.id.0.to_string(),"arguments":targetnode.arguments})
            } else {
                json!({"filterId":node.id.0.to_string(),"enabled":false})
            };
            s.execute(if stage == "edit" { "xcf.filter.set" } else { "xcf.filter.enable" }, params).unwrap();
            let edited = (*s.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, targetnode.arguments);
            assert_eq!(current.filter_stacks[0].nodes[0].enabled, targetnode.enabled);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
            );
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, source);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.active().unwrap().doc, edited);
            count += 1;
        }
        assert_eq!(fs::read(root.join(row["file"].as_str().unwrap())).unwrap(), bytes);
    }
    assert_eq!(count, 44);
}
