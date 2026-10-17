//! UNRUN ordinary layer-mask placement regression drafts; no new source domain.
use super::*;

fn owner(sample: Sample) -> Drawable {
    let decoded = if sample == Sample::F16 { Sample::F32 } else { sample };
    let mask = Drawable {
        name: "local mask".into(),
        width: 2,
        height: 2,
        model: Model::Gray,
        alpha: false,
        plane: Some(Plane { width: 2, height: 2, channels: 1, sample: decoded, bytes: vec![0; 4 * decoded.source_bytes()] }),
        ..Default::default()
    };
    Drawable { name: "offset owner".into(), width: 2, height: 2, offset: (-257, 129), mask: Some(Box::new(mask)), ..Default::default() }
}

#[test]
fn local_zero_mask_retains_existing_world_placement_at_every_native_depth() {
    for sample in [Sample::U8, Sample::U16, Sample::U32, Sample::F16, Sample::F32, Sample::F64] {
        let d = owner(sample);
        let before = d.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes.clone();
        let pointer = d.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes.as_ptr();
        let placement = validate(&d, sample, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(placement.world_bounds, Rect::new(-257, 129, -255, 131));
        assert_eq!(placement.world_offset, [-257, 129]);
        assert_eq!(d.mask.as_ref().unwrap().offset, (0, 0));
        assert_eq!(d.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes, before);
        assert_eq!(d.mask.as_ref().unwrap().plane.as_ref().unwrap().bytes.as_ptr(), pointer);
    }
}

#[test]
fn nonzero_offset_and_unequal_size_fail_admission_before_the_copy_stage() {
    // Before this guard the first case passed admit and its offset was discarded.
    for variant in 0..4 {
        let mut d = owner(Sample::U8);
        let mask = d.mask.as_mut().unwrap();
        match variant {
            0 => mask.offset = (1, 0),
            1 => mask.offset = (0, -1),
            2 => mask.width = 1,
            _ => mask.height = 3,
        }
        let copied = std::cell::Cell::new(false);
        let result = super::super::admit(&d, false, false, false, false, None, Sample::U8, &Interrupt::NONE).inspect(|_| {
            copied.set(true);
        });
        assert!(result.is_err());
        assert!(!copied.get());
    }
}

#[test]
fn actual_native_descriptor_and_length_are_checked_without_default_substitution() {
    for variant in 0..8 {
        let mut d = owner(Sample::F64);
        let mask = d.mask.as_mut().unwrap();
        match variant {
            0 => mask.model = Model::Rgb,
            1 => mask.alpha = true,
            2 => mask.group = true,
            3 => mask.plane.as_mut().unwrap().channels = 2,
            4 => mask.plane.as_mut().unwrap().sample = Sample::F32,
            5 => mask.plane.as_mut().unwrap().width = 1,
            6 => {
                mask.plane.as_mut().unwrap().bytes.pop();
            }
            _ => mask.plane = None,
        }
        assert!(validate(&d, Sample::F64, &Interrupt::NONE).is_err());
    }
}

#[test]
fn cancellation_and_unrepresentable_world_bounds_fail_before_mapping() {
    let mut d = owner(Sample::U16);
    let interrupt = Interrupt::cancel_only(&|| true);
    assert!(matches!(validate(&d, Sample::U16, &interrupt), Err(IoError::Cancelled)));
    d.offset = (1 << 29, 0);
    assert!(validate(&d, Sample::U16, &Interrupt::NONE).is_err());
    d.mask = None;
    assert_eq!(validate(&d, Sample::U16, &Interrupt::NONE).unwrap(), None);
}
