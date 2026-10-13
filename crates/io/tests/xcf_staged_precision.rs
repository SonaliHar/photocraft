#[path = "common/xcf.rs"]
mod common;
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, BudgetKind, DecodeOptions, PlainSourceAccounting, ResourceLimits, ResourcePolicy, Source, XcfError};

struct Bytes(Vec<u8>);
impl Source for Bytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, output: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
        let at = usize::try_from(offset).unwrap();
        output.copy_from_slice(&self.0[at..at + output.len()]);
        Ok(())
    }
}
fn normalize_ids(actual: &mut photocraft_doc::Document, expected: &photocraft_doc::Document) {
    use photocraft_doc::item_identity::ItemRef;
    actual.id = expected.id;
    assert_eq!(actual.layers.len(), expected.layers.len());
    for (layer, reference) in actual.layers.iter_mut().zip(&expected.layers) {
        let old = ItemRef::Layer(layer.id);
        let new = ItemRef::Layer(reference.id);
        layer.id = reference.id;
        let state = actual.xcf_state.as_mut().unwrap();
        for item in &mut state.items {
            if item.item == old {
                item.item = new;
            }
        }
        for item in &mut state.selection.selected {
            if *item == old {
                *item = new;
            }
        }
        if state.selection.active == Some(old) {
            state.selection.active = Some(new);
        }
        for record in &mut state.opaque {
            if record.owner == Some(old) {
                record.owner = Some(new);
            }
        }
    }
}
fn check(bytes: Vec<u8>) {
    let original = bytes.clone();
    let parsed = photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap();
    let plane = parsed.layers[0].plane.as_ref().unwrap();
    let expected = photocraft_io::xcf_import::import("own.xcf", &bytes, &Interrupt::NONE).unwrap().document;
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let mut source = Bytes(bytes);
    let actual = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
        "own.xcf",
        &mut source,
        &Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        PlainSourceAccounting::BorrowedMemory,
    )
    .unwrap()
    .document;
    let mut normalized = actual.clone();
    normalize_ids(&mut normalized, &expected);
    assert_eq!(normalized, expected, "whole source/current/native metadata");
    let a = photocraft_compose::gimp_render::try_render(&actual, actual.bounds(), &mut |_| Ok(())).unwrap();
    let b = photocraft_compose::gimp_render::try_render(&expected, expected.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(a.px, b.px, "exact matched current projection");
    assert_eq!(actual.pixel_format(), expected.pixel_format());
    assert_eq!(actual.icc_profile, expected.icc_profile);
    assert_eq!(actual.sample_encoding, expected.sample_encoding);
    assert_eq!(actual.source_color_context, expected.source_color_context);
    let photocraft_doc::LayerContent::Raster(surface) = &actual.layers[0].content else { panic!() };
    let photocraft_doc::LayerContent::Raster(eager) = &expected.layers[0].content else { panic!() };
    assert_eq!(surface, eager);
    let bpp = plane.channels * plane.sample.source_bytes();
    let mut raw = vec![0; bpp];
    for y in 0..plane.height {
        for x in 0..plane.width {
            surface.try_read_pixel_bytes(parsed.layers[0].offset.0 + x as i32, parsed.layers[0].offset.1 + y as i32, &mut raw).unwrap();
            let at = (y as usize * plane.width as usize + x as usize) * bpp;
            assert_eq!(raw, &plane.bytes[at..at + bpp]);
        }
    }
    surface.try_read_pixel_bytes(parsed.layers[0].offset.0 - 1, parsed.layers[0].offset.1, &mut raw).unwrap();
    assert_eq!(raw, vec![0; bpp]);
    let native = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), actual);
    assert_eq!(source.0, original);
    for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
        assert_eq!(budget.used(kind), 0);
    }
}
#[test]
fn wide_staged_exact_native_all_transports_models_and_hidden_color() {
    for gray in [false, true] {
        for precision in [250, 350, 550, 650, 750] {
            let mut pixels = Vec::new();
            for i in 0..65 * 3 {
                for c in 0..if gray { 2 } else { 4 } {
                    let alpha = c == if gray { 1 } else { 3 };
                    let zero = alpha && i % 3 == 0;
                    match precision {
                        250 => pixels.extend(if zero { 0u16 } else { 32769 + c }.to_be_bytes()),
                        350 => pixels.extend(if zero { 0u32 } else { 2_147_483_649 + u32::from(c) }.to_be_bytes()),
                        550 => pixels.extend(
                            if zero {
                                0u16
                            } else if !alpha && c == 0 && i % 2 == 0 {
                                0x8000
                            } else {
                                0x3801
                            }
                            .to_be_bytes(),
                        ),
                        650 => pixels.extend(
                            if zero {
                                0f32
                            } else if !alpha && c == 0 && i % 2 == 0 {
                                -0.0
                            } else {
                                f32::from_bits(0x3f00_0001 + u32::from(c))
                            }
                            .to_be_bytes(),
                        ),
                        750 => pixels.extend(
                            if zero {
                                0f64
                            } else if !alpha && c == 0 && i % 2 == 0 {
                                -0.0
                            } else {
                                f64::from_bits(0x3fe0_0000_0000_0001 + u64::from(c))
                            }
                            .to_be_bytes(),
                        ),
                        _ => panic!(),
                    }
                }
            }
            for compression in [0, 1, 2] {
                let mut offset = Vec::new();
                offset.extend((-17i32).to_be_bytes());
                offset.extend(11i32.to_be_bytes());
                let options = common::FixtureOptions { model: u32::from(gray), layer_properties: vec![(15, offset)], ..Default::default() };
                check(common::image_custom(12, precision, compression, 65, 3, &pixels, &options));
            }
        }
    }
}

#[test]
#[ignore = "requires reviewed unpublished full XCF precision matrix"]
fn original_precision_matrix_staged_exact_native_and_reopen() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS"));
    for model in ["rgb", "gray"] {
        for sample in ["u16", "u32", "half", "float", "double"] {
            let folder = if matches!(sample, "u32" | "double") { root.clone() } else { root.join("precision-matrix") };
            check(std::fs::read(folder.join(format!("{model}-{sample}_non_linear.xcf"))).unwrap());
        }
    }
}

#[test]
fn wide_staged_quota_cancel_and_invalid_float_alpha_refund() {
    for precision in [250, 350, 550, 650, 750] {
        let pixel = match precision {
            250 => [1u16, 32769, 65535, 0].into_iter().flat_map(u16::to_be_bytes).collect::<Vec<_>>(),
            350 => [1u32, 2_147_483_649, u32::MAX, 0].into_iter().flat_map(u32::to_be_bytes).collect(),
            550 => [0x8000u16, 0x3801, 0x3c00, 0].into_iter().flat_map(u16::to_be_bytes).collect(),
            650 => [-0.0f32, f32::from_bits(0x3f00_0001), 1.0, 0.0].into_iter().flat_map(f32::to_be_bytes).collect(),
            750 => [-0.0f64, f64::from_bits(0x3fe0_0000_0000_0001), 1.0, 0.0].into_iter().flat_map(f64::to_be_bytes).collect(),
            _ => panic!(),
        };
        for mode in 0..3 {
            let base = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap();
            let limits = match mode {
                0 => base.lowered(&[("residentBytes", 0)]).unwrap(),
                1 => base.lowered(&[("sampleWorkUnits", 0)]).unwrap(),
                _ => base,
            };
            let cancelled = || mode == 2;
            let budget = Budget::new(limits, Some(&cancelled));
            let mut source = Bytes(common::image(12, precision, 0, 1, 1, &pixel));
            assert!(
                photocraft_io::xcf_import::import_staged_plain_source_with_budget(
                    "own.xcf",
                    &mut source,
                    &Interrupt::NONE,
                    &DecodeOptions::default(),
                    &budget,
                    PlainSourceAccounting::BorrowedMemory
                )
                .is_err()
            );
            for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
                assert_eq!(budget.used(kind), 0);
            }
        }
    }
    for alpha in [-0.5f64, 1.5, f64::NAN, f64::INFINITY] {
        let pixel: Vec<_> = [0.25f64, 0.5, 0.75, alpha].into_iter().flat_map(f64::to_be_bytes).collect();
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let mut source = Bytes(common::image(12, 750, 0, 1, 1, &pixel));
        let error = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            "own.xcf",
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
        )
        .err()
        .unwrap();
        assert!(matches!(error, photocraft_io::IoError::Xcf(ref e) if e.message.contains("alpha") || e.message.contains("nonfinite")), "{error}");
        for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
            assert_eq!(budget.used(kind), 0);
        }
    }
}

#[test]
fn wide_late_provenance_io_and_cancel_keep_surface_then_refund() {
    struct Late<'a> {
        bytes: Vec<u8>,
        stop: &'a std::cell::Cell<bool>,
        cancel: bool,
        held: u64,
    }
    impl Source for Late<'_> {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
            if offset >= 65_536 {
                self.held = budget.used(BudgetKind::Resident);
                if self.cancel {
                    self.stop.set(true);
                } else {
                    return Err(XcfError {
                        kind: photocraft_xcf::ErrorKind::Structure,
                        offset,
                        item: String::new(),
                        message: "own wide late provenance IO".into(),
                    });
                }
            }
            let at = offset as usize;
            out.copy_from_slice(&self.bytes[at..at + out.len()]);
            Ok(())
        }
    }
    for cancel in [false, true] {
        let pixels: Vec<_> = [-0.0f64, f64::from_bits(0x3fe0_0000_0000_0001), 1.0, 0.0].into_iter().flat_map(f64::to_be_bytes).collect();
        let mut bytes = common::image(12, 750, 0, 1, 1, &pixels);
        bytes.extend(vec![0; 131_072]);
        let stop = std::cell::Cell::new(false);
        let cancelled = || stop.get();
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), Some(&cancelled));
        let encoded = budget.reserve(BudgetKind::Encoded, bytes.len() as u64, "own source").unwrap();
        let scratch = budget.reserve(BudgetKind::Scratch, bytes.len() as u64, "own source").unwrap();
        let mut source = Late { bytes, stop: &stop, cancel, held: 0 };
        let error = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            "own.xcf",
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::CallerReservedScratch,
        )
        .err()
        .unwrap();
        assert!(source.held >= 256 * 256 * 32, "wide Surface stays charged until late stamp: {}", source.held);
        assert!(
            if cancel {
                matches!(error, photocraft_io::IoError::Cancelled)
            } else {
                matches!(error, photocraft_io::IoError::Xcf(ref e) if e.message == "own wide late provenance IO")
            },
            "{error}"
        );
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), source.len());
        assert_eq!(budget.used(BudgetKind::Scratch), source.len());
        drop(encoded);
        drop(scratch);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
        assert_eq!(budget.used(BudgetKind::Scratch), 0);
    }
}
