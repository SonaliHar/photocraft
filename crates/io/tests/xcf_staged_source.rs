#[path = "common/xcf.rs"]
mod common;
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, BudgetKind, DecodeOptions, PlainSourceAccounting, ResourceLimits, ResourcePolicy, Source, XcfError};
struct Bytes(Vec<u8>);
impl Source for Bytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], _: &Budget<'_>) -> Result<(), XcfError> {
        let start = usize::try_from(offset).unwrap();
        out.copy_from_slice(&self.0[start..start + out.len()]);
        Ok(())
    }
}
#[test]
fn staged_plain_source_avoids_eager_plane_surface_overlap() {
    let pixels: Vec<u8> = (0..512 * 512).flat_map(|i| [i as u8, (i / 512) as u8, 19, if i % 7 == 0 { 0 } else { 127 }]).collect();
    let encoded = common::image(12, 150, 0, 512, 512, &pixels);
    let limits = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap().lowered(&[("residentBytes", 2_400_000)]).unwrap();
    let budget = Budget::new(limits, None);
    let _encoded = budget.reserve(BudgetKind::Encoded, encoded.len() as u64, "test source").unwrap();
    let _scratch = budget.reserve(BudgetKind::Scratch, encoded.len() as u64, "test source").unwrap();
    let mut source = Bytes(encoded);
    let eager = photocraft_io::xcf_import::import_plain_source_with_budget(
        "own.xcf",
        &mut source,
        &Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        PlainSourceAccounting::CallerReservedScratch,
    );
    assert!(
        matches!(eager, Err(photocraft_io::IoError::Xcf(ref e)) if e.kind == photocraft_xcf::ErrorKind::Limit && e.message.contains("resident")),
        "meaningful eager resident overlap control: {eager:?}"
    );
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    let staged = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
        "own.xcf",
        &mut source,
        &Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        PlainSourceAccounting::CallerReservedScratch,
    )
    .unwrap();
    let photocraft_doc::LayerContent::Raster(surface) = &staged.document.layers[0].content else { panic!() };
    let mut actual = [0; 4];
    surface.try_read_pixel_bytes(511, 511, &mut actual).unwrap();
    assert_eq!(actual, &pixels[pixels.len() - 4..]);
    assert_eq!(budget.used(BudgetKind::Resident), 0, "ownership transferred only at return");
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
#[test]
fn eager_staged_rgb_gray_offsets_hidden_colors_native_and_defaults_match() {
    for gray in [false, true] {
        let channels = if gray { 2 } else { 4 };
        let width = 65;
        let height = 67;
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                if gray { vec![(i % 251) as u8, if i % 3 == 0 { 0 } else { 129 }] } else { vec![(i % 251) as u8, 93, 17, if i % 3 == 0 { 0 } else { 129 }] }
            })
            .collect();
        let mut offset = Vec::new();
        offset.extend((-17i32).to_be_bytes());
        offset.extend(11i32.to_be_bytes());
        let options = common::FixtureOptions { model: u32::from(gray), layer_properties: vec![(15, offset)], ..Default::default() };
        let encoded = common::image_custom(12, 150, 0, width, height, &pixels, &options);
        let expected = photocraft_io::xcf_import::import("own.xcf", &encoded, &Interrupt::NONE).unwrap().document;
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let mut source = Bytes(encoded);
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
        assert_eq!(normalized, expected);
        let photocraft_doc::LayerContent::Raster(surface) = &actual.layers[0].content else { panic!() };
        let mut raw = vec![0; channels];
        surface.try_read_pixel_bytes(-17, 11, &mut raw).unwrap();
        assert_eq!(raw, &pixels[..channels]);
        surface.try_read_pixel_bytes(-18, 11, &mut raw).unwrap();
        assert_eq!(raw, vec![0; channels]);
        let native = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), actual);
        for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
            assert_eq!(budget.used(kind), 0);
        }
    }
}
struct Late<'a> {
    source: Bytes,
    size: u64,
    fail: bool,
    stop: Option<&'a std::sync::atomic::AtomicBool>,
    minimum_retained: u64,
}
impl Source for Late<'_> {
    fn len(&self) -> u64 {
        self.size
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        if offset >= 65536 {
            self.minimum_retained = self.minimum_retained.min(budget.used(BudgetKind::Resident));
            if let Some(stop) = self.stop {
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if self.fail {
                return Err(XcfError { kind: photocraft_xcf::ErrorKind::Structure, offset, item: String::new(), message: "own late hash failure".into() });
            }
        }
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = self.source.0.get(offset as usize + i).copied().unwrap_or(0);
        }
        Ok(())
    }
}
#[test]
fn staged_late_hash_error_cancel_and_success_hold_surface_until_handoff() {
    for mode in 0..3 {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let cancelled = || stop.load(std::sync::atomic::Ordering::SeqCst);
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), Some(&cancelled));
        let mut source = Late {
            source: Bytes(common::image(12, 150, 0, 1, 1, &[19, 41, 87, 0])),
            size: 131072,
            fail: mode == 0,
            stop: (mode == 1).then_some(&stop),
            minimum_retained: u64::MAX,
        };
        let encoded = budget.reserve(BudgetKind::Encoded, source.size, "own encoded").unwrap();
        let scratch = budget.reserve(BudgetKind::Scratch, source.size, "own scratch").unwrap();
        let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            "late.xcf",
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::CallerReservedScratch,
        );
        match mode {
            0 => assert!(matches!(result,Err(photocraft_io::IoError::Xcf(e)) if e.message.contains("own late hash"))),
            1 => assert!(matches!(result, Err(photocraft_io::IoError::Cancelled))),
            _ => assert!(result.is_ok()),
        }
        // One current RGBA8 COW256tile remains resident during late provenance reads.
        assert!(source.minimum_retained >= 256 * 256 * 4);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), source.size);
        assert_eq!(budget.used(BudgetKind::Scratch), source.size);
        drop(encoded);
        drop(scratch);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
        assert_eq!(budget.used(BudgetKind::Scratch), 0);
    }
}
#[test]
fn staged_unsupported_families_and_missing_quotas_never_retry_eager() {
    let pixels = [10, 20, 30, 255];
    for assigned in [false, true] {
        let mut options = common::FixtureOptions::default();
        if assigned {
            options.image_properties.push((20, vec![0; 140]));
        } else {
            options.layer_type = Some(0);
        }
        let encoded = common::image_custom(12, 150, 0, 1, 1, if assigned { &pixels } else { &pixels[..3] }, &options);
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            "unsupported.xcf",
            &mut Bytes(encoded),
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
        );
        assert!(result.is_err());
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
    }
}

#[test]
#[cfg(feature = "corpus")]
fn eligible_gimp_originals_match_eager_native_current_and_projection() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf");
    for name in [
        "rgb-u8_non_linear.xcf",
        "gray-u8_non_linear.xcf",
        "rgb-u8-basic.xcf",
        "rgb-u8-layers.xcf",
        "rgb-u8-hidden.xcf",
        "rgb-u8-locks.xcf",
        "rgb-u8-guides.xcf",
        "rgb-u8-none.xcf",
        "rgb-u8-zlib.xcf",
        // GIMP omits redundant built-in assignment; source ICC is absent despite name.
        "rgb-u8-explicit-profile.xcf",
        "legacy210-gray-basic.xcf",
        "legacy210-rgb-basic.xcf",
        "legacy28-gray-basic.xcf",
        "legacy28-rgb-basic.xcf",
    ] {
        let bytes = std::fs::read(root.join(name)).expect("run cargo xtask corpus --xcf");
        let eager = photocraft_io::xcf_import::import(name, &bytes, &Interrupt::NONE).unwrap().document;
        if name == "rgb-u8-explicit-profile.xcf" {
            assert!(photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap().icc.is_empty(), "actual absent source ICC");
        }
        let source_hash = blake3::hash(&bytes);
        let mut source = Bytes(bytes);
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let staged = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            name,
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .document;
        assert_eq!(source_hash, blake3::hash(&source.0));
        let mut normalized = staged.clone();
        normalize_ids(&mut normalized, &eager);
        assert_eq!(normalized, eager, "{name}: whole document source/current/metadata");
        let native = photocraft_format::save_to_bytes(&staged, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), staged);
        let a = photocraft_compose::gimp_render::try_render(&staged, staged.bounds(), &mut |_| Ok(())).unwrap();
        let b = photocraft_compose::gimp_render::try_render(&eager, eager.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(a.px, b.px, "{name}: exact current projection parity");
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
    }
}

struct Counted {
    bytes: Vec<u8>,
    reads: usize,
    fail_at: Option<u64>,
    held_at_failure: u64,
}
impl Source for Counted {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8], budget: &Budget<'_>) -> Result<(), XcfError> {
        self.reads += 1;
        if self.fail_at.is_some_and(|limit| offset >= limit) {
            self.held_at_failure = budget.used(BudgetKind::Resident);
            return Err(XcfError { kind: photocraft_xcf::ErrorKind::Structure, offset, item: String::new(), message: "own late tile read failure".into() });
        }
        let start = offset as usize;
        out.copy_from_slice(&self.bytes[start..start + out.len()]);
        Ok(())
    }
}
#[test]
fn late_tile_io_discards_partially_built_surface_before_refunds() {
    let pixels = vec![81; 512 * 512 * 4];
    let bytes = common::image(12, 150, 0, 512, 512, &pixels);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
    let encoded = budget.reserve(BudgetKind::Encoded, bytes.len() as u64, "own encoded").unwrap();
    let scratch = budget.reserve(BudgetKind::Scratch, bytes.len() as u64, "own scratch").unwrap();
    let mut source = Counted { bytes, reads: 0, fail_at: Some(32768), held_at_failure: 0 };
    let error = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
        "late.xcf",
        &mut source,
        &Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        PlainSourceAccounting::CallerReservedScratch,
    )
    .err()
    .unwrap();
    assert!(matches!(error,photocraft_io::IoError::Xcf(e) if e.message.contains("own late tile")));
    assert!(source.held_at_failure >= 512 * 512 * 4);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), source.len());
    assert_eq!(budget.used(BudgetKind::Scratch), source.len());
    drop(encoded);
    drop(scratch);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Scratch), 0);
}
#[test]
fn staged_missing_reservations_zero_resident_work_and_cancel_fail_atomically() {
    for mode in 0..4 {
        let base = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap();
        let limits = match mode {
            1 => base.lowered(&[("residentBytes", 0)]).unwrap(),
            2 => base.lowered(&[("sampleWorkUnits", 0)]).unwrap(),
            _ => base,
        };
        let stop = || mode == 3;
        let budget = Budget::new(limits, Some(&stop));
        let mut source = Counted { bytes: common::image(12, 150, 0, 1, 1, &[11, 33, 97, 0]), reads: 0, fail_at: None, held_at_failure: 0 };
        let accounting = if mode == 0 { PlainSourceAccounting::CallerReservedScratch } else { PlainSourceAccounting::BorrowedMemory };
        let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            "failure.xcf",
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            accounting,
        );
        assert!(result.is_err());
        if mode != 2 {
            assert_eq!(source.reads, 0);
        }
        for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
            assert_eq!(budget.used(kind), 0);
        }
    }
}

#[test]
#[cfg(feature = "corpus")]
fn richer_gimp_source_cache_mask_profile_and_filter_families_stay_eager() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf");
    for name in ["rgb-u8-mask.xcf", "rgb-u8-group.xcf", "rgb-u8-wide-profile.xcf", "rgb-u8-selection.xcf", "unsupported-gegl-effect.xcf"] {
        let bytes = std::fs::read(root.join(name)).expect("run cargo xtask corpus --xcf");
        if name == "rgb-u8-wide-profile.xcf" {
            assert!(!photocraft_xcf::decode(&bytes, &DecodeOptions::default()).unwrap().icc.is_empty(), "actual source ICC presence, never filename inference");
        }
        assert!(photocraft_io::xcf_import::import(name, &bytes, &Interrupt::NONE).is_ok(), "{name}: existing eager admission retained");
        let mut source = Bytes(bytes);
        let before = blake3::hash(&source.0);
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None);
        let result = photocraft_io::xcf_import::import_staged_plain_source_with_budget(
            name,
            &mut source,
            &Interrupt::NONE,
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
        );
        assert!(result.is_err(), "{name}: richer source must not enter staged plain family");
        assert_eq!(blake3::hash(&source.0), before);
        for kind in [BudgetKind::Resident, BudgetKind::Encoded, BudgetKind::Scratch] {
            assert_eq!(budget.used(kind), 0, "{name}");
        }
    }
}
