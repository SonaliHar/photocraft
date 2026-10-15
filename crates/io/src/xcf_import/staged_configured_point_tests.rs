//! UNRUN configured source identity and exact native-domain drafts.
use super::super::super::plane::fixture;
use super::*;
use photocraft_xcf::{Argument, Effect};
fn source(operation: &str, precision: u32, width: u32, height: u32) -> XcfFile {
    let word = match precision {
        100 | 150 | 175 => vec![127],
        200 | 250 | 275 => 32768u16.to_be_bytes().to_vec(),
        300 | 350 | 375 => 0x80000000u32.to_be_bytes().to_vec(),
        500 | 550 | 575 => 0x3800u16.to_be_bytes().to_vec(),
        600 | 650 | 675 => 0.5f32.to_be_bytes().to_vec(),
        700 | 750 | 775 => 0.5f64.to_be_bytes().to_vec(),
        _ => unreachable!(),
    };
    let bytes = fixture::image(22, precision, 0, width, height, &word.repeat(width as usize * height as usize * 4));
    let mut f = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    let d = &mut f.layers[0];
    d.visible = true;
    d.opacity = 1.0;
    d.mode = 28;
    d.blend_space = 0;
    d.composite_space = -1;
    d.composite_mode = -1;
    d.effects.push(Effect {
        name: String::new(),
        icon: String::new(),
        operation: operation.into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: Vec::new(),
        mask: None,
        state: Default::default(),
    });
    f
}
#[test]
fn color_balance_depth_and_precision_keep_distinct_original_qualifiers() {
    for precision in [100, 150, 175, 200, 250, 275, 300, 350, 375, 500, 550, 575, 600, 650, 675, 700, 750, 775] {
        let f = source("gimp:color-balance", precision, 8, 6);
        let expected = if matches!(precision, 100 | 150 | 175 | 275 | 300 | 350 | 375 | 500 | 550 | 575 | 675 | 775) {
            ConfiguredPointFamily::ColorBalanceDepth
        } else {
            ConfiguredPointFamily::ColorBalance
        };
        assert_eq!(ConfiguredPointFamily::hint(&f), Some(expected));
        assert!(expected.qualify(&f, &Interrupt::NONE, None).unwrap());
        assert_eq!(SourceSelectedPolicy::family(&f), SelectedStagedFamily::ConfiguredPoint(expected));
        let other =
            if expected == ConfiguredPointFamily::ColorBalance { ConfiguredPointFamily::ColorBalanceDepth } else { ConfiguredPointFamily::ColorBalance };
        assert!(!other.qualify(&f, &Interrupt::NONE, None).unwrap());
        assert_eq!(
            expected.domain(),
            if expected == ConfiguredPointFamily::ColorBalance { NeighborhoodDomain::ColorBalancePrecision } else { NeighborhoodDomain::ColorBalanceDepth }
        );
    }
}
#[test]
fn structured_levels_preserves_finite_hdr_but_coverage_and_geometry_remain_closed() {
    let mut f = source("gimp:levels", 650, 12, 9);
    f.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&4.0f32.to_ne_bytes());
    assert!(ConfiguredPointFamily::Levels.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&1.25f32.to_ne_bytes());
    assert!(!ConfiguredPointFamily::Levels.qualify(&f, &Interrupt::NONE, None).unwrap());
    f.layers[0].plane.as_mut().unwrap().bytes[12..16].copy_from_slice(&0.5f32.to_ne_bytes());
    f.layers[0].offset = (-1, 0);
    assert!(!ConfiguredPointFamily::Levels.qualify(&f, &Interrupt::NONE, None).unwrap());
    let f = source("gimp:levels", 750, 12, 9);
    assert!(!ConfiguredPointFamily::Levels.qualify(&f, &Interrupt::NONE, None).unwrap());
    let f = source("gimp:curves", 150, 12, 9);
    assert!(ConfiguredPointFamily::Curves.qualify(&f, &Interrupt::NONE, None).unwrap());
    let f = source("gimp:curves", 650, 12, 9);
    assert!(!ConfiguredPointFamily::Curves.qualify(&f, &Interrupt::NONE, None).unwrap());
}
#[test]
fn hints_do_not_admit_untyped_configs_and_work_cancel_failures_propagate() {
    let mut f = source("gimp:curves", 150, 12, 9);
    let arg = Argument { name: "config".into(), property_index: 0, value: ArgumentValue::Config(String::new()), configuration: None, graph: None };
    f.layers[0].effects[0].arguments.push(arg.clone());
    assert!(ConfiguredPointFamily::Curves.qualify(&f, &Interrupt::NONE, None).unwrap());
    assert!(
        crate::xcf_filters::qualified_stack(
            &f.layers[0].effects,
            &[None],
            photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
            1,
            22,
            NeighborhoodDomain::Unqualified
        )
        .is_err()
    );
    f.layers[0].effects[0].arguments.resize(100000, arg);
    assert_eq!(ConfiguredPointFamily::hint(&f), None);
    let f = source("gimp:levels", 650, 12, 9);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(ConfiguredPointFamily::Levels.qualify(&f, &Interrupt::NONE, Some(&budget)).is_err());
    let cancel = || true;
    let stop = Interrupt::cancel_only(&cancel);
    assert!(matches!(ConfiguredPointFamily::Levels.qualify(&f, &stop, None), Err(IoError::Cancelled)));
}
