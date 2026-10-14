//! Synthetic physical effect-record drafts, UNRUN; not full XCF/source proof.
use super::*;
fn fixture(empty_icon_nul: bool) -> (Vec<u8>, Effect<()>) {
    let mut source = b"gimp xcf v022\0".to_vec();
    let name = "HSV source";
    let icon = "";
    let operation = "gegl:stretch-contrast-hsv";
    for value in [name, icon, operation, "0:0"] {
        let n = if value.is_empty() && !empty_icon_nul { 0 } else { value.len() + 1 };
        source.extend_from_slice(&(n as u32).to_be_bytes());
        if n > 0 {
            source.extend_from_slice(value.as_bytes());
            source.push(0);
        }
    }
    let words = [(8, 1), (33, 1f32.to_bits()), (7, 62), (35, 239108689), (36, 239108689), (37, 239108689), (46, 0), (44, 1)];
    let mut properties = Vec::new();
    for (tag, value) in words {
        source.extend_from_slice(&(tag as u32).to_be_bytes());
        source.extend_from_slice(&4u32.to_be_bytes());
        let offset = source.len() as u64;
        let payload = value.to_be_bytes().to_vec();
        source.extend_from_slice(&payload);
        properties.push(RawProperty { tag, offset, declared_bytes: 4, payload });
    }
    source.extend_from_slice(&[0; 16]);
    let state = EffectState {
        enabled: Some(true),
        opacity_u8: None,
        opacity_bits: Some(1f32.to_bits()),
        mode: Some(62),
        composite_mode: Some(239108689),
        composite_space: Some(239108689),
        blend_space: Some(239108689),
        clipping: Some(0),
        region: Some(1),
    };
    (
        source,
        Effect {
            name: name.into(),
            icon: icon.into(),
            operation: operation.into(),
            version: Some("0:0".into()),
            properties,
            arguments: Vec::new(),
            mask: None,
            state,
        },
    )
}
fn identity() -> StretchHsvSourceIdentity<'static> {
    StretchHsvSourceIdentity { xcf_version: 22, expected_epoch: "explicit-caller-test-epoch", schema_sha256: STRETCH_HSV_SCHEMA }
}
fn map<'a>(s: &'a [u8], e: &'a Effect<()>) -> Result<MappedStretchHsvSource<'a, ()>, IoError> {
    map_retained_stretch_hsv_source(s, e, FilterId(7), None, identity(), StretchHsvMappingLimits::default(), &Interrupt::NONE, &mut |_, _| Ok(()), &mut |_| {
        Ok(())
    })
}
#[test]
fn exact_zero_argument_records_preserve_both_empty_string_frames_and_opaque_words() {
    for nul in [false, true] {
        let (s, e) = fixture(nul);
        let mapped = map(&s, &e).unwrap();
        assert!(std::ptr::eq(mapped.original_effect(), &e));
        assert_eq!(mapped.original_source(), s);
        let n = mapped.node();
        assert!(n.arguments.is_empty());
        assert_eq!(n.operation.name, e.operation);
        assert_eq!(n.operation.schema_epoch, identity().expected_epoch);
        let outer = n.composition.as_ref().unwrap();
        assert_eq!(outer.stored_operator, 239108689);
        assert_eq!(outer.stored_blend_space, 239108689);
        assert!(outer.resolved.is_none());
        assert_eq!(n.opacity_f64_bits, 1f64.to_bits());
    }
}
#[test]
fn raw_header_or_state_changes_cannot_be_replaced_by_asserted_typed_metadata() {
    let (s, e) = fixture(false);
    let mut altered = e.clone();
    altered.name = "different".into();
    assert!(map(&s, &altered).is_err());
    let mut altered = e.clone();
    altered.state.region = Some(0);
    assert!(map(&s, &altered).is_err());
    let mut raw = s.clone();
    let at = e.properties[0].offset as usize;
    raw[at + 3] = 0;
    assert!(map(&raw, &e).is_err());
    let mut altered = e.clone();
    altered.properties.remove(0);
    assert!(map(&s, &altered).is_err());
    let mut altered = e.clone();
    altered.arguments.push(Argument {
        name: "unexpected".into(),
        property_index: 0,
        value: photocraft_xcf::ArgumentValue::Float64Bits(0),
        configuration: None,
        graph: None,
    });
    assert!(map(&s, &altered).is_err());
}
#[test]
fn source_callbacks_admit_before_scan_and_preserve_mask_association_refusal() {
    let (s, e) = fixture(false);
    let mut charged = Vec::new();
    let mapped = map_retained_stretch_hsv_source(
        &s,
        &e,
        FilterId(7),
        None,
        identity(),
        StretchHsvMappingLimits::default(),
        &Interrupt::NONE,
        &mut |resident, index| {
            charged.push((resident, index));
            Ok(())
        },
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(charged.len(), 1);
    assert!(mapped.requirements().selected_input_bytes >= s.len() as u64);
    assert!(
        map_retained_stretch_hsv_source(
            &s,
            &e,
            FilterId(7),
            Some(MaskId(8)),
            identity(),
            StretchHsvMappingLimits::default(),
            &Interrupt::NONE,
            &mut |_, _| Ok(()),
            &mut |_| Ok(())
        )
        .is_err()
    );
    let mut small = StretchHsvMappingLimits::default();
    small.resident_bytes = 1;
    assert!(map_retained_stretch_hsv_source(&s, &e, FilterId(7), None, identity(), small, &Interrupt::NONE, &mut |_, _| Ok(()), &mut |_| Ok(())).is_err());
    let cancel = || true;
    assert!(matches!(
        map_retained_stretch_hsv_source(
            &s,
            &e,
            FilterId(7),
            None,
            identity(),
            StretchHsvMappingLimits::default(),
            &Interrupt::cancel_only(&cancel),
            &mut |_, _| Ok(()),
            &mut |_| Ok(())
        ),
        Err(IoError::Cancelled)
    ));
}
