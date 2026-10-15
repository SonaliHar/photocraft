//! Paid synthetic CURRENT/native geometry, not a producer translation oracle.
#[path = "observed_box_metadata_fixture.rs"]
#[allow(dead_code)]
mod metadata;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
mod native;
use photocraft_doc::{Document, LayerContent, SampleType, accounted_document::AccountedDocument, item_identity::ItemRef};
pub(super) fn ctl() -> photocraft_raster::backing::ReadControl<'static> {
    native::ctl()
}
pub(super) fn archive(owner: &AccountedDocument) -> photocraft_format::owned_document::OwnedDocumentArchive {
    native::archive(owner)
}
pub(super) fn encoded(owner: &AccountedDocument) -> Vec<u8> {
    native::encoded(owner)
}
pub(super) fn owner(sample: SampleType, assigned: bool, locked: bool) -> AccountedDocument {
    let base = metadata::owner(sample, assigned);
    // External harness metadata copy, admitted before allocation. Retain the
    // actual base/native/profile receipts; no naked cloned-model authority.
    let fee = base.quota().reserve(photocraft_raster::backing::quota::QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = base.document().clone();
    doc.layers[0].effects.reference = Some((1.25, -0.0));
    doc.layers[0].locks.position = locked;
    if assigned {
        AccountedDocument::new_readonly_assigned_point_storage(doc, std::sync::Arc::new((base.clone(), fee)), base.quota().clone(), &ctl()).unwrap()
    } else {
        AccountedDocument::new_readonly_point_storage(doc, std::sync::Arc::new((base.clone(), fee)), base.quota().clone(), &ctl()).unwrap()
    }
}
pub(super) fn exact(before: &Document, after: &Document, dx: i32, dy: i32) {
    assert_eq!(before.size, after.size);
    assert_eq!(before.icc_profile, after.icc_profile);
    assert_eq!(before.source_color_context, after.source_color_context);
    assert_eq!(before.working_profile_interpretation, after.working_profile_interpretation);
    assert_eq!(before.sample_encoding, after.sample_encoding);
    let (old, now) = (&before.layers[0], &after.layers[0]);
    assert_eq!(old.id, now.id);
    assert_eq!(old.locks, now.locks);
    assert_eq!(old.blend, now.blend);
    assert_eq!(old.fill_opacity.to_bits(), now.fill_opacity.to_bits());
    let (LayerContent::BackedRaster(a), LayerContent::BackedRaster(b)) = (&old.content, &now.content) else { panic!("native") };
    let r = a.operative_bounds();
    let shifted = photocraft_geom::Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
    assert_eq!(b.operative_bounds(), shifted);
    assert_eq!(a.format(), b.format());
    assert_eq!(a.default_bytes(), b.default_bytes());
    // Logical native words must be identical under the inverse coordinate map;
    // physical root tile hashes need not be identical for unaligned views.
    assert_eq!(native::native(before), native::native(after));
    let expected_reference = old
        .effects
        .reference
        .map(|(x, y)| if (dx, dy) == (0, 0) { (x.to_bits(), y.to_bits()) } else { ((x + f64::from(dx)).to_bits(), (y + f64::from(dy)).to_bits()) });
    assert_eq!(now.effects.reference.map(|(x, y)| (x.to_bits(), y.to_bits())), expected_reference);
    let mut expected = before.xcf_state.clone().unwrap();
    for item in &mut expected.items {
        if item.item == ItemRef::Layer(old.id) {
            item.raster_bounds = Some(shifted);
            if item.name != now.name {
                item.custom_name = true;
                item.name = now.name.clone();
            }
            item.visible = now.visible;
        }
    }
    // This compares the physical effect-mask bounds/offset/flags/native surface,
    // filter node/stack revision/raw opaque fields and provenance unchanged.
    assert_eq!(after.xcf_state, Some(expected));
}
pub(super) fn reference(doc: &Document) -> photocraft_compose::Buffer {
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref();
    let profile = original.map_or_else(|| photocraft_cms::Builtin::Srgb.profile().clone(), |bytes| photocraft_cms::Profile::parse(bytes).unwrap());
    let curves = profile.trc.as_ref().unwrap();
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("native") };
    let literal = native::Literal { bounds: backed.operative_bounds(), raw: native::native(doc), sample: doc.depth };
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let FilterValue::Number(ExactNumber::Signed { width: 32, value: radius }) = node.arguments[0].value else { panic!("radius") };
    // Existing full Surface kernel runs anew over independently shifted raw
    // samples. Never translate a previously filtered bitmap: global256 resets
    // are observable for unaligned moves. All reference buffers are external.
    let mut linear = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Rgb, SampleType::F64, true));
    for y in literal.bounds.y0..literal.bounds.y1 {
        for x in literal.bounds.x0..literal.bounds.x1 {
            let p = native::sample(&literal, x, y);
            linear.try_write_pixel_f64(x, y, &[curves[0].eval64(p[0]), curves[1].eval64(p[1]), curves[2].eval64(p[2]), p[3]]).unwrap();
        }
    }
    let extent = if node.enabled { photocraft_algo::gimp::neighborhood::box_blur_extent(literal.bounds, radius as u32).unwrap() } else { literal.bounds };
    let filtered = node.enabled.then(|| {
        photocraft_algo::gimp::neighborhood::try_box_blur(
            &linear,
            extent,
            literal.bounds,
            radius as u32,
            Default::default(),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap()
    });
    let normal =
        photocraft_compose::gimp_composite::WorkingSpace::new(&profile, photocraft_doc::SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let opacity = if doc.layers[0].visible { f64::from(doc.layers[0].opacity) } else { 0.0 };
    let mut px = Vec::new();
    for y in 0..doc.size.height as i32 {
        for x in 0..doc.size.width as i32 {
            let p = if extent.contains(x, y) {
                if let Some(filtered) = &filtered {
                    let mut p = [0.; 4];
                    filtered.try_read_pixel_f64(x, y, &mut p).unwrap();
                    [curves[0].eval_inverse64(p[0]), curves[1].eval_inverse64(p[1]), curves[2].eval_inverse64(p[2]), p[3]]
                } else {
                    native::sample(&literal, x, y)
                }
            } else {
                [0.; 4]
            };
            px.push(normal.normal([0.; 4], p, opacity, GimpOperator::Union).unwrap().map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: doc.bounds(), px }
}
