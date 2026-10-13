#![cfg(feature = "corpus")]
//! Current native precision guards are checked before an edit/history transaction.
use photocraft_engine::Session;
use serde_json::{Value, json};
#[test]
#[ignore = "local unpublished Component geometry current native controls"]
fn geometry_current_nonfinite_alpha_and_outside_pixels_are_atomic_errors() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-geometry-evidence");
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|row| row["enabled"] == true && row["invert"] == false && row["linear"] == false) {
        let bytes = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
        let source = photocraft_io::import("geometry.xcf", &bytes).unwrap().document;
        let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let bounds = source
            .xcf_state
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(source.layers[0].id))
            .unwrap()
            .raster_bounds
            .unwrap();
        for (color, alpha) in [(f64::NAN, 0.5), (f64::INFINITY, 0.5), (1e30, 0.5), (0.5, 1.00001)] {
            let mut changed = source.clone();
            let pixel = if changed.mode == photocraft_doc::ColorMode::Grayscale { vec![color, alpha] } else { vec![color, 0.5, 0.75, alpha] };
            changed.layers[0].surface_mut().unwrap().try_write_pixel_f64(bounds.x0, bounds.y0, &pixel).unwrap();
            let raw = changed.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
            let mut failed = Session::new();
            failed.add_document(changed, None).unwrap();
            assert!(failed.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err(), "{} {color}/{alpha}", row["id"]);
            assert_eq!(failed.active().unwrap().doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), raw);
            assert_eq!(failed.active().unwrap().history.past_len(), 0);
            count += 1;
        }
        let mut changed = source.clone();
        let pixel = if changed.mode == photocraft_doc::ColorMode::Grayscale { vec![0.5, 0.5] } else { vec![0.25, 0.5, 0.75, 0.5] };
        changed.layers[0].surface_mut().unwrap().try_write_pixel_f64(bounds.x0 - 1, bounds.y0, &pixel).unwrap();
        let mut failed = Session::new();
        failed.add_document(changed.clone(), None).unwrap();
        assert!(failed.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).is_err());
        assert_eq!(*failed.active().unwrap().doc, changed);
        assert_eq!(failed.active().unwrap().history.past_len(), 0);
        count += 1;
    }
    assert_eq!(count, 220);
    println!("220 hostile native/nonfinite/alpha/outside cases rejected atomically");
}
