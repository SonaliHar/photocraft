#[path = "video_options.rs"]
mod packet;
use photocraft_raster::Interrupt;
fn admitted(file: &photocraft_xcf::XcfFile) -> Result<bool, crate::IoError> {
    Ok(matches!(
        crate::xcf_point_replace_source::classify(file, &Interrupt::NONE, None)?,
        Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::VideoDegradation))
    ))
}
#[test]
#[ignore = "Video options raw effect layout stays within the witnessed source schema"]
fn options_raw_property_layout_is_refused_when_extended() {
    let observations = packet::observations().unwrap();
    let row = &observations["records"][0];
    let file = photocraft_xcf::decode(&packet::blob(&packet::root(), row).unwrap(), &Default::default()).unwrap();
    eprintln!(
        "actual new effect raw tags/declared widths: {:?}",
        file.layers[0].effects[0].properties.iter().map(|p| (p.tag, p.declared_bytes)).collect::<Vec<_>>()
    );
    for mutation in 0..3 {
        let mut bad = file.clone();
        let effect = &mut bad.layers[0].effects[0];
        match mutation {
            0 => effect.properties.push(photocraft_xcf::RawProperty { tag: 999, offset: 0, declared_bytes: 4, payload: vec![0; 4] }),
            1 => effect.properties.push(effect.properties[0].clone()),
            2 => effect.properties[0].declared_bytes = 5,
            _ => panic!("unexpected mutation ordinal"),
        }
        assert!(!admitted(&bad).unwrap(), "raw layout mutation{mutation}");
    }
}
#[test]
#[ignore = "Video options masks must carry only the witnessed native zero raster"]
fn options_mask_nonraster_metadata_is_refused() {
    let observations = packet::observations().unwrap();
    let row = &observations["records"][0];
    let file = photocraft_xcf::decode(&packet::blob(&packet::root(), row).unwrap(), &Default::default()).unwrap();
    for mutation in 0..8 {
        let mut bad = file.clone();
        let mask = bad.layers[0].effects[0].mask.as_mut().unwrap();
        match mutation {
            0 => mask.path = vec![0],
            1 => mask.floating_attachment = Some(1),
            2 => mask.mask = Some(Box::new((**mask).clone())),
            3 => mask.link = Some(photocraft_xcf::LinkedLayer { flags: 0, path: "inert-never-read.png".into(), width: 16, height: 16 }),
            4 => mask.transform = Some(photocraft_xcf::LayerTransform { offset: (0, 0), interpolation: 0, matrix: [1., 0., 0., 0., 1., 0., 0., 0., 1.] }),
            5 => {
                mask.text = Some(photocraft_xcf::TextLayer {
                    parasite_flags: 0,
                    raw: vec![],
                    text: None,
                    markup: None,
                    font: None,
                    configuration: vec![],
                    unknown_fields: vec![],
                    content: String::new(),
                    runs: vec![],
                    styles: vec![],
                    layout: Default::default(),
                })
            }
            6 => {
                let fill = photocraft_xcf::VectorFill { style: 0, antialias: false, color: None, pattern: None };
                mask.vector = Some(photocraft_xcf::VectorLayer {
                    source_modified: false,
                    path_tattoo: 0,
                    filled: false,
                    stroked: false,
                    fill: fill.clone(),
                    stroke: photocraft_xcf::VectorStroke { fill, width_bits: 0, cap_style: 0, join_style: 0, miter_limit_bits: 0, dashes_bits: vec![] },
                });
            }
            7 => mask.text_flags = 2,
            _ => panic!("unexpected mutation ordinal"),
        }
        assert!(!admitted(&bad).unwrap(), "nonraster mask mutation{mutation}");
    }
}
#[test]
#[ignore = "immutable temp Video108 source domain and hostile controls"]
fn options_108_source_predicate_is_closed() {
    let observations = packet::observations().unwrap();
    for row in observations["records"].as_array().unwrap() {
        let file = photocraft_xcf::decode(&packet::blob(&packet::root(), row).unwrap(), &Default::default()).unwrap();
        assert!(admitted(&file).unwrap());
        assert_eq!(super::observed_epoch(&file.layers[0].effects[0]), Some(packet::EPOCH));
        assert!(crate::xcf_point_replace_source::classify(&file, &Interrupt::cancel_only(&|| true), None).is_err());
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let late = || {
            let old = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            old >= 2
        };
        assert!(crate::xcf_point_replace_source::classify(&file, &Interrupt::cancel_only(&late), None).is_err());
        let limits = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
        assert!(crate::xcf_point_replace_source::classify(&file, &Interrupt::NONE, Some(&photocraft_xcf::Budget::new(limits, None))).is_err());
        for mutation in 0..34 {
            let mut bad = file.clone();
            match mutation {
                0 => bad.version = 21,
                1 => bad.precision = 675,
                2 => bad.encoding = photocraft_xcf::Encoding::Linear,
                3 => bad.model = photocraft_xcf::Model::Gray,
                4 => bad.width = 17,
                5 => bad.layers[0].offset = (2, 1),
                6 => bad.layers[0].effects[0].version = Some("future".into()),
                7 => bad.layers[0].effects[0].state.enabled = None,
                8 => bad.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
                9 => bad.layers[0].effects[0].state.blend_space = Some(-1877441967),
                10 => bad.layers[0].effects[0].mask = None,
                11 => bad.layers[0].effects[0].arguments.clear(),
                12 => {
                    let duplicate = bad.layers[0].effects[0].arguments[0].clone();
                    bad.layers[0].effects[0].arguments[1] = duplicate;
                }
                13 => bad.layers[0].effects[0].arguments[0].name = "unknown".into(),
                14 => bad.layers[0].effects[0].state.region = Some(1),
                15 => bad.layers[0].group = true,
                16 => bad.layers[0].visible = false,
                17 => bad.layers[0].effects[0].state.clipping = Some(1),
                18 => bad.icc = vec![1],
                19 => bad.layers[0].effects[0].operation = "gegl:invert-gamma".into(),
                20 => bad.layers[0].effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Enum(9),
                21 => bad.layers[0].effects[0].arguments[0].value = photocraft_xcf::ArgumentValue::Boolean(false),
                22 => bad.layers[0].effects[0].state.mode = Some(28),
                23 => bad.layers[0].effects[0].state.opacity_u8 = Some(255),
                24 => bad.layers[0].effects[0].state.composite_space = Some(-1739226543),
                25 => bad.layers[0].effects[0].state.composite_mode = Some(0),
                26 => bad.layers[0].effects[0].arguments[0].graph = Some(photocraft_xcf::StoredGraph { entries: vec![], nodes: 0, edges: 0 }),
                27 => bad.layers[0].effects[0].arguments[0].configuration = Some(photocraft_xcf::EffectConfiguration { nodes: vec![], typed: None }),
                28 => bad.layers.push(bad.layers[0].clone()),
                29 => bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::NAN.to_ne_bytes()),
                30 => bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&2f32.to_ne_bytes()),
                31 => bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&f32::INFINITY.to_ne_bytes()),
                32 => bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                33 => bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = photocraft_xcf::Sample::U32,
                _ => panic!("unexpected mutation ordinal"),
            }
            assert!(!admitted(&bad).unwrap(), "{} mutation{mutation}", row["id"]);
        }
    }
}
