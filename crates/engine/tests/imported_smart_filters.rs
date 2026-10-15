//! Imported smart filters remain active when replacing their source (#6267).
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{Session, smart_cmds::replace_contents_bytes};
use serde_json::json;

fn pattern(depth: u64, seed: i32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("pattern", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let v = 0.2 + ((x * 7 + y * 11 + seed) % 5) as f32 * 0.15;
                surface.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn imported_sharpen_applies_after_replacing_contents() {
    for depth in [8, 16, 32] {
        {
            let command = "filter.sharpen.sharpen";
            let params = json!({});
            let mut direct = pattern(depth, 0);
            let original_id = LayerId(direct.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap());
            direct.execute(command, params).unwrap();
            let exported = photocraft_io::export(&direct.active().unwrap().doc, "before.psd", &Default::default()).unwrap();
            assert!(exported.warnings.is_empty(), "{}", exported.warnings.join("; "));
            let doc = photocraft_io::import("before.psd", &exported.bytes).unwrap().document;
            let imported_id = doc
                .walk()
                .into_iter()
                .find_map(|(_, _, layer)| match &layer.content {
                    LayerContent::Smart(sm) => {
                        assert_eq!(sm.smart_filters.len(), 1);
                        assert_eq!(sm.smart_filters[0].command, command);
                        Some(layer.id)
                    }
                    _ => None,
                })
                .unwrap();
            let mut imported = Session::new();
            imported.add_document(doc, None);
            let replacement = pattern(depth, 2);
            let bytes = photocraft_io::export(&replacement.active().unwrap().doc, "replacement.png", &Default::default()).unwrap().bytes;
            replace_contents_bytes(&mut direct, original_id, "replacement.png", bytes.clone()).unwrap();
            replace_contents_bytes(&mut imported, imported_id, "replacement.png", bytes).unwrap();
            let expected = photocraft_compose::flatten(&direct.active().unwrap().doc).px;
            let actual = photocraft_compose::flatten(&imported.active().unwrap().doc).px;
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(&expected) {
                assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.005), "imported {command} differs after replacement at depth {depth}");
            }
            imported.execute("layer.smartFilter.disableSmartFilters", json!({"layer": imported_id.0, "enabled": false})).unwrap();
            let unfiltered = photocraft_compose::flatten(&imported.active().unwrap().doc).px;
            assert!(
                actual.iter().zip(&unfiltered).any(|(a, b)| (a[0] - b[0]).abs() > 0.01),
                "{command} must respect its settings on replacement pixels at depth {depth}"
            );
        }
    }
}
