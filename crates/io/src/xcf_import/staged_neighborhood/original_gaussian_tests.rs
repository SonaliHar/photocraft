//! UNRUN synthetic predicate/visitor and authentic selected-import drafts.
//! Synthetic DTOs prove refusal boundaries only. Corpus source/eager/output
//! buffers are external harness captures, not product-accounted allocations.
use super::*;
use crate::xcf_import::plane::{MappingPlane, ParsedPlaneDescriptor, fixture};
use photocraft_xcf::{Argument, Effect, EffectState};

fn source(x: f32) -> XcfFile {
    let bytes = fixture::image(22, 150, 0, 96, 80, &[11, 29, 83, 255].repeat(96 * 80));
    let mut file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
    let owner = &mut file.layers[0];
    owner.visible = true;
    owner.opacity = 1.0;
    owner.mode = 28;
    owner.blend_space = 0;
    owner.composite_space = -1;
    owner.composite_mode = -1;
    let mut mask = owner.clone();
    mask.model = Model::Gray;
    mask.alpha = false;
    mask.mode = 0;
    mask.blend_space = 2;
    mask.composite_space = 2;
    mask.composite_mode = 1;
    mask.mask_enabled = true;
    mask.mask_shown = false;
    mask.mask_editing = false;
    mask.legacy_linked = false;
    mask.unsupported.clear();
    mask.effects.clear();
    mask.mask = None;
    mask.plane = Some(Plane { width: 96, height: 80, channels: 1, sample: Sample::U8, bytes: vec![0; 96 * 80] });
    owner.effects = vec![Effect {
        name: "Own blur".into(),
        icon: String::new(),
        operation: "gegl:gaussian-blur".into(),
        version: Some("0:0".into()),
        properties: Vec::new(),
        arguments: [
            ("std-dev-x", ArgumentValue::Float32Bits(x.to_bits())),
            ("std-dev-y", ArgumentValue::Float32Bits(1.5f32.to_bits())),
            ("filter", ArgumentValue::Enum(0)),
            ("abyss-policy", ArgumentValue::Enum(1)),
            ("clip-extent", ArgumentValue::Boolean(true)),
        ]
        .into_iter()
        .map(|(name, value)| Argument { name: name.into(), value, property_index: 0, configuration: None, graph: None })
        .collect(),
        mask: Some(Box::new(mask)),
        state: EffectState {
            enabled: Some(true),
            opacity_bits: Some(1f32.to_bits()),
            mode: Some(62),
            blend_space: Some(-856260015),
            composite_space: Some(-856260015),
            composite_mode: Some(-856260015),
            clipping: Some(0),
            region: Some(0),
            ..Default::default()
        },
    }];
    file
}
#[test]
fn original_and_edited_parameters_preserve_the_old_opaque_classifier_and_raw_bytes() {
    for x in [2.25, 3.75] {
        let mut file = source(x);
        let before = file.layers[0].plane.as_ref().unwrap().bytes.clone();
        assert!(crate::xcf_import::admission::classify_late(&file).point_context);
        assert!(<Plane as MappingPlane>::legacy_point_candidate(&file.layers[0], true, false, Size::new(96, 80)).unwrap());
        assert!(qualify(&file, &Interrupt::NONE, None).unwrap());
        assert_eq!(NeighborhoodSourceFamily::hint(&file), Some(NeighborhoodSourceFamily::OriginalGaussian));
        assert_eq!(NeighborhoodSourceFamily::OriginalGaussian.domain(), crate::xcf_filters::NeighborhoodDomain::RasterOriginal);
        file.layers[0].effects[0].arguments.reverse();
        assert!(qualify(&file, &Interrupt::NONE, None).unwrap()); // No argument normalization/order rewrite.
        assert_eq!(file.layers[0].plane.as_ref().unwrap().bytes, before);
        file.layers[0].plane.as_mut().unwrap().bytes[3] = 254;
        assert!(!<Plane as MappingPlane>::legacy_point_candidate(&file.layers[0], true, false, Size::new(96, 80)).unwrap());
        assert!(!qualify(&file, &Interrupt::NONE, None).unwrap());
    }
}
#[test]
fn nearby_source_profile_geometry_state_arguments_and_mask_values_do_not_enroll() {
    for case in 0..19 {
        let mut f = source(2.25);
        match case {
            0 => f.width = 95,
            1 => f.precision = 100,
            2 => f.encoding = Encoding::Linear,
            3 => f.icc = vec![1; 132],
            4 => f.layers[0].offset = (-1, 0),
            5 => f.layers[0].visible = false,
            6 => f.layers[0].effects[0].state.enabled = Some(false),
            7 => f.layers[0].effects[0].state.composite_mode = Some(-856260014),
            8 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float32Bits(2.5f32.to_bits()),
            9 => f.layers[0].effects[0].arguments[0].value = ArgumentValue::Float64Bits(2.25f64.to_bits()),
            10 => f.layers[0].effects[0].arguments[1].name = "std-dev-x".into(),
            11 => f.layers[0].effects[0].mask = None,
            12 => f.layers[0].effects[0].mask.as_mut().unwrap().offset = (1, 0),
            13 => f.layers[0].effects[0].mask.as_mut().unwrap().mask_enabled = false,
            14 => f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[7679] = 1,
            15 => {
                f.layers[0].plane.as_mut().unwrap().bytes.pop();
            }
            16 => {
                let duplicate = f.layers[0].clone();
                f.layers.push(duplicate);
            }
            17 => f.layers[0].effects[0].arguments[2].value = ArgumentValue::Enum(1),
            _ => f.layers[0].effects[0].state.opacity_bits = Some(0.5f32.to_bits()),
        }
        assert!(!qualify(&f, &Interrupt::NONE, None).unwrap(), "case {case}");
    }
    let mut f = source(2.25);
    let arg = f.layers[0].effects[0].arguments[0].clone();
    f.layers[0].effects[0].arguments = vec![arg; 100000];
    assert_eq!(NeighborhoodSourceFamily::hint(&f), None);
    assert!(!qualify(&f, &Interrupt::NONE, None).unwrap());
}

struct Visitor {
    fault: u8,
}
impl ExactParsedPlane for Visitor {
    fn descriptor(&self) -> ParsedPlaneDescriptor {
        ParsedPlaneDescriptor { width: 96, height: 80, channels: 4, sample: Sample::U8 }
    }
    fn visit_native(&self, ctl: &Interrupt<'_>, visit: &mut dyn FnMut(&[u8]) -> Result<(), IoError>) -> Result<(), IoError> {
        let mut block = [255; 256];
        for index in 0..120 {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if self.fault == 1 && index == 119 {
                return Ok(());
            }
            if self.fault == 2 && index == 119 {
                return Err(IoError::Unsupported("injected provider fault".into()));
            }
            if self.fault == 3 && index == 0 {
                visit(&[255; 257])?;
                return Ok(());
            }
            if self.fault == 4 && index == 0 {
                block[3] = 0;
            }
            visit(&block)?;
        }
        if self.fault == 5 {
            visit(&block)?;
        }
        Ok(())
    }
}
#[test]
fn bounded_visitor_errors_and_shared_work_or_cancel_are_fail_closed() {
    assert!(native(&Visitor { fault: 0 }, 4, 30720, true, &Interrupt::NONE, None).unwrap());
    for fault in [1, 2, 3, 5] {
        assert!(native(&Visitor { fault }, 4, 30720, true, &Interrupt::NONE, None).is_err());
    }
    assert!(!native(&Visitor { fault: 4 }, 4, 30720, true, &Interrupt::NONE, None).unwrap());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 32)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    assert!(qualify(&source(2.25), &Interrupt::NONE, Some(&budget)).is_err());
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 320)]).unwrap();
    let budget = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
    // Refused first-block alpha does not license uncharged producer continuation.
    assert!(native(&Visitor { fault: 4 }, 4, 30720, true, &Interrupt::NONE, Some(&budget)).is_err());
    let stopped = || true;
    assert!(matches!(qualify(&source(2.25), &Interrupt::cancel_only(&stopped), None), Err(IoError::Cancelled)));
}

#[cfg(feature = "corpus")]
#[test]
#[ignore = "UNRUN: exact SHA-pinned original/edited Gaussian source through single selected completion"]
fn authentic_original_and_edit_keep_domain_native_words_and_actual_quota_custody() {
    use photocraft_raster::backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLimits},
    };
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    for (path, pin) in [
        ("filter-owner-layer.xcf", "63eb43a9ad9135babe8d832891334d691a0b2899e0ebbeea6271f7161d1b91ba"),
        ("filter-owner-layer.edit.xcf", "9a9d833afdfe29b6386a60059bf10e6336bfa446aea3ed483469c5ea04b60a79"),
    ] {
        let bytes = std::fs::read(root.join(path)).unwrap();
        let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(actual, pin);
        let eager = crate::import(path, &bytes).unwrap().document;
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let baseline = q.usage().unwrap();
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let starts = std::sync::atomic::AtomicU64::new(0);
        let progress = |at: f32| {
            if at.to_bits() == 0.05f32.to_bits() {
                starts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        };
        let options = DecodeOptions { progress: Some(&progress), ..Default::default() };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let owned = crate::import_owned_xcf_bytes_in_quota(path, Arc::new(bytes), &options, &limits, q.clone(), &ctl).unwrap();
        assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 1);
        let crate::OwnedXcfImport::Point(owned) = owned else { panic!("original Gaussian typed holder") };
        let owner = owned.document();
        assert!(owner.is_readonly_point_storage());
        assert!(owner.quota().shares_ledger(&q));
        let current = owner.document().xcf_state.as_ref().unwrap();
        let old = eager.xcf_state.as_ref().unwrap();
        let node = &current.filter_stacks[0].nodes[0];
        let old_node = &old.filter_stacks[0].nodes[0];
        assert_eq!(node.operation, old_node.operation);
        assert_eq!(node.arguments, old_node.arguments);
        assert_eq!(node.composition, old_node.composition);
        assert_eq!(node.enabled, old_node.enabled);
        assert_eq!(current.filter_masks.len(), old.filter_masks.len());
        for (a, b) in current.filter_masks.iter().zip(&old.filter_masks) {
            assert_eq!(a.bounds, b.bounds);
            assert_eq!(a.surface, b.surface);
        }
        assert_eq!(current.filter_stacks[0].nodes[0].operation.schema_epoch, "xcf-22-stock-gaussian-replace");
        let photocraft_doc::LayerContent::BackedRaster(backed) = &owner.document().layers[0].content else { panic!("native owner") };
        for y in 0..80 {
            for x in 0..96 {
                let mut actual = [0; 4];
                let mut expected = [0; 4];
                backed.checked_view().read_pixel_native_into(x, y, &mut actual, &ctl).unwrap();
                eager.layers[0].surface().unwrap().try_read_pixel_bytes(x, y, &mut expected).unwrap();
                assert_eq!(actual, expected);
            }
        }
        drop(owned);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
