//! UNRUN data-only mixed Exposure custody. Source/capture DTO literals below
//! reference Sol6's support file without compiling any upward-layer code.
//! Native words/ICC/source properties are observed data; synthetic canonical IDs,
//! revisions/default padding and declared CURRENT service epochs are not a source
//! import, generated-profile equality, evaluator or numerical acceptance claim.
use super::*;
use serde_json::Value;
struct Case {
    stem: &'static str,
    source_bytes: usize,
    source_sha: &'static str,
    capture_bytes: usize,
    precision: u32,
}
// Literal DTOs from engine/accounted_filtered_flat_exposure_test_support.rs.
const CASES: [Case; 2] = [
    Case {
        stem: "f32-plain-exposure-ordered-point",
        source_bytes: 8051,
        source_sha: "b41ff7782734b803494e43a0274f0789757aa67902e335933455e4c58179ca2c",
        capture_bytes: 93477,
        precision: 650,
    },
    Case {
        stem: "f64-plain-exposure-ordered-point",
        source_bytes: 8113,
        source_sha: "ffa38184286dfa0ef1462223bb55e217822bed5595b350af0ff7ce786c71f015",
        capture_bytes: 93565,
        precision: 750,
    },
];
struct CaptureStore {
    descriptor: NativePlaneDescriptor,
    bytes: Arc<Vec<u8>>,
    quota: BackingQuota,
    _guard: Arc<Guard>,
}
impl std::fmt::Debug for CaptureStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureStore").field("descriptor", &self.descriptor).finish_non_exhaustive()
    }
}
impl NativePlaneStore for CaptureStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let start = usize::try_from(offset).map_err(|_| BackingError::Length)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Length)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
struct CaptureRetention {
    source: Arc<Vec<u8>>,
    _guard: Arc<Guard>,
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes().as_chunks::<2>().0.iter().map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}
fn properties(out: &mut Vec<OpaqueRecord>, row: &Value, owner: ItemRef) {
    for (index, p) in row.as_array().unwrap().iter().enumerate() {
        let bytes = Arc::new(hex(p["hex"].as_str().unwrap()));
        out.push(OpaqueRecord {
            owner: Some(owner),
            namespace: "gimp/xcf/source-property".into(),
            identity: format!("{}@{}:{}", p["tag"].as_u64().unwrap(), p["offset"].as_u64().unwrap(), bytes.len()),
            source_version: 22,
            order: index as u32,
            flags: 0,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
    }
}
fn argument(p: &Value) -> FilterArgument {
    let raw = hex(p["hex"].as_str().unwrap());
    let n = u32::from_be_bytes(raw[..4].try_into().unwrap()) as usize;
    assert_eq!(raw[4 + n - 1], 0);
    let name = std::str::from_utf8(&raw[4..4 + n - 1]).unwrap().to_owned();
    let typ = u32::from_be_bytes(raw[4 + n..8 + n].try_into().unwrap());
    let mut cursor = 8 + n;
    let value = if typ == 3 {
        assert_eq!(raw.len(), cursor + 4);
        FilterValue::Number(ExactNumber::Float32Bits(u32::from_be_bytes(raw[cursor..cursor + 4].try_into().unwrap())))
    } else {
        assert_eq!(typ, 8);
        let len = u32::from_be_bytes(raw[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let format = std::str::from_utf8(&raw[cursor..cursor + len - 1]).unwrap();
        assert_eq!(format, "RGBA float");
        cursor += len;
        assert_eq!(u32::from_be_bytes(raw[cursor..cursor + 4].try_into().unwrap()), 16);
        cursor += 4;
        let components = raw[cursor..cursor + 16].as_chunks::<4>().0.iter().map(|p| ExactNumber::Float32Bits(u32::from_le_bytes(*p))).collect();
        cursor += 16;
        assert_eq!(u32::from_be_bytes(raw[cursor..cursor + 4].try_into().unwrap()), 0);
        assert_eq!(raw.len(), cursor + 4);
        FilterValue::Color { components, encoding: SampleEncoding::Linear, profile: None }
    };
    FilterArgument { name, value }
}
fn epoch(name: &str) -> &'static str {
    match name {
        "gegl:exposure" => "xcf22-gegl-exposure-mixed-wide-v1",
        "gimp:semi-flatten" => "xcf22-gimp-semi-flatten-literal-v1",
        "gegl:hue-chroma" => "xcf22-gegl-hue-chroma-lch-float-v1",
        "gegl:vibrance" => "xcf22-gegl-vibrance-linear-lch-float-v1",
        _ => panic!("unexpected captured family"),
    }
}
fn fixture(case: &Case) -> (AccountedDocument, Arc<Vec<u8>>) {
    let sample = if case.precision == 650 { SampleType::F32 } else { SampleType::F64 };
    let q = quota();
    // Real16MiB Index/Resident preholds precede file buffers, JSON/strings, native
    // byte owners, model/opaque/profile replacement and retention allocations.
    let (mut d, g) = ordered_fixture(&q, sample, &[0, 1, 4, 4]);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-filtered-flat-exposure-gimp-20261016/author-v2/outputs");
    let source = Arc::new(std::fs::read(dir.join(format!("{}.xcf", case.stem))).unwrap());
    assert_eq!(source.len(), case.source_bytes);
    let json = std::fs::read(dir.join(format!("{}.case.json", case.stem))).unwrap();
    assert_eq!(json.len(), case.capture_bytes);
    let capture: Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(capture["raw"]["precision"], case.precision);
    assert_eq!(capture["raw"]["canvas"], serde_json::json!([8, 6]));
    let read = |blob: &Value| {
        let b = std::fs::read(dir.join(blob["file"].as_str().unwrap())).unwrap();
        assert_eq!(b.len(), blob["bytes"].as_u64().unwrap() as usize);
        b
    };
    let icc = read(&capture["stages"]["live"]["profiles"]["effective"]);
    assert!(capture["stages"]["live"]["profiles"]["assigned"].is_null());
    d.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: blake3::hash(&icc).to_hex().to_string() };
    d.icc_profile = Some(Arc::new(icc));
    d.size = Size::new(8, 6);
    let bounds = d.bounds();
    let state = d.xcf_state.as_mut().unwrap();
    state.provenance.source_hash = Some(case.source_sha.into());
    state.opaque.clear();
    for (index, row) in capture["raw"]["roots"].as_array().unwrap().iter().rev().enumerate() {
        let layer = &mut d.layers[index];
        layer.name = row["name"].as_str().unwrap().into();
        layer.visible = true;
        layer.opacity = 1.;
        let native = Arc::new(read(&row["stored_source"]));
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        assert_eq!(native.len(), 8 * 6 * format.bytes_per_pixel());
        let descriptor = NativePlaneDescriptor::new(format, bounds, &vec![0; format.bytes_per_pixel()]).unwrap();
        let raster =
            BackedRaster::new(Arc::new(CaptureStore { descriptor, bytes: native, quota: q.clone(), _guard: g.clone() }), q.clone(), 1, &ctl()).unwrap();
        layer.content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
        let item = state.items.iter_mut().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap();
        item.name = layer.name.clone();
        item.visible = true;
        item.raster_bounds = Some(bounds);
        properties(&mut state.opaque, &row["properties"], ItemRef::Layer(layer.id));
        let stack_index = state.filter_stacks.iter().position(|s| s.owner == ItemRef::Layer(layer.id));
        let effects = row["effects"].as_array().unwrap();
        if effects.is_empty() {
            assert!(stack_index.is_none());
            continue;
        }
        let si = stack_index.unwrap();
        assert_eq!(state.filter_stacks[si].nodes.len(), effects.len());
        for (ei, effect) in effects.iter().enumerate() {
            let node = &mut state.filter_stacks[si].nodes[ei];
            let name = effect["strings"][2].as_str().unwrap();
            node.operation =
                OperationIdentity { name: name.into(), stored_version: effect["strings"][3].as_str().unwrap().into(), schema_epoch: epoch(name).into() };
            node.source_display_name = Some(effect["strings"][0].as_str().unwrap().into());
            node.source_icon = Some(effect["strings"][1].as_str().unwrap().into());
            node.arguments = effect["properties"].as_array().unwrap().iter().filter(|p| p["tag"] == 45).map(argument).collect();
            let property = |tag| {
                u32::from_be_bytes(
                    hex(effect["properties"].as_array().unwrap().iter().find(|p| p["tag"] == tag).unwrap()["hex"].as_str().unwrap()).try_into().unwrap(),
                )
            };
            node.enabled = property(8) != 0;
            node.opacity_f64_bits = f64::from(f32::from_bits(property(33))).to_bits();
            node.source_clipping = Some(property(46));
            node.source_region = Some(property(44));
            let composition = node.composition.as_mut().unwrap();
            composition.compatibility_epoch = epoch(name).into();
            composition.source_mode = property(7) as i32;
            composition.stored_blend_space = property(37) as i32;
            composition.stored_composite_space = property(36) as i32;
            composition.stored_operator = property(35) as i32;
            let Some(ItemRef::Mask(mid)) = node.mask else { panic!() };
            properties(&mut state.opaque, &effect["properties"], ItemRef::Layer(layer.id));
            properties(&mut state.opaque, &effect["mask"]["properties"], ItemRef::Mask(mid));
            let bytes = read(&effect["mask"]["stored_native"]);
            assert!(bytes.iter().all(|v| *v == 0));
            let m = state.filter_masks.iter_mut().find(|m| m.id == mid).unwrap();
            m.bounds = bounds;
            m.surface = Surface::try_with_default_bytes(m.surface.format(), &vec![0; m.surface.format().bytes_per_pixel()]).unwrap();
            assert_eq!(m.surface.try_read_interleaved(bounds).unwrap(), bytes);
            let item = state.items.iter_mut().find(|i| i.item == ItemRef::Mask(mid)).unwrap();
            item.raster_bounds = Some(bounds);
            item.mask.as_mut().unwrap().bounds = bounds;
        }
    }
    assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
    let retention = Arc::new(CaptureRetention { _guard: g, source: source.clone() });
    (AccountedDocument::new_readonly_filtered_flat_point_current_storage(d, retention, q, &ctl()).unwrap(), source)
}
fn native(layer: &crate::Layer) -> Vec<u8> {
    let LayerContent::BackedRaster(view) = &layer.content else { panic!() };
    let pin = view.checked_view().pin_tile(TileCoord::new(0, 0), &ctl()).unwrap();
    let mut out = Vec::new();
    let mut pixel = vec![0; view.descriptor().format().bytes_per_pixel()];
    for y in 0..6 {
        for x in 0..8 {
            pin.read_pixel_native_into(x, y, &mut pixel, &ctl()).unwrap();
            out.extend_from_slice(&pixel);
        }
    }
    out
}
#[test]
#[ignore = "UNRUN: actual Exposuremixed DTO/native words with genuinely paid synthetic CURRENT owners; nonfirst args/enable/move/remove toempty preserve IDs/masks/archives/ICC/sourcebytes, no source/evaluator qualification"]
fn captured_exposure_nonfirst_target_and_empty_continuation() {
    for case in CASES {
        let (owner, source) = fixture(&case);
        let native_before = owner.document().layers.iter().map(native).collect::<Vec<_>>();
        let s = owner.document().xcf_state.as_ref().unwrap();
        let stack = &s.filter_stacks[1];
        let node = &stack.nodes[3];
        assert_eq!(node.operation.name, "gegl:exposure");
        assert_eq!(node.operation.stored_version, "1:0");
        let ItemRef::Layer(layer) = stack.owner else { panic!() };
        let fid = node.id;
        let mask = node.mask;
        assert_eq!(node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["black-level", "exposure"]);
        let args = [
            PointArgumentPatch { name: "exposure", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) },
            PointArgumentPatch { name: "black-level", value: PointArgumentValue::Float32Bits(0.125f32.to_bits()) },
        ];
        let patch = PointFilterPatch { layer, filter: fid, expected_stack_revision: stack.revision, arguments: &args, enabled: Some(false) };
        let changed = owner.try_with_readonly_filtered_flat_point_filter(patch, &ctl()).unwrap();
        let s = changed.document().xcf_state.as_ref().unwrap();
        assert_eq!(s.filter_stacks[1].revision, 18);
        assert_eq!(s.filter_stacks[1].nodes[3].id, fid);
        assert_eq!(s.filter_stacks[1].nodes[3].mask, mask);
        assert!(!s.filter_stacks[1].nodes[3].enabled);
        let moved = changed
            .try_move_readonly_filtered_flat_point_filter(PointFilterMovePatch { layer, filter: fid, expected_stack_revision: 18, index: 1 }, &ctl())
            .unwrap();
        assert_eq!(moved.document().xcf_state.as_ref().unwrap().filter_stacks[1].nodes[1].id, fid);
        let mut current =
            moved.try_remove_readonly_filtered_flat_point_filter(PointFilterRemovePatch { layer, filter: fid, expected_stack_revision: 19 }, &ctl()).unwrap();
        for si in 0..3 {
            loop {
                let stack = &current.document().xcf_state.as_ref().unwrap().filter_stacks[si];
                let Some(node) = stack.nodes.first() else { break };
                let ItemRef::Layer(layer) = stack.owner else { panic!() };
                current = current
                    .try_remove_readonly_filtered_flat_point_filter(
                        PointFilterRemovePatch { layer, filter: node.id, expected_stack_revision: stack.revision },
                        &ctl(),
                    )
                    .unwrap();
            }
        }
        let titled = current.try_with_document_name("captured-exposure-current.pcraft", &ctl()).unwrap();
        let s = titled.document().xcf_state.as_ref().unwrap();
        assert_eq!(s.filter_stacks.len(), 3);
        assert!(s.filter_stacks.iter().all(|s| s.nodes.is_empty()));
        assert!(s.filter_masks.is_empty());
        assert_eq!(s.items.len(), 4);
        assert_eq!(titled.document().layers.iter().map(native).collect::<Vec<_>>(), native_before);
        assert!(Arc::ptr_eq(&titled.owner.retention, &owner.owner.retention));
        assert!(Arc::ptr_eq(titled.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        assert_eq!(titled.document().source_color_context, owner.document().source_color_context);
        let retained = titled.owner.retention.downcast_ref::<CaptureRetention>().unwrap();
        assert!(Arc::ptr_eq(&retained.source, &source));
        assert_eq!(retained.source.as_slice(), source.as_slice());
        for (a, b) in owner.document().xcf_state.as_ref().unwrap().opaque.iter().zip(&s.opaque) {
            assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
            assert_eq!((&a.blake3, a.order, a.flags), (&b.blake3, b.order, b.flags));
            if let Some(ItemRef::Mask(id)) = a.owner {
                assert!(b.owner.is_none());
                assert_eq!(b.identity, format!("deleted-filter-mask/{}/{}", id.0, a.identity));
            } else {
                assert_eq!(a, b);
            }
        }
        assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(titled.document(), titled.quota(), &ctl()).is_err());
        titled.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
        // External comparison alias ends before the last paid retention owner.
        drop(source);
    }
}
