//! UNRUN residual identity, exact source-domain and bounded visitor drafts.
use super::super::super::plane::fixture;
use super::*;
use photocraft_xcf::{Argument, Effect};

fn file(operation: &str, precision: u32, model: u32, width: u32, height: u32) -> XcfFile {
    let channels = if model == 0 { 4 } else { 2 };
    let sample = match precision {
        150 => vec![255],
        250 => 32768u16.to_be_bytes().to_vec(),
        600 | 650 => 0.5f32.to_be_bytes().to_vec(),
        750 => 0.5f64.to_be_bytes().to_vec(),
        _ => unreachable!(),
    };
    let bytes = fixture::image_custom(
        22,
        precision,
        0,
        width,
        height,
        &sample.repeat(width as usize * height as usize * channels),
        &fixture::FixtureOptions { model, layer_type: Some(if model == 0 { 1 } else { 3 }), ..Default::default() },
    );
    let mut file = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    let layer = &mut file.layers[0];
    layer.visible = true;
    layer.opacity = 1.0;
    layer.mode = 28;
    layer.blend_space = 0;
    layer.composite_space = -1;
    layer.composite_mode = -1;
    layer.effects.push(Effect {
        name: String::new(),
        icon: String::new(),
        operation: operation.into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: Default::default(),
    });
    file
}
fn colorspace(file: &mut XcfFile, value: i32) {
    file.layers[0].effects[0].arguments =
        vec![Argument { name: "colorspace".into(), property_index: 0, value: ArgumentValue::Enum(value), configuration: None, graph: None }];
}
#[test]
fn source_route_keeps_distinct_precision_domains_and_does_not_relabel_pointreplace() {
    for (model, cs, expected) in
        [(0, 0, ResidualPointFamily::SaturationRgb), (0, 2, ResidualPointFamily::SaturationYuv), (1, 2, ResidualPointFamily::SaturationGray)]
    {
        let mut f = file("gegl:saturation", 650, model, 8, 6);
        colorspace(&mut f, cs);
        assert_eq!(ResidualPointFamily::hint(&f), Some(expected));
        assert_eq!(SourceSelectedPolicy::family(&f), SelectedStagedFamily::ResidualPoint(expected));
        assert!(expected.qualify(&f, &Interrupt::NONE, None).unwrap());
        let mut late = admission::LateAdmissionDomains::default();
        expected.set_late(&mut late);
        assert_eq!(late.saturation_yuv_domain, expected == ResidualPointFamily::SaturationYuv);
        assert_eq!(late.saturation_gray_domain, expected == ResidualPointFamily::SaturationGray);
        assert_eq!(late.saturation_domain, expected == ResidualPointFamily::SaturationRgb);
    }
    let mut yuv64 = file("gegl:saturation", 750, 0, 8, 6);
    colorspace(&mut yuv64, 2);
    assert!(!ResidualPointFamily::SaturationYuv.qualify(&yuv64, &Interrupt::NONE, None).unwrap());
    for precision in [250, 600, 750] {
        let f = file("gegl:value-invert", precision, 0, 8, 6);
        assert!(ResidualPointFamily::ValueInvertPrecision.qualify(&f, &Interrupt::NONE, None).unwrap());
    }
    let f = file("gegl:value-invert", 650, 0, 8, 6);
    assert!(!ResidualPointFamily::ValueInvertPrecision.qualify(&f, &Interrupt::NONE, None).unwrap());
    let f = file("gimp:desaturate", 650, 1, 8, 6);
    assert!(!ResidualPointFamily::Desaturate.qualify(&f, &Interrupt::NONE, None).unwrap());
}
#[test]
fn posterize_bounded_native_predicate_matches_original_opaque_gate_and_refuses_changes() {
    let mut f = file("gimp:posterize", 150, 0, 32, 32);
    assert!(super::super::super::qualified_opaque_neighborhood(&f, "gimp:posterize"));
    assert!(ResidualPointFamily::Posterize.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].plane.as_mut().unwrap().bytes[3] = 254;
    assert!(!super::super::super::qualified_opaque_neighborhood(&f, "gimp:posterize"));
    assert!(!ResidualPointFamily::Posterize.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].plane.as_mut().unwrap().bytes[3] = 255;
    f.layers[0].offset = (-1, 0);
    assert!(!ResidualPointFamily::Posterize.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].offset = (0, 0);
    f.layers[0].plane.as_mut().unwrap().bytes.pop();
    assert!(!ResidualPointFamily::Posterize.qualify(&f, &Interrupt::NONE, None).unwrap());
}
#[test]
fn hostile_routing_is_o1_capped_and_real_native_work_and_cancel_are_not_waived() {
    let mut f = file("gegl:saturation", 650, 0, 8, 6);
    colorspace(&mut f, 2);
    let argument = f.layers[0].effects[0].arguments[0].clone();
    f.layers[0].effects[0].arguments.resize(100000, argument);
    assert_eq!(ResidualPointFamily::hint(&f), None);
    assert!(!ResidualPointFamily::SaturationYuv.qualify(&f, &Interrupt::NONE, None).unwrap());
    let f = file("gimp:posterize", 150, 0, 32, 32);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(ResidualPointFamily::Posterize.qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(ResidualPointFamily::Posterize.qualify(&f, &cancelled, None), Err(IoError::Cancelled)));
}
