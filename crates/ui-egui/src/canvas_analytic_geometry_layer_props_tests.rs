//! UNRUN registered CURRENT metadata editing over genuine assigned geometry.
//! Existing original256 evidence helper supplies exact pins. Reference/source/
//! serialized Vecs are external harness captures, never source/render grants.
use super::analytic_geometry_tests::{cases, ctl, native, operators, profiles};
use super::*;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use serde_json::{Value, json};
use std::io::Cursor;

fn retained(before: &Document, current: &Document) {
    assert_eq!(native(current), native(before));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let a = before.xcf_state.as_ref().unwrap();
    let b = current.xcf_state.as_ref().unwrap();
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.filter_stacks, b.filter_stacks);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.current_filter_output_precision, b.current_filter_output_precision);
}
// This CURRENT scene has a hidden foreground. The original resident scene
// qualifier deliberately stays closed; reuse only its existing Normal kernel
// for the one visible backdrop, without fabricating an original-source grant.
fn backdrop_kernel_reference(doc: &Document) -> photocraft_compose::Buffer {
    use photocraft_compose::gimp_composite::{WorkingSpace, encoding};
    let layer = doc.layers.iter().find(|layer| layer.mask.is_none()).unwrap();
    assert!(layer.visible);
    assert!(doc.layers.iter().find(|layer| layer.mask.is_some()).is_some_and(|layer| !layer.visible));
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_deref().unwrap();
    let stored = doc.sample_encoding.unwrap();
    let space = if doc.mode == photocraft_doc::ColorMode::Grayscale { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb };
    let profile = photocraft_cms::xcf_color::effective_profile(Some(original), space, encoding(stored)).unwrap();
    let policy = doc
        .xcf_state
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id))
        .unwrap()
        .composition
        .as_ref()
        .unwrap()
        .resolved
        .as_ref()
        .unwrap();
    let working = WorkingSpace::new(&profile, stored, policy.composite_space).unwrap();
    let photocraft_doc::LayerContent::Raster(surface) = &layer.content else { panic!("external resident reference") };
    let channels = surface.format().channels();
    let mut px = Vec::new();
    for y in 0..doc.size.height {
        for x in 0..doc.size.width {
            let mut samples = [0.0; 4];
            surface.try_read_pixel_f64(x as i32, y as i32, &mut samples[..channels]).unwrap();
            let pixel = if doc.mode == photocraft_doc::ColorMode::Grayscale { [samples[0], samples[0], samples[0], samples[1]] } else { samples };
            let output = working.normal([0.0; 4], pixel, f64::from(layer.opacity) * f64::from(layer.fill_opacity), policy.operator).unwrap();
            px.push(output.map(|value| value as f32));
        }
    }
    photocraft_compose::Buffer { rect: doc.bounds(), px }
}

#[test]
#[ignore = "UNRUN:64 genuine left-top source domains, registered combined layerprops cross-layer coalescing/history and pathless native continuation"]
fn assigned_geometry_registered_ui_properties_keep_first_label_and_all_native_owners() {
    let selected: Vec<_> = cases().into_iter().filter(|(root, row)| root.ends_with("left-top") && row["id"].as_str().unwrap().ends_with("-source")).collect();
    assert_eq!(selected.len(), 64);
    for (root, row) in selected {
        let name = row["source_file"].as_str().unwrap();
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
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::AnalyticGeometry(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        profiles(&before, &row);
        operators(&before, &row);
        let foreground = before.layers.iter().find(|layer| layer.mask.is_some()).unwrap().id;
        let backdrop = before.layers.iter().find(|layer| layer.mask.is_none()).unwrap().id;
        let old_backdrop = before.layers.iter().find(|layer| layer.id == backdrop).unwrap();
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.setProps", json!({"layer":foreground.0,"visible":false,"coalesce":"geometry-gesture"})).unwrap();
        app.run("layer.setProps", json!({"layer":backdrop.0,"name":"Backdrop current","opacity":0.375,"coalesce":"geometry-gesture"})).unwrap();
        let changed = app.session.active().unwrap().doc.clone();
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        assert!(!changed.layers.iter().find(|layer| layer.id == foreground).unwrap().visible);
        let current_backdrop = changed.layers.iter().find(|layer| layer.id == backdrop).unwrap();
        assert_eq!(current_backdrop.name, "Backdrop current");
        assert_eq!(current_backdrop.opacity.to_bits(), 0.375f32.to_bits());
        retained(&before, &changed);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        app.run("layer.setProps", json!({"layer":backdrop.0,"name":old_backdrop.name,"visible":old_backdrop.visible,"opacity":f64::from(old_backdrop.opacity),"coalesce":"geometry-gesture"})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        app.run("layer.setProps", json!({"layer":foreground.0,"name":"Foreground current"})).unwrap();
        let edited = app.session.active().unwrap().doc.clone();
        retained(&before, &edited);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 2);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Layer Properties"));
        // Reference reproduces only the CURRENT metadata edits on the old
        // resident consumer. It is not independently authored hidden-source data.
        let mut resident = photocraft_io::import(name, &bytes).unwrap().document;
        for layer in &mut resident.layers {
            if layer.mask.is_some() {
                layer.visible = false;
                layer.name = "Foreground current".into();
            } else {
                layer.name = "Backdrop current".into();
                layer.opacity = 0.375;
            }
        }
        for item in &mut resident.xcf_state.as_mut().unwrap().items {
            if let photocraft_doc::item_identity::ItemRef::Layer(id) = item.item {
                let layer = resident.layers.iter().find(|layer| layer.id == id).unwrap();
                item.name = layer.name.clone();
                item.custom_name = true;
                item.visible = layer.visible;
            }
        }
        assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&resident).is_err(), "original scene qualification is unchanged");
        let reference = backdrop_kernel_reference(&resident);
        let expected = display_image(None, &reference);
        let (display, fee) = backed_snapshot_display_image(&edited, edited.size.width, edited.size.height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels);
        drop((display, fee));
        let query = photocraft_engine::accounted_plain_xcf_inspect::prepare(&app.session, &Value::Null, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 3);
        assert_eq!(query.value()["filters"]["total"], 0);
        drop(query);
        let saved = photocraft_format::owned_document::save_current_document_archive(&edited, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
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
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "properties.pcraft", archive, &ctl()).unwrap();
        let current = reopened.session.active().unwrap();
        assert!(current.path.is_none());
        assert!(!current.source_read_only);
        retained(&edited, &current.doc);
        assert_eq!(current.doc.xcf_state, edited.xcf_state);
        assert_eq!(
            current.doc.layers.iter().map(|layer| (&layer.name, layer.visible, layer.opacity.to_bits())).collect::<Vec<_>>(),
            edited.layers.iter().map(|layer| (&layer.name, layer.visible, layer.opacity.to_bits())).collect::<Vec<_>>()
        );
        let (display, fee) = backed_snapshot_display_image(&current.doc, current.doc.size.width, current.doc.size.height, None, &Interrupt::NONE).unwrap();
        assert_eq!(display.pixels, expected.pixels);
        drop((display, fee));
        reopened.run("layer.setProps", json!({"layer":foreground.0,"visible":true})).unwrap();
        retained(&before, &reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
    }
}
