use super::*;
use photocraft_doc::{XcfState, item_identity::*, xcf_filters::*, xcf_items::*};

fn raw() -> FilterArgument {
    FilterArgument {
        name: "raw".into(),
        value: FilterValue::StoredColor(ExactColorStorage { layout: "HSL float".into(), samples: vec![0; 12], opaque_tail: vec![] }),
    }
}
fn document(args: Vec<FilterArgument>) -> Document {
    let mut doc =
        Document::with_background("scanner", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let mut state = XcfState::default();
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(doc.layers[0].id),
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(19),
            operation: OperationIdentity { name: "own:storage".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
            source_display_name: None,
            source_icon: None,
            arguments: args,
            enabled: false,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    doc.xcf_state = Some(state);
    doc
}
#[test]
fn legacy_argument_boundary_does_not_charge_container_overhead_again() {
    let args = (0..50_000)
        .map(|i| FilterArgument {
            name: format!("a{i}"),
            value: FilterValue::Config(vec![FilterArgument { name: "b".into(), value: FilterValue::Bool(false) }]),
        })
        .collect();
    let doc = document(args);
    assert!(!requires_reader_v9_state_with(&doc, photocraft_raster::Interrupt::NONE).unwrap());
}
#[test]
fn all_three_editable_source_configurations_require_nine() {
    let sources = [
        EditableSource::Text {
            content: "own".into(),
            markup: None,
            font: ResourceId(1),
            direction: 0,
            source_modified: false,
            configuration: vec![raw()],
            current_origin: None,
        },
        EditableSource::Vector { path: PathId(1), source_modified: false, configuration: vec![raw()] },
        EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![raw()], current: None },
    ];
    for source in sources {
        let mut doc = document(vec![]);
        let owner = ItemRef::Layer(doc.layers[0].id);
        doc.xcf_state.as_mut().unwrap().source_items.push(SourceItem { owner, source, transform: None, cache: None });
        // Feature census is intentionally independent of semantic/resource validation.
        assert!(requires_reader_v9_state_with(&doc, photocraft_raster::Interrupt::NONE).unwrap());
    }
}
#[test]
fn scan_cancellation_and_depth_are_named_errors_without_mutation() {
    let mut argument = raw();
    for _ in 0..33 {
        argument = FilterArgument { name: "nested".into(), value: FilterValue::Config(vec![argument]) };
    }
    let doc = document(vec![argument]);
    let before = doc.clone();
    assert!(matches!(requires_reader_v9_state_with(&doc, photocraft_raster::Interrupt::NONE), Err(FormatError::LimitExceeded(_))));
    assert!(matches!(requires_reader_v9_state_with(&doc, photocraft_raster::Interrupt::cancel_only(&|| true)), Err(FormatError::Cancelled)));
    assert_eq!(doc, before);
}
