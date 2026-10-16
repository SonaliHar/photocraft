use serde_json::json;

use super::*;

/// Writes test images of the given sizes (each a flat colour) and returns their paths.
fn images(name: &str, sizes: &[(u32, u32)]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-figure-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = Vec::new();
    for (i, (w, h)) in sizes.iter().enumerate() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "background": "#808080"})).unwrap();
        let path = dir.join(format!("scan{i}.png"));
        let out = photocraft_io::export(&s.active().unwrap().doc, &path.to_string_lossy(), &photocraft_io::ExportOptions::default()).unwrap();
        std::fs::write(&path, out.bytes).unwrap();
        paths.push(path.to_string_lossy().into_owned());
    }
    (dir, paths)
}

#[test]
fn rows_match_heights_fill_the_width_and_label_each_panel() {
    // Two CT slices of different sizes and a wide reconstruction.
    let (dir, files) = images("rows", &[(512, 512), (400, 300), (800, 400)]);
    let mut s = Session::new();
    let r = s.execute(CMD, json!({"files": files, "columns": 2, "width": "double", "gutterMm": 1.0})).unwrap();
    // 175 mm at 300 dpi; a 1 mm gutter is 12 px.
    assert_eq!(r["width"].as_u64(), Some(2067), "{r}");
    let rects: Vec<[u64; 4]> = r["panels"].as_array().unwrap().iter().map(|p| serde_json::from_value(p["rect"].clone()).unwrap()).collect();
    assert_eq!(rects[0][3], rects[1][3], "same height in a row");
    assert_eq!(rects[1][0] + rects[1][2], 2067, "the row ends at the right edge");
    assert_eq!(rects[1][0], rects[0][2] + 12, "one gutter between them");
    assert_eq!(rects[2][1], rects[0][3] + 12, "the second row starts after one gutter");
    assert_eq!(rects[2][2], 2067, "a single image fills its row");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.resolution_dpi, 300.0);
    let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
    for want in ["scan0", "Label a", "scan1", "Label b", "scan2", "Label c"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    let label = doc.layers.iter().find(|l| l.name == "Label a").unwrap();
    assert!(matches!(label.content, LayerContent::Text(_)), "editable type");
    assert!(!label.effects.items.is_empty(), "outlined");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn grid_layout_fits_images_in_equal_cells() {
    let (dir, files) = images("grid", &[(300, 300), (300, 600)]);
    let mut s = Session::new();
    let r = s
        .execute(
            CMD,
            json!({"files": files, "layout": "grid", "width": "custom", "widthMm": 100, "gutterMm": 0, "labels": "(A)", "labelPosition": "bottomRight"}),
        )
        .unwrap();
    let rects: Vec<[u64; 4]> = r["panels"].as_array().unwrap().iter().map(|p| serde_json::from_value(p["rect"].clone()).unwrap()).collect();
    // Cells are square (the widest image's shape); the tall image is centred inside its cell.
    assert_eq!(rects[0][2], rects[0][3]);
    assert_eq!(rects[1][3], rects[0][3], "fitted by height");
    assert!(rects[1][2] < rects[1][3]);
    assert!(s.active().unwrap().doc.layers.iter().any(|l| l.name == "Label (B)"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn labels_count_past_z() {
    assert_eq!(label("abc", 0), "a");
    assert_eq!(label("ABC", 25), "Z");
    assert_eq!(label("abc", 26), "aa");
    assert_eq!(label("(a)", 1), "(b)");
    assert_eq!(label("A)", 2), "C)");
}

#[test]
fn bad_params_are_errors_and_open_nothing() {
    let (dir, files) = images("bad", &[(10, 10)]);
    let mut s = Session::new();
    for p in [
        json!({}),
        json!({"files": []}),
        json!({"files": [1, 2]}),
        json!({"files": files, "columns": 0}),
        json!({"files": files, "layout": "spiral"}),
        json!({"files": files, "width": "triple"}),
        json!({"files": files, "width": "custom", "widthMm": 1e9}),
        json!({"files": files, "dpi": 1e9}),
        json!({"files": files, "gutterColor": "white"}),
        json!({"files": files, "labels": "123"}),
        json!({"files": files, "labelPosition": "middle"}),
        json!({"files": files, "labelOutline": "yes"}),
        json!({"files": ["/no/such/image.png"]}),
    ] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        assert!(s.active().is_none(), "no document opened for {p}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
