//! UNRUN census-only harness values; no original-source or paid payload claim.
use super::*;
use photocraft_doc::{Group, Layer, item_identity::ItemRef, xcf_state::ItemState};

fn fixture() -> Document {
    let mut doc = Document::new("group census only", Size::new(17, 13), ColorMode::Rgb, SampleType::F32);
    let raster = |name| Layer::new(name, LayerContent::Raster(Surface::new(doc.pixel_format())));
    let bottom = raster("bottom");
    let children = vec![raster("child one"), raster("child two")];
    let top = Layer::new("group", LayerContent::Group(Group { children, expanded: true, artboard: None }));
    let mut state = photocraft_doc::XcfState::default();
    let LayerContent::Group(group) = &top.content else { panic!("fixture") };
    for layer in std::iter::once(&bottom).chain(std::iter::once(&top)).chain(group.children.iter()) {
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(doc.bounds()),
            mask: None,
            composition: None,
        });
    }
    doc.layers = vec![bottom, top];
    doc.xcf_state = Some(state);
    doc
}
fn limits() -> photocraft_xcf::ResourceLimits {
    photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()
}
#[test]
fn all_four_metadata_records_and_child_vector_capacity_but_only_three_stores() {
    let mut doc = fixture();
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let first = measure_isolated_normal_group(&doc, &budget, &Interrupt::NONE).unwrap();
    assert_eq!(first.layers, 3);
    assert!(measure(&doc, &budget, &Interrupt::NONE).is_err());
    assert!(measure_point_storage(&doc, &budget, &Interrupt::NONE).is_err());
    let LayerContent::Group(group) = &mut doc.layers[1].content else { panic!("fixture") };
    let old_capacity = group.children.capacity();
    group.children.reserve_exact(9);
    let extra = (group.children.capacity() - old_capacity) * size_of::<Layer>();
    let before_name = group.children[0].name.capacity();
    group.children[0].name.reserve_exact(73);
    let extra_name = group.children[0].name.capacity() - before_name;
    let next = measure_isolated_normal_group(&doc, &budget, &Interrupt::NONE).unwrap();
    assert_eq!(next.layers, 3);
    assert_eq!(next.model - first.model, (extra + extra_name) as u64);
}
#[test]
fn optional_group_mask_map_and_header_are_counted_without_becoming_native_store() {
    let mut doc = fixture();
    let budget = photocraft_xcf::Budget::new(limits(), None);
    let before = measure_isolated_normal_group(&doc, &budget, &Interrupt::NONE).unwrap();
    let mut surface = Surface::new(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    surface.try_write_pixel_f64(0, 0, &[-0.0]).unwrap();
    doc.layers[1].mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1.0, feather: 0.0 });
    let item = doc.xcf_state.as_ref().unwrap().items[0].clone();
    doc.xcf_state.as_mut().unwrap().items.push(item); // census cardinality only, not canonical proof
    let after = measure_isolated_normal_group(&doc, &budget, &Interrupt::NONE).unwrap();
    assert_eq!(after.layers, 3);
    assert!(after.model >= before.model + (size_of::<LayerMask>() + size_of::<Surface>() + 128 + 256 + 8) as u64);
    assert_eq!(after.payload, before.payload); // native mask bytes stay in genuine Sink receipts
}
#[test]
fn wrong_shapes_nested_content_and_control_fail_without_expanding_other_entries() {
    let budget = photocraft_xcf::Budget::new(limits(), None);
    for fault in 0..4 {
        let mut doc = fixture();
        let LayerContent::Group(group) = &mut doc.layers[1].content else { panic!("fixture") };
        match fault {
            0 => {
                group.children.pop();
            }
            1 => group.children[0].content = LayerContent::Group(Group { children: Vec::new(), expanded: false, artboard: None }),
            2 => {
                group.children[0].mask = Some(LayerMask {
                    surface: Arc::new(Surface::new(photocraft_doc::PixelFormat::GRAY8)),
                    enabled: true,
                    linked: true,
                    density: 1.0,
                    feather: 0.0,
                })
            }
            _ => {
                doc.layers.pop();
            }
        }
        assert!(measure_isolated_normal_group(&doc, &budget, &Interrupt::NONE).is_err());
    }
    let doc = fixture();
    let stop = Interrupt::cancel_only(&|| true);
    assert!(matches!(measure_isolated_normal_group(&doc, &budget, &stop), Err(IoError::Cancelled)));
    let limits = limits().lowered(&[("sampleWorkUnits", 0)]).unwrap();
    let denied = photocraft_xcf::Budget::new(limits, None);
    assert!(measure_isolated_normal_group(&doc, &denied, &Interrupt::NONE).is_err());
}
