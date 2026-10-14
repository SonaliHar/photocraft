//! Synthetic regression drafts: intentionally unrun in this implementation wave.
use super::*;
use std::cell::Cell;
fn file() -> XcfFile<Plane> {
    XcfFile {
        paths: Vec::new(),
        tattoo: None,
        item_sets: Vec::new(),
        version: 22,
        width: 1,
        height: 1,
        model: Model::Gray,
        precision: 0,
        sample: Sample::U8,
        encoding: Encoding::NonlinearProfile,
        compression: Compression::None,
        icc: Vec::new(),
        dpi: (72., 72.),
        unit: Unit::Inch,
        palette: Vec::new(),
        layers: Vec::new(),
        channels: Vec::new(),
        guides: Vec::new(),
        diagnostics: Vec::new(),
        charged_bytes: usize::MAX,
        raw_properties: Vec::new(),
    }
}
fn budget() -> Budget<'static> {
    Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None)
}
fn bytes(f: &XcfFile<Plane>) -> u64 {
    census_owned_file(f, OwnedBackingLimits::default(), &budget(), &|| Ok(())).unwrap().retained_bytes()
}
#[test]
fn actual_spare_capacity_replaces_stale_decoder_counter() {
    let mut f = file();
    let before = bytes(&f);
    f.icc.reserve_exact(900);
    f.palette.reserve_exact(17);
    let expected = before + f.icc.capacity() as u64 + (f.palette.capacity() * size_of::<[u8; 3]>()) as u64;
    assert_eq!(bytes(&f), expected);
    f.charged_bytes = 0;
    assert_eq!(bytes(&f), expected);
    let b = budget();
    let r = census_owned_file(&f, OwnedBackingLimits::default(), &b, &|| Ok(())).unwrap();
    assert!(r.belongs_to(&f));
    assert!(!r.belongs_to(&f.clone()));
    assert_eq!(r.index_bytes(), 0);
    assert_eq!(b.used(BudgetKind::Work), r.work_units());
    assert_eq!(b.used(BudgetKind::Visits), r.visits());
    assert_eq!(b.used(BudgetKind::Resident), 0);
}
#[test]
fn graph_typed_configuration_and_boxed_masks_count_deep_backing() {
    let mut f = file();
    let mut d = Drawable::<Plane>::default();
    d.mask = Some(Box::new(Drawable {
        plane: Some(Plane { width: 1, height: 1, channels: 1, sample: Sample::U8, bytes: Vec::with_capacity(77) }),
        ..Drawable::default()
    }));
    let config = EffectConfiguration {
        nodes: vec![ConfigNode {
            start: 0,
            end: 0,
            value: ConfigValue::List(vec![ConfigNode { start: 0, end: 0, value: ConfigValue::Quoted(Vec::with_capacity(31)) }]),
        }],
        typed: Some(TypedConfiguration::Curves(CurvesConfiguration {
            time_bits: None,
            trc: Some(String::with_capacity(19)),
            channels: vec![CurveChannel {
                channel: String::with_capacity(23),
                curve: StoredCurve {
                    curve_type: String::with_capacity(29),
                    points_bits: Vec::with_capacity(5),
                    point_types: vec![String::with_capacity(37)],
                    n_samples: 0,
                    samples_bits: Vec::with_capacity(41),
                },
            }],
        })),
    };
    d.effects.push(Effect {
        name: String::new(),
        icon: String::new(),
        operation: String::new(),
        version: None,
        properties: Vec::new(),
        mask: None,
        state: EffectState::default(),
        arguments: vec![Argument {
            name: String::new(),
            property_index: 0,
            value: ArgumentValue::Color { encoding: String::with_capacity(11), samples: Vec::with_capacity(13), icc: Vec::with_capacity(17) },
            configuration: Some(config),
            graph: Some(StoredGraph {
                nodes: 0,
                edges: 0,
                entries: vec![GraphEntry::Property {
                    name: String::with_capacity(43),
                    start: 0,
                    end: 0,
                    value: GraphValue::Chain(vec![GraphEntry::Operation { identity: String::with_capacity(47), start: 0, end: 0 }]),
                }],
            }),
        }],
    });
    f.layers.push(d);
    let before = bytes(&f);
    let GraphEntry::Property { value: GraphValue::Chain(chain), .. } = &mut f.layers[0].effects[0].arguments[0].graph.as_mut().unwrap().entries[0] else {
        panic!("fixture");
    };
    let GraphEntry::Operation { identity, .. } = &mut chain[0] else {
        panic!("fixture");
    };
    let old = identity.capacity();
    identity.reserve_exact(123);
    let delta = identity.capacity() - old;
    assert_eq!(bytes(&f), before + delta as u64);
    assert!(before > size_of::<XcfFile<Plane>>() as u64 + 77 + 31 + 19 + 23 + 29 + 37 + 11 + 13 + 17 + 43 + 47);
}
#[test]
fn caller_control_is_independent_and_work_is_not_refunded() {
    let mut f = file();
    f.diagnostics = vec![String::new(); 16];
    let calls = Cell::new(0);
    let poll = || {
        let n = calls.get() + 1;
        calls.set(n);
        if n == 12 { Err(XcfError::new(ErrorKind::Cancelled, 0, "current caller cancelled")) } else { Ok(()) }
    };
    let b = budget();
    let e = census_owned_file(&f, OwnedBackingLimits::default(), &b, &poll).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert!(b.used(BudgetKind::Work) > 0);
    assert_eq!(b.used(BudgetKind::Resident), 0);
}
#[test]
fn byte_work_visit_and_depth_limits_refuse_without_receipt() {
    let f = file();
    let base = bytes(&f);
    let defaults = OwnedBackingLimits::default();
    for limits in [
        OwnedBackingLimits { resident_bytes: base - 1, ..defaults },
        OwnedBackingLimits { work_units: 0, ..defaults },
        OwnedBackingLimits { visits: 0, ..defaults },
        OwnedBackingLimits { depth: 0, ..defaults },
    ] {
        assert_eq!(census_owned_file(&f, limits, &budget(), &|| Ok(())).unwrap_err().kind, ErrorKind::Limit);
    }
    let mut f = file();
    let mut d = Drawable::<Plane>::default();
    for _ in 0..8 {
        d = Drawable { mask: Some(Box::new(d)), ..Drawable::default() };
    }
    f.layers.push(d);
    assert_eq!(census_owned_file(&f, OwnedBackingLimits { depth: 10, ..defaults }, &budget(), &|| Ok(())).unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn path_parasites_and_text_layout_spare_storage_are_counted() {
    let mut f = file();
    let mut path = VectorPath::default();
    path.parasites.push(PathParasite { name: String::with_capacity(29), flags: 0, bytes: Vec::with_capacity(83) });
    path.strokes.push(PathStroke { closed: false, points: vec![PathPoint { kind: 0, legacy_integer_coordinates: None, components: Vec::with_capacity(19) }] });
    f.paths.push(path);
    let text = TextLayer {
        parasite_flags: 0,
        raw: Vec::new(),
        text: None,
        markup: None,
        font: Some(FontReference { unknown_fields: vec![String::with_capacity(53)], ..FontReference::default() }),
        configuration: Vec::new(),
        unknown_fields: Vec::new(),
        content: String::new(),
        runs: vec![TextRun { start: 0, end: 0, style_ids: Vec::with_capacity(17) }],
        styles: vec![MarkupStyle { tag: String::new(), attributes: vec![(String::with_capacity(37), String::with_capacity(41))] }],
        layout: TextLayout { language: Some(String::with_capacity(47)), ..TextLayout::default() },
    };
    f.layers.push(Drawable { text: Some(text), ..Drawable::default() });
    let before = bytes(&f);
    let v = &mut f.layers[0].text.as_mut().unwrap().layout.language.as_mut().unwrap();
    let old = v.capacity();
    v.reserve_exact(200);
    let delta = v.capacity() - old;
    assert_eq!(bytes(&f), before + delta as u64);
}
