//! UNRUN authored group sources -> synthetic CURRENT metadata/mask controls.
//! Native/source captures remain exact. Changed bitmaps use existing Normal
//! kernels as a CURRENT adapter reference, not independent GIMP evidence.
use super::*;
use crate::canvas::isolated_group_test_support::{cases, ctl, display};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaLimits},
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;
fn ids(doc: &Document) -> Vec<photocraft_doc::LayerId> {
    let group = doc.layers.iter().find(|layer| matches!(layer.content, LayerContent::Group(_))).unwrap();
    let LayerContent::Group(content) = &group.content else { panic!("group") };
    let backdrop = doc.layers.iter().find(|layer| !matches!(layer.content, LayerContent::Group(_))).unwrap();
    vec![group.id, content.children[0].id, content.children[1].id, backdrop.id]
}
fn payload(doc: &Document) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>) {
        for layer in layers {
            let mut digest = Sha256::new();
            digest.update(layer.id.0.to_ne_bytes());
            match &layer.content {
                LayerContent::Group(group) => {
                    digest.update(format!("{:?}", (group.expanded, &group.artboard)).as_bytes());
                    assert_eq!(group.children.len(), 2);
                }
                LayerContent::BackedRaster(backed) => {
                    digest.update(format!("{:?}", backed.descriptor()).as_bytes());
                    digest.update(backed.descriptor().default_bytes());
                    for coord in backed.descriptor().bounds().tiles() {
                        let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                        match pin.encoded() {
                            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
                        }
                    }
                }
                _ => panic!("real group/native leaves only"),
            }
            if let Some(mask) = &layer.mask {
                digest.update(
                    format!("{:?}", (mask.surface.format(), mask.surface.tile_bounds(), mask.linked, mask.density.to_bits(), mask.feather.to_bits()))
                        .as_bytes(),
                );
                digest.update(mask.surface.default_bytes());
                for (coord, tile) in mask.surface.tiles() {
                    digest.update(format!("{coord:?}").as_bytes());
                    digest.update(tile.bytes());
                }
            }
            out.push(digest.finalize().iter().map(|v| format!("{v:02x}")).collect());
            if let LayerContent::Group(group) = &layer.content {
                walk(&group.children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.layers, &mut out);
    assert_eq!(out.len(), 4);
    out
}
fn retained(before: &Document, current: &Document) {
    assert_eq!(payload(before), payload(current));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.sample_encoding, before.sample_encoding);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let a = before.xcf_state.as_ref().unwrap();
    let b = current.xcf_state.as_ref().unwrap();
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.filter_stacks, b.filter_stacks);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.current_filter_output_precision, b.current_filter_output_precision);
    for id in ids(current) {
        let layer = current.layer(id).unwrap();
        let item = b.items.iter().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(id)).unwrap();
        assert_eq!(item.name, layer.name);
        assert_eq!(item.visible, layer.visible);
        let old = a.items.iter().find(|old| old.item == item.item).unwrap();
        assert_eq!(item.raster_bounds, old.raster_bounds);
        assert_eq!(item.composition, old.composition);
        assert_eq!(item.parent, old.parent);
    }
}
// Reuse only the existing Normal kernel over the current raw leaves. This
// external harness does not invoke or widen the old original-scene qualifier.
fn kernel_reference(doc: &Document) -> photocraft_compose::Buffer {
    use photocraft_compose::gimp_composite::{WorkingSpace, encoding};
    use photocraft_doc::import_compositing::{GimpOperator, GimpSpace};
    fn at(doc: &Document, layers: &[Layer], x: i32, y: i32, working: &WorkingSpace) -> [f64; 4] {
        let mut output = [0.; 4];
        for layer in layers {
            if !layer.visible || layer.opacity == 0. {
                continue;
            }
            let item = doc.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap();
            let rect = item.raster_bounds.unwrap();
            if !rect.contains(x, y) {
                continue;
            }
            let source = match &layer.content {
                LayerContent::Group(group) => at(doc, &group.children, x, y, working),
                LayerContent::BackedRaster(backed) => {
                    let view = backed.checked_view();
                    let mut sample = [0.; 4];
                    view.read_pixel_f64_into(x, y, &mut sample[..view.format().channels()], &ctl()).unwrap();
                    if doc.mode == photocraft_doc::ColorMode::Grayscale { [sample[0], sample[0], sample[0], sample[1]] } else { sample }
                }
                _ => panic!("native current reference"),
            };
            let coverage = if let Some(mask) = layer.mask.as_ref().filter(|mask| mask.enabled) {
                let mut value = [0.];
                mask.surface.try_read_pixel_f64(x, y, &mut value).unwrap();
                value[0]
            } else {
                1.
            };
            output = working.normal(output, source, f64::from(layer.opacity) * coverage, GimpOperator::Union).unwrap();
        }
        output
    }
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref().unwrap();
    let space = if doc.mode == photocraft_doc::ColorMode::Grayscale { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
    let profile = photocraft_cms::xcf_color::effective_profile(Some(original), space, encoding(doc.sample_encoding.unwrap())).unwrap();
    let working = WorkingSpace::new(&profile, doc.sample_encoding.unwrap(), GimpSpace::LinearRgb).unwrap();
    let mut px = Vec::new();
    for y in 0..doc.size.height {
        for x in 0..doc.size.width {
            px.push(at(doc, &doc.layers, x as i32, y as i32, &working).map(|v| v as f32));
        }
    }
    photocraft_compose::Buffer { rect: doc.bounds(), px }
}
fn image_matches_kernel(snapshot: &photocraft_engine::DocumentSnapshot) {
    let reference = crate::canvas::display_image(None, &kernel_reference(snapshot));
    let (actual, fee) = display(snapshot).unwrap();
    assert_eq!(actual.pixels, reference.pixels);
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN:24 actual group sources; all four CURRENT layer targets, coalescing, guarded mask control, native continuation and existing Normal kernel reference"]
fn group_controls_keep_children_native_and_source_roles() {
    for (root, row) in cases() {
        let name = row["source"]["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
            name,
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let targets = ids(&before);
        let group = targets[0];
        let backdrop = targets[3];
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.setProps", json!({"layer":group.0,"visible":!before.layer(group).unwrap().visible,"coalesce":"group-gesture"})).unwrap();
        for (index, id) in targets.iter().enumerate() {
            app.run("layer.setProps",json!({"layer":id.0,"name":format!("Group current {index}"),"visible":index==3,"opacity":if index==3 {0.375} else {0.5},"coalesce":"group-gesture"})).unwrap();
        }
        let changed = app.session.active().unwrap().doc.clone();
        retained(&before, &changed);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        for (index, id) in targets.iter().enumerate() {
            let layer = changed.layer(*id).unwrap();
            assert_eq!(layer.name, format!("Group current {index}"));
            assert_eq!(layer.visible, index == 3);
            assert_eq!(layer.opacity.to_bits(), if index == 3 { 0.375f32.to_bits() } else { 0.5f32.to_bits() });
        }
        image_matches_kernel(&changed);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        let old = before.layer(backdrop).unwrap();
        app.run(
            "layer.setProps",
            json!({"layer":backdrop.0,"name":old.name,"visible":old.visible,"opacity":f64::from(old.opacity),"coalesce":"group-gesture"}),
        )
        .unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        if changed.layer(group).unwrap().mask.is_some() {
            let target = capture(&app, group).unwrap();
            assert!(permitted(&app, &target));
            let desired = !target.enabled;
            let journal = app.session.journal.len();
            let enabled = target.enabled;
            let reply = apply(&mut app, &target, desired).unwrap();
            assert_eq!(reply, json!({"maskId":target.mask.0.to_string(),"layerId":group.0.to_string(),"enabled":desired,"changed":true}));
            assert_eq!(app.session.journal.len(), journal);
            retained(&before, &app.session.active().unwrap().doc);
            assert_eq!(apply(&mut app, &target, desired).unwrap_err(), stale());
            app.run("edit.undo", json!({})).unwrap();
            let history = app.session.active().unwrap().history.clone();
            let noop = capture(&app, group).unwrap();
            assert!(!apply(&mut app, &noop, enabled).unwrap()["changed"].as_bool().unwrap());
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            app.run("edit.redo", json!({})).unwrap();
            let current = capture(&app, group).unwrap();
            if current.enabled {
                apply(&mut app, &current, false).unwrap();
            }
            let current = app.session.active().unwrap().doc.clone();
            assert!(!current.layer(group).unwrap().mask.as_ref().unwrap().enabled);
            let mask = current
                .xcf_state
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find_map(|item| item.mask.as_ref().filter(|mask| mask.owner == ItemRef::Layer(group)))
                .unwrap();
            assert!(!mask.enabled);
        }
        let current = app.session.active().unwrap().doc.clone();
        retained(&before, &current);
        image_matches_kernel(&current);
        let renamed = current.accounted().unwrap().try_with_document_name("Group controls native", &ctl()).unwrap();
        let saved =
            photocraft_format::owned_document::save_current_document_archive(renamed.document(), Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
                .unwrap()
                .into_inner();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(saved.as_slice()),
            saved.len() as u64,
            Default::default(),
            quota.clone(),
            &ctl(),
        )
        .unwrap();
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "group-controls.pcraft", archive, &ctl()).unwrap();
        let state = reopened.session.active().unwrap();
        assert!(!state.source_read_only);
        assert!(state.path.is_none());
        assert_eq!(state.doc.name, "Group controls native");
        assert_eq!(state.doc.xcf_state, current.xcf_state);
        retained(&before, &state.doc);
        image_matches_kernel(&state.doc);
        // Restore activity through registered CURRENT commands; no hidden/disabled authored-source claim.
        for id in targets.iter().take(3) {
            reopened.run("layer.setProps", json!({"layer":id.0,"visible":true})).unwrap();
        }
        image_matches_kernel(&reopened.session.active().unwrap().doc);
        if let Some(target) = capture(&reopened, group) {
            assert!(!target.enabled);
            apply(&mut reopened, &target, true).unwrap();
            image_matches_kernel(&reopened.session.active().unwrap().doc);
        }
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
}
