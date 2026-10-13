#![cfg(feature = "corpus")]
//! Registered edits compared with independent Alpha Clip source/edit/activation/promotion states.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn import(root: &Path, row: &Value) -> photocraft_doc::Document {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("alpha-clip.xcf", &bytes).unwrap().document
}
fn arguments(doc: &photocraft_doc::Document) -> Vec<Value> {
    doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().map(|a| json!({"name":a.name,"value":a.value})).collect()
}
fn render(doc: &photocraft_doc::Document) -> Vec<[f64; 4]> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap().px
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn registered_alpha_clip_edits_native_history_noop_and_half_promotion() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-alpha-clip-evidence");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), "527bc502322bcbe33d0ecb7a659363be85d0ce99a46e1fc552a77d0c77cef38d");
    let obs: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = obs["records"].as_array().unwrap();
    let mut count = 0;
    for row in rows.iter().filter(|r| r["role"] == "source") {
        let source = import(&root, row);
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let mut unchanged = Session::new();
        unchanged.add_document(source.clone(), None).unwrap();
        unchanged.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments(&source)})).unwrap();
        assert_eq!(*unchanged.active().unwrap().doc, source);
        assert_eq!(unchanged.active().unwrap().history.past_len(), 0);
        for role in ["edit", "disabled", "neither", "low-only", "high-only", "promoted-current"] {
            if role == "promoted-current" && !row["precision"].as_str().unwrap().starts_with("HALF_") {
                continue;
            }
            let Some(target_row) = rows.iter().find(|r| {
                r["model"] == row["model"]
                    && (r["precision"] == row["precision"]
                        || (role == "promoted-current"
                            && row["precision"].as_str().unwrap().starts_with("HALF_")
                            && r["precision"].as_str().unwrap() == row["precision"].as_str().unwrap().replace("HALF_", "FLOAT_")))
                    && r["role"] == role
            }) else {
                continue;
            };
            let target = import(&root, target_row);
            let mut s = Session::new();
            s.add_document(source.clone(), None).unwrap();
            if role == "promoted-current" {
                s.execute("image.mode.bits32", json!({})).unwrap();
                assert_eq!(s.active().unwrap().doc.layers, source.layers);
                assert_eq!(
                    s.active().unwrap().doc.xcf_state.as_ref().unwrap().current_filter_output_precision,
                    photocraft_doc::FilterOutputPrecision::Canonical
                );
            }
            if role == "disabled" {
                s.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
            } else {
                s.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":arguments(&target)})).unwrap();
            }
            let edited = (*s.active().unwrap().doc).clone();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
            assert_eq!(render(&edited), render(&target));
            assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap()).unwrap(), edited);
            s.execute("edit.undo", json!({})).unwrap();
            if role == "promoted-current" {
                s.execute("edit.undo", json!({})).unwrap();
            }
            assert_eq!(*s.active().unwrap().doc, source);
            s.execute("edit.redo", json!({})).unwrap();
            if role == "promoted-current" {
                s.execute("edit.redo", json!({})).unwrap();
            }
            assert_eq!(*s.active().unwrap().doc, edited);
            count += 1;
        }
        for params in [
            json!([{"name":"clip-low","value":{"Number":{"Float32Bits":0}}}]),
            json!([{"name":"low-limit","value":{"Number":{"Float32Bits":f32::NAN.to_bits()}}}]),
            json!([{"name":"low-limit","value":{"Number":{"Float32Bits":(0.9f32).to_bits()}}}]),
            json!([{"name":"high-limit","value":{"Number":{"Float32Bits":(-0.1f32).to_bits()}}}]),
            json!([{"name":"clip-low","value":{"Bool":false}},{"name":"clip-low","value":{"Bool":true}}]),
            json!([{"name":"unknown","value":{"Bool":false}}]),
        ] {
            let mut rejected = Session::new();
            rejected.add_document(source.clone(), None).unwrap();
            assert!(rejected.execute("xcf.filter.set", json!({"filterId":id.0.to_string(),"arguments":params})).is_err());
            assert_eq!(*rejected.active().unwrap().doc, source);
            assert_eq!(rejected.active().unwrap().history.past_len(), 0);
        }
        for mutation in 0..8 {
            let mut changed = source.clone();
            let owner = changed.layers[0].id;
            match mutation {
                0 => changed.size.width = 9,
                1 => {
                    changed
                        .xcf_state
                        .as_mut()
                        .unwrap()
                        .items
                        .iter_mut()
                        .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(owner))
                        .unwrap()
                        .raster_bounds = Some(photocraft_geom::Rect::from_xywh(1, 0, 8, 6))
                }
                2 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.stored_version = "0:1".into(),
                3 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "other".into(),
                4 => changed.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_pixel_f64(7, 5, &[0.5]).unwrap(),
                5 => changed.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
                6 => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                _ => changed.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = None,
            };
            let mut failed = Session::new();
            failed.add_document(changed.clone(), None).unwrap();
            assert!(failed.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{} {mutation}", row["id"]);
            assert_eq!(*failed.active().unwrap().doc, changed);
            assert_eq!(failed.active().unwrap().history.past_len(), 0);
        }
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":id.0.to_string(),"arguments":[{"name":"clip-low","value":{"Bool":false}}]}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            )
            .is_err()
        );
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
    }
    assert_eq!(count, 96);
    println!("96 registered independent AlphaClip edits/activation/toggles/promotion/native/history;216 malformed controls and36cancel");
}
