//! Investigation against the pinned ag-psd smart-filters fixture.
use photocraft_doc::LayerContent;

#[test]
#[ignore = "requires PHOTOCRAFT_SMART_FILTER_FIXTURE pointing to the pinned ag-psd fixture"]
fn reported_smart_filters_are_live_on_import() {
    let path = std::env::var("PHOTOCRAFT_SMART_FILTER_FIXTURE").expect("fixture path");
    let bytes = std::fs::read(path).expect("fixture bytes");
    let imported = photocraft_io::import("smart-filters.psd", &bytes).expect("valid pinned PSD");
    let filters: Vec<_> = imported
        .document
        .walk()
        .into_iter()
        .flat_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(sm) => sm.smart_filters.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert!(!filters.is_empty(), "fixture must contain smart filters");
    for f in &filters {
        println!("filter: {} id={:?} name={:?}", f.command, f.params.get("filterID"), f.params.get("name"));
    }
    let command = "filter.sharpen.sharpen";
    assert!(filters.iter().any(|f| f.command == command), "reported smart filter remains unsupported: {command}");
}
