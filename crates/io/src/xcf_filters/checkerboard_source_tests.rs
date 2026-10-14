//! Synthetic source-frame drafts UNRUN; no complete XCF or stock render proof.
use super::*;
use photocraft_xcf::EffectState;
fn fixture(mask: bool) -> (Vec<u8>, Effect<()>) {
    let mut source = b"gimp xcf v022\0".to_vec();
    for text in ["Checker literal", "", "gegl:checkerboard", "0:0"] {
        source.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        source.extend_from_slice(text.as_bytes());
        source.push(0);
    }
    let mut props = Vec::new();
    let mut property = |tag: u32, raw: Vec<u8>| {
        source.extend_from_slice(&tag.to_be_bytes());
        source.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        let offset = source.len() as u64;
        source.extend_from_slice(&raw);
        props.push(RawProperty { tag, offset, declared_bytes: raw.len() as u32, payload: raw });
    };
    for (tag, value) in [(8, 1u32), (33, 1f32.to_bits()), (7, 62), (35, 239108689), (36, 239108689), (37, 239108689), (46, 0), (44, 1)] {
        property(tag, value.to_be_bytes().to_vec());
    }
    let mut args = Vec::new();
    for (i, (name, value)) in [("x", 16i32), ("y", 17), ("x-offset", i32::MIN), ("y-offset", i32::MAX)].into_iter().enumerate() {
        let mut raw = ((name.len() + 1) as u32).to_be_bytes().to_vec();
        raw.extend_from_slice(name.as_bytes());
        raw.push(0);
        raw.extend_from_slice(&1u32.to_be_bytes());
        raw.extend_from_slice(&value.to_be_bytes());
        property(45, raw);
        args.push(Argument { name: name.into(), property_index: 8 + i, value: ArgumentValue::Integer(value), configuration: None, graph: None });
    }
    for (i, name) in ["color1", "color2"].into_iter().enumerate() {
        let encoding = "R'G'B'A float";
        let samples: Vec<u8> = [0x80000000u32, 0x7fc01234, 1, 0x3f800000].into_iter().flat_map(u32::to_le_bytes).collect();
        let tail = vec![0xde, 0xad, i as u8];
        let mut raw = ((name.len() + 1) as u32).to_be_bytes().to_vec();
        raw.extend_from_slice(name.as_bytes());
        raw.push(0);
        raw.extend_from_slice(&8u32.to_be_bytes());
        raw.extend_from_slice(&((encoding.len() + 1) as u32).to_be_bytes());
        raw.extend_from_slice(encoding.as_bytes());
        raw.push(0);
        raw.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        raw.extend_from_slice(&samples);
        raw.extend_from_slice(&(tail.len() as u32).to_be_bytes());
        raw.extend_from_slice(&tail);
        property(45, raw);
        args.push(Argument {
            name: name.into(),
            property_index: 12 + i,
            value: ArgumentValue::Color { encoding: encoding.into(), samples, icc: tail },
            configuration: None,
            graph: None,
        });
    }
    source.extend_from_slice(&[0; 8]);
    source.extend_from_slice(&(if mask { 2048u64 } else { 0 }).to_be_bytes());
    let effect = Effect {
        name: "Checker literal".into(),
        icon: "".into(),
        operation: "gegl:checkerboard".into(),
        version: Some("0:0".into()),
        properties: props,
        arguments: args,
        mask: mask.then(|| Box::new(Drawable { record_offset: 2048, name: "held original mask".into(), width: 96, height: 80, ..Drawable::default() })),
        state: EffectState {
            enabled: Some(true),
            opacity_u8: None,
            opacity_bits: Some(1f32.to_bits()),
            mode: Some(62),
            composite_mode: Some(239108689),
            composite_space: Some(239108689),
            blend_space: Some(239108689),
            clipping: Some(0),
            region: Some(1),
        },
    };
    (source, effect)
}
fn identity() -> CheckerboardSourceIdentity<'static> {
    CheckerboardSourceIdentity { xcf_version: 22, expected_epoch: CHECKERBOARD_EPOCH, schema_sha256: CHECKERBOARD_SCHEMA }
}
fn map<'a>(s: &'a [u8], e: &'a Effect<()>) -> Result<MappedCheckerboardSource<'a, ()>, IoError> {
    map_retained_checkerboard_source(
        s,
        e,
        FilterId(7),
        e.mask.as_ref().map(|_| MaskId(9)),
        identity(),
        CheckerboardMappingLimits::default(),
        &Interrupt::NONE,
        &mut |_, _| Ok(()),
        &mut |_| Ok(()),
    )
}
#[test]
fn six_fields_exact_native_colors_signed_offsets_and_explicit_unstored_gap() {
    let (s, e) = fixture(false);
    let m = map(&s, &e).unwrap();
    assert_eq!(m.node().arguments.len(), 6);
    assert_eq!(m.schema_gap(), PublicSchemaGap::UnknownOutputFormatParameterNotStored);
    assert_eq!(m.stored().integer_bits, [16, 17, i32::MIN as u32, i32::MAX as u32]);
    assert_eq!(m.node().arguments[2].value, FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(i32::MIN) }));
    for (slot, arg) in m.node().arguments.iter().skip(4).enumerate() {
        let FilterValue::StoredColor(c) = &arg.value else { panic!("stored color") };
        assert_eq!(c.layout, m.stored().colors[slot].layout);
        assert_eq!(c.samples, m.stored().colors[slot].samples);
        assert_eq!(c.opaque_tail, m.stored().colors[slot].opaque_tail);
    }
    assert!(m.node().composition.as_ref().unwrap().resolved.is_none());
}
#[test]
fn original_mask_is_borrowed_separately_and_physical_pointer_cannot_be_substituted() {
    let (s, e) = fixture(true);
    let m = map(&s, &e).unwrap();
    assert!(std::ptr::eq(m.original_mask().unwrap(), e.mask.as_deref().unwrap()));
    assert_eq!(m.original_mask().unwrap().width, 96);
    assert_eq!(m.node().mask, Some(ItemRef::Mask(MaskId(9))));
    let mut changed = e.clone();
    changed.mask.as_mut().unwrap().record_offset = 2049;
    assert!(map(&s, &changed).is_err());
}
#[test]
fn original_header_integer_layout_sample_tail_and_property_associations_are_required() {
    let (s, e) = fixture(false);
    let mut changed = e.clone();
    changed.name.push('x');
    assert!(map(&s, &changed).is_err());
    let mut changed = e.clone();
    changed.arguments[0].value = ArgumentValue::Integer(17);
    assert!(map(&s, &changed).is_err());
    let mut changed = e.clone();
    changed.arguments[1].property_index = changed.arguments[0].property_index;
    assert!(map(&s, &changed).is_err());
    for field in 0..3 {
        let mut changed = e.clone();
        let ArgumentValue::Color { encoding, samples, icc } = &mut changed.arguments[4].value else { panic!("fixture") };
        match field {
            0 => encoding.push('x'),
            1 => samples[0] ^= 1,
            _ => icc[0] ^= 1,
        };
        assert!(map(&s, &changed).is_err());
    }
}
#[test]
fn unknown_or_seventh_default_is_never_invented() {
    let (s, e) = fixture(false);
    let mut changed = e.clone();
    changed.arguments[0].name = "unknown".into();
    assert!(map(&s, &changed).is_err());
    let mut changed = e.clone();
    changed.arguments.push(Argument { name: "unknown".into(), property_index: 0, value: ArgumentValue::Unknown(99), configuration: None, graph: None });
    assert!(map(&s, &changed).is_err());
    let mut id = identity();
    id.expected_epoch = "other inventory";
    assert!(
        map_retained_checkerboard_source(
            &s,
            &e,
            FilterId(7),
            None,
            id,
            CheckerboardMappingLimits::default(),
            &Interrupt::NONE,
            &mut |_, _| Ok(()),
            &mut |_| Ok(())
        )
        .is_err()
    );
}
#[test]
fn whole_lower_reserve_work_precedes_source_scans_and_candidate_copy() {
    let (s, e) = fixture(false);
    let mut charged = false;
    assert!(
        map_retained_checkerboard_source(
            &s,
            &e,
            FilterId(7),
            None,
            identity(),
            CheckerboardMappingLimits::default(),
            &Interrupt::NONE,
            &mut |_, _| Err(bad("outer resident refusal")),
            &mut |_| {
                charged = true;
                Ok(())
            }
        )
        .is_err()
    );
    assert!(!charged);
    assert!(
        map_retained_checkerboard_source(
            &s,
            &e,
            FilterId(7),
            None,
            identity(),
            CheckerboardMappingLimits::default(),
            &Interrupt::cancel_only(&|| true),
            &mut |_, _| Ok(()),
            &mut |_| Ok(())
        )
        .is_err()
    );
    let mut limits = CheckerboardMappingLimits::default();
    limits.work_units = 1;
    assert!(map_retained_checkerboard_source(&s, &e, FilterId(7), None, identity(), limits, &Interrupt::NONE, &mut |_, _| Ok(()), &mut |_| Ok(())).is_err());
}
