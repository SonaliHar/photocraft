//! UNRUN actual guarded mask control on independently authored enabled sources.
//! Disabled canvas captures are synthetic CURRENT persistence references, not
//! disabled GIMP evidence. Harness source/archive Vecs are external captures.
use super::*;
use crate::canvas::assigned_geometry_test_support::{cases, ctl, display, plane_words, profiles};
use photocraft_doc::Document;
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::io::Cursor;

fn retained(before: &Document, current: &Document, id: MaskId, enabled: bool) {
    assert_eq!(plane_words(before), plane_words(current));
    assert_eq!(current.icc_profile, before.icc_profile);
    assert_eq!(current.source_color_context, before.source_color_context);
    assert_eq!(current.working_profile_interpretation, before.working_profile_interpretation);
    let mut expected = before.xcf_state.as_ref().unwrap().clone();
    expected.items.iter_mut().find(|item| item.item == ItemRef::Mask(id)).unwrap().mask.as_mut().unwrap().enabled = enabled;
    assert_eq!(current.xcf_state.as_ref().unwrap(), &expected);
    for (a, b) in before.layers.iter().zip(&current.layers) {
        assert_eq!(
            (a.id, &a.name, a.visible, a.opacity.to_bits(), a.fill_opacity.to_bits()),
            (b.id, &b.name, b.visible, b.opacity.to_bits(), b.fill_opacity.to_bits())
        );
        if let Some(mask) = &b.mask {
            assert_eq!(mask.enabled, enabled);
        }
    }
}
fn reply(value: &serde_json::Value, mask: MaskId, layer: LayerId, enabled: bool, changed: bool) {
    assert_eq!(value, &json!({"maskId":mask.0.to_string(),"layerId":layer.0.to_string(),"enabled":enabled,"changed":changed}));
    assert_eq!(value.as_object().unwrap().len(), 4);
}

#[test]
#[ignore = "UNRUN:64 genuine geometry sources through actual capture/permitted/apply control, undo/noopredo, stale target and native disabled continuation"]
fn guarded_assigned_mask_control_preserves_focus_source_and_native_payloads() {
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
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, name, &imported, &ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        profiles(&before, &row);
        let owner = before.layers.iter().find(|layer| layer.mask.is_some()).unwrap().id;
        let state = app.session.active().unwrap();
        let focus = (state.active_layer, state.layer_anchor, state.selected_layers.clone(), state.channel_view.clone());
        let target = capture(&app, owner).unwrap();
        assert!(target.enabled);
        assert!(permitted(&app, &target));
        let mask = target.mask;
        let (original_image, original_fee) = display(&before).unwrap();
        let journal = app.session.journal.len();
        reply(&apply(&mut app, &target, false).unwrap(), mask, owner, false, true);
        assert_eq!(app.session.journal.len(), journal, "upstream mask command is not journaled");
        let disabled = app.session.active().unwrap().doc.clone();
        retained(&before, &disabled, mask, false);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Set XCF Layer Mask Enabled"));
        let state = app.session.active().unwrap();
        assert_eq!((state.active_layer, state.layer_anchor, state.selected_layers.clone(), state.channel_view.clone()), focus);
        let history = state.history.clone();
        let revision = state.revision;
        assert_eq!(apply(&mut app, &target, false).unwrap_err(), stale());
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        // This is only the installed CURRENT bitmap retained across persistence.
        let (disabled_image, disabled_fee) = display(&disabled).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let (undo_image, undo_fee) = display(&app.session.active().unwrap().doc).unwrap();
        assert_eq!(undo_image.pixels, original_image.pixels);
        drop((undo_image, undo_fee));
        let history = app.session.active().unwrap().history.clone();
        let revision = app.session.active().unwrap().revision;
        let current = capture(&app, owner).unwrap();
        reply(&apply(&mut app, &current, true).unwrap(), mask, owner, true, false);
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(app.session.active().unwrap().revision, revision);
        app.run("edit.redo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&disabled));
        let saved = photocraft_format::owned_document::save_current_document_archive(&disabled, Cursor::new(Vec::new()), Default::default(), &quota, &ctl())
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
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "mask-current.pcraft", archive, &ctl()).unwrap();
        let state = reopened.session.active().unwrap();
        assert!(state.path.is_none());
        assert!(!state.source_read_only);
        retained(&before, &state.doc, mask, false);
        let (actual, fee) = display(&state.doc).unwrap();
        assert_eq!(actual.pixels, disabled_image.pixels);
        drop((actual, fee));
        let current = capture(&reopened, owner).unwrap();
        assert!(!current.enabled);
        assert!(permitted(&reopened, &current));
        reply(&apply(&mut reopened, &current, true).unwrap(), mask, owner, true, true);
        retained(&before, &reopened.session.active().unwrap().doc, mask, true);
        let (actual, fee) = display(&reopened.session.active().unwrap().doc).unwrap();
        assert_eq!(actual.pixels, original_image.pixels);
        drop((actual, fee));
        assert_eq!(std::fs::read(root.join(name)).unwrap(), bytes);
        drop((original_image, original_fee));
        drop((disabled_image, disabled_fee));
    }
}
