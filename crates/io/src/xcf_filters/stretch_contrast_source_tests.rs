//! UNRUN physical-record drafts; not complete XCF/math/profile qualification.
use super::*;
use photocraft_xcf::EffectState;
fn fixture(keep: bool, perceptual: bool) -> (Vec<u8>, Effect<()>) {
    let mut source = b"gimp xcf v022\0".to_vec();
    for text in ["Plain Stretch literal", "", "gegl:stretch-contrast", "0:0"] {
        source.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        source.extend_from_slice(text.as_bytes());
        source.push(0);
    }
    let mut properties = Vec::new();
    let mut property = |tag: u32, payload: Vec<u8>| {
        source.extend_from_slice(&tag.to_be_bytes());
        source.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        let offset = source.len() as u64;
        source.extend_from_slice(&payload);
        properties.push(RawProperty { tag, offset, declared_bytes: payload.len() as u32, payload });
    };
    for (tag, value) in [(8, 1u32), (33, 1f32.to_bits()), (7, 62), (35, 0x6e2a8251), (36, 0x6e2a8251), (37, 0x6e2a8251), (46, 0), (44, 1)] {
        property(tag, value.to_be_bytes().to_vec());
    }
    let mut arguments = Vec::new();
    for (i, (name, value)) in [("keep-colors", keep), ("perceptual", perceptual)].into_iter().enumerate() {
        let mut payload = ((name.len() + 1) as u32).to_be_bytes().to_vec();
        payload.extend_from_slice(name.as_bytes());
        payload.push(0);
        payload.extend_from_slice(&2u32.to_be_bytes());
        payload.extend_from_slice(&u32::from(value).to_be_bytes());
        property(45, payload);
        arguments.push(Argument { name: name.into(), property_index: 8 + i, value: ArgumentValue::Boolean(value), configuration: None, graph: None });
    }
    source.extend_from_slice(&[0; 16]);
    let effect = Effect {
        name: "Plain Stretch literal".into(),
        icon: "".into(),
        operation: "gegl:stretch-contrast".into(),
        version: Some("0:0".into()),
        properties,
        arguments,
        mask: None,
        state: EffectState {
            enabled: Some(true),
            opacity_u8: None,
            opacity_bits: Some(1f32.to_bits()),
            mode: Some(62),
            composite_mode: Some(0x6e2a8251),
            composite_space: Some(0x6e2a8251),
            blend_space: Some(0x6e2a8251),
            clipping: Some(0),
            region: Some(1),
        },
    };
    (source, effect)
}
fn identity() -> StretchContrastSourceIdentity<'static> {
    StretchContrastSourceIdentity { xcf_version: 22, expected_epoch: STRETCH_CONTRAST_EPOCH, schema_sha256: STRETCH_CONTRAST_SCHEMA }
}
fn map<'a>(source: &'a [u8], effect: &'a Effect<()>) -> Result<MappedStretchContrastSource<'a, ()>, IoError> {
    map_retained_stretch_contrast_source(
        source,
        effect,
        FilterId(7),
        None,
        identity(),
        StretchContrastMappingLimits::default(),
        &Interrupt::NONE,
        &mut |_, _| Ok(()),
        &mut |_| Ok(()),
    )
}
#[test]
fn every_stored_boolean_combination_survives_without_false_false_math_coercion() {
    for keep in [false, true] {
        for perceptual in [false, true] {
            let (source, effect) = fixture(keep, perceptual);
            let mapped = map(&source, &effect).unwrap();
            assert!(std::ptr::eq(mapped.original_effect(), &effect));
            assert_eq!(mapped.original_source(), source);
            assert_eq!(mapped.stored().keep_colors, keep);
            assert_eq!(mapped.stored().perceptual, perceptual);
            assert_eq!(mapped.node().arguments[0].value, FilterValue::Bool(keep));
            assert_eq!(mapped.node().arguments[1].value, FilterValue::Bool(perceptual));
            assert_eq!(mapped.node().operation.name, "gegl:stretch-contrast");
            assert!(mapped.node().composition.as_ref().unwrap().resolved.is_none());
        }
    }
}
#[test]
fn raw_type_boolean_header_outer_and_duplicate_association_are_authoritative() {
    let (source, effect) = fixture(true, false);
    let mut e = effect.clone();
    e.arguments[0].value = ArgumentValue::Boolean(false);
    assert!(map(&source, &e).is_err());
    let mut e = effect.clone();
    e.arguments[1].property_index = e.arguments[0].property_index;
    assert!(map(&source, &e).is_err());
    let mut e = effect.clone();
    e.operation = "gegl:stretch-contrast-hsv".into();
    assert!(map(&source, &e).is_err());
    let mut e = effect.clone();
    e.state.region = Some(0);
    assert!(map(&source, &e).is_err());
    let mut bytes = source.clone();
    let at = effect.properties[8].offset as usize + effect.properties[8].payload.len() - 1;
    bytes[at] = 2;
    assert!(map(&bytes, &effect).is_err());
    let mut e = effect.clone();
    e.arguments[1].name = "keep-colors".into();
    assert!(map(&source, &e).is_err());
}
#[test]
fn reserve_then_full_charge_precede_input_work_and_failure_never_returns_candidate() {
    let (source, effect) = fixture(false, false);
    let mut work_called = false;
    assert!(
        map_retained_stretch_contrast_source(
            &source,
            &effect,
            FilterId(7),
            None,
            identity(),
            StretchContrastMappingLimits::default(),
            &Interrupt::NONE,
            &mut |_, _| Err(bad("reserve denied")),
            &mut |_| {
                work_called = true;
                Ok(())
            }
        )
        .is_err()
    );
    assert!(!work_called);
    let mut reserved = false;
    assert!(
        map_retained_stretch_contrast_source(
            &source,
            &effect,
            FilterId(7),
            None,
            identity(),
            StretchContrastMappingLimits::default(),
            &Interrupt::NONE,
            &mut |_, _| {
                reserved = true;
                Ok(())
            },
            &mut |_| Err(bad("work denied"))
        )
        .is_err()
    );
    assert!(reserved);
    let token = Interrupt::cancel_only(&|| true);
    assert!(matches!(
        map_retained_stretch_contrast_source(
            &source,
            &effect,
            FilterId(7),
            None,
            identity(),
            StretchContrastMappingLimits::default(),
            &token,
            &mut |_, _| Ok(()),
            &mut |_| Ok(())
        ),
        Err(IoError::Cancelled)
    ));
    let limits = StretchContrastMappingLimits { work_units: 1, ..Default::default() };
    assert!(
        map_retained_stretch_contrast_source(&source, &effect, FilterId(7), None, identity(), limits, &Interrupt::NONE, &mut |_, _| Ok(()), &mut |_| Ok(()))
            .is_err()
    );
}
