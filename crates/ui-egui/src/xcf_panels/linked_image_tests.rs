use super::*;
use crate::{FileDialogAnswer, FileDialogReply, FileDialogRequest, Services};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size, XcfState, item_identity::ResourceId, xcf_items::SourceItem, xcf_resources::ResourceReference,
};
use std::{cell::RefCell, rc::Rc};

type Replies = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;
const BUTTON: &str = "Replace Linked Image (Reduced)…";

fn document() -> (Document, LayerId) {
    let mut doc = Document::with_background("Own pending picker", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let owner = doc.layers[0].id;
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own inert identity".into(),
        collection: None,
        internal: false,
        locator: Some("file:///never-authorized.png".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(owner),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: None,
    });
    doc.xcf_state = Some(state);
    (doc, owner)
}
fn app(doc: Document) -> (PhotocraftApp, Replies) {
    let replies = Replies::default();
    let held = replies.clone();
    let services = Services { file_dialog: Some(Box::new(move |request, _, reply| held.borrow_mut().push((request, reply)))), ..Default::default() };
    let mut session = photocraft_engine::Session::new();
    session.open_document(doc, None).unwrap();
    let app = PhotocraftApp::new(session, services);
    assert!(!app.background_jobs);
    (app, replies)
}
fn answer(replies: &Replies, value: Option<FileDialogAnswer>) {
    assert_eq!(replies.borrow().len(), 1);
    let (request, reply) = replies.borrow_mut().remove(0);
    assert_eq!(request, FileDialogRequest::XcfLinkedImage);
    reply.send(value);
}

#[test]
fn actual_widget_queues_only_clicked_bounded_request_and_cancel_is_atomic() {
    let (doc, owner) = document();
    let (app, replies) = app(doc.clone());
    let revision = app.session.active().unwrap().revision;
    let history = app.session.active().unwrap().history.past_len();
    let mut h = Harness::builder().with_size(egui::vec2(440., 260.)).build_ui_state(
        move |ui, app| {
            properties(app, ui, owner);
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    h.run();
    assert!(replies.borrow().is_empty());
    assert_eq!(*h.state().session.active().unwrap().doc, doc);
    h.get_by_label(BUTTON).click();
    h.run();
    assert!(h.state().file_dialog_open());
    assert_eq!(*h.state().session.active().unwrap().doc, doc);
    answer(&replies, None);
    h.run();
    assert!(!h.state().file_dialog_open());
    assert!(!h.state().ui.status_error);
    assert_eq!(*h.state().session.active().unwrap().doc, doc);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

#[test]
fn actual_widget_failed_oversize_and_path_answers_do_not_dispatch_or_read() {
    for value in [
        FileDialogAnswer::Failed("xcf.resource.read-failed".into()),
        FileDialogAnswer::Contents("own.png".into(), vec![0; (8 << 20) + 1]),
        FileDialogAnswer::Paths(vec!["file:///never-authorized.png".into()]),
        FileDialogAnswer::SaveTo("never-read.png".into()),
    ] {
        let (doc, owner) = document();
        let (app, replies) = app(doc.clone());
        let revision = app.session.active().unwrap().revision;
        let history = app.session.active().unwrap().history.past_len();
        let mut h = Harness::builder().build_ui_state(
            move |ui, app| {
                properties(app, ui, owner);
                app.poll_file_dialog(ui.ctx(), None);
            },
            app,
        );
        h.get_by_label(BUTTON).click();
        h.run();
        answer(&replies, Some(value));
        h.run();
        assert!(h.state().ui.status_error);
        assert_eq!(*h.state().session.active().unwrap().doc, doc);
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn completed_picker_never_refocuses_switched_or_closed_document() {
    for close in [false, true] {
        let (doc, owner) = document();
        let (app, replies) = app(doc.clone());
        let mut h = Harness::builder().build_ui_state(
            move |ui, app| {
                properties(app, ui, owner);
                app.poll_file_dialog(ui.ctx(), None);
            },
            app,
        );
        h.get_by_label(BUTTON).click();
        h.run();
        h.state_mut().run("file.new", serde_json::json!({"width":2,"height":2})).unwrap();
        if close {
            h.state_mut().session.close(0).unwrap();
        }
        let active = h.state().session.active().unwrap().doc.clone();
        let index = h.state().session.active_index();
        answer(&replies, Some(FileDialogAnswer::Contents("own.png".into(), vec![1])));
        h.run();
        assert!(h.state().ui.status_error);
        assert!(h.state().ui.status.contains("stale-target"));
        assert_eq!(h.state().session.active_index(), index);
        assert_eq!(*h.state().session.active().unwrap().doc, *active);
        if !close {
            assert_eq!(*h.state().session.documents()[0].doc, doc);
        }
    }
}

#[test]
fn completed_picker_rejects_edit_then_undo_revision_without_losing_redo() {
    let (doc, owner) = document();
    let (app, replies) = app(doc);
    let mut h = Harness::builder().build_ui_state(
        move |ui, app| {
            properties(app, ui, owner);
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    h.get_by_label(BUTTON).click();
    h.run();
    h.state_mut().run("layer.renameLayer", serde_json::json!({"layer":owner.0,"name":"Own pending edit"})).unwrap();
    h.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    let before = h.state().session.active().unwrap().doc.clone();
    let revision = h.state().session.active().unwrap().revision;
    assert!(h.state().session.active().unwrap().history.can_redo());
    answer(&replies, Some(FileDialogAnswer::Contents("own.png".into(), vec![1])));
    h.run();
    assert!(h.state().ui.status.contains("stale-target"));
    assert_eq!(*h.state().session.active().unwrap().doc, *before);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert!(h.state().session.active().unwrap().history.can_redo());
    h.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.layers[0].name, "Own pending edit");
}

#[test]
fn properties_never_copy_or_show_unbounded_locator_and_identity_metadata() {
    let (mut doc, owner) = document();
    let entry = &mut doc.xcf_state.as_mut().unwrap().resources[0];
    entry.name = "Own unseen metadata".repeat(100_000);
    entry.locator = Some("never-authorized".repeat(100_000));
    let (app, replies) = app(doc.clone());
    let mut h = Harness::builder().build_ui_state(move |ui, app| properties(app, ui, owner), app);
    h.run();
    h.get_by_label(BUTTON);
    assert!(replies.borrow().is_empty());
    assert_eq!(*h.state().session.active().unwrap().doc, doc);
}

#[test]
#[ignore = "explicit qualified linked source/PNG local witness; optional Vulkan Properties capture"]
fn actual_qualified_picker_substitution_is_one_registered_edit_with_native_history() {
    for background_jobs in [false, true] {
        qualified_picker_substitution(background_jobs);
    }
}

fn qualified_picker_substitution(background_jobs: bool) {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation");
    let source = std::fs::read(root.join("link-none.xcf")).unwrap();
    let original_png = std::fs::read(root.join("own-linked-resource.png")).unwrap();
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(source.len(), 4588);
    assert_eq!(hash(&source), "b40f7d42962b18ce1a7fff73fdc48354b6a32efed3fc559966acc21f5544f674");
    assert_eq!(original_png.len(), 667);
    assert_eq!(hash(&original_png), "fec97c32d4d01dfe813191f1e37eadd6d409556d6ad98dd8e02fd4bbea5260a8");
    let original = photocraft_io::import("link-none.xcf", &source).unwrap().document;
    let linked = original.xcf_state.as_ref().unwrap().source_items.iter().find(|s| matches!(s.source, EditableSource::Linked { .. })).unwrap();
    let ItemRef::Layer(owner) = linked.owner else { panic!("linked layer") };
    let EditableSource::Linked { resource, .. } = linked.source else { panic!("linked source") };
    let mut image = photocraft_codecs::decode(&original_png).unwrap();
    assert_eq!(image.sample_type(), photocraft_codecs::SampleType::U8);
    assert_eq!(image.layout(), photocraft_codecs::ChannelLayout::Rgba);
    for pixel in image.data_mut().as_chunks_mut::<4>().0 {
        for value in &mut pixel[..3] {
            *value = value.wrapping_add(41);
        }
    }
    let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let mut reference = photocraft_engine::Session::new();
    reference.open_document(original.clone(), None).unwrap();
    photocraft_engine::xcf_resources::recover_authorized_bytes(
        &mut reference,
        resource,
        photocraft_engine::xcf_resources::ServiceResourceBytes::from_authorized_service(ResourceKind::LinkedImage, bytes.clone(), true).unwrap(),
        photocraft_engine::xcf_resources::RecoveryPolicy::AllowSubstitution,
        &photocraft_raster::Interrupt::NONE,
    )
    .unwrap();
    reference.execute("xcf.link.refresh", serde_json::json!({"layerId":owner.0.to_string()})).unwrap();
    let expected = reference.active().unwrap().doc.clone();
    let (app, replies) = app(original.clone());
    let mut builder = Harness::builder().with_size(egui::vec2(440., 300.));
    let output = std::env::var_os("PHOTOCRAFT_LINKED_PROPERTIES_VISUALS").map(std::path::PathBuf::from);
    if output.is_some() {
        let mut setup = egui_kittest::wgpu::default_wgpu_setup();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
            create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
        }
        builder = builder.wgpu_setup(setup);
    }
    let mut h = builder.build_ui_state(
        move |ui, app| {
            properties(app, ui, owner);
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    let before = h.state().session.active().unwrap().doc.clone();
    let history = h.state().session.active().unwrap().history.entry_labels().count();
    // The actual native shell enables background jobs. This command must still
    // use its registered synchronous dispatch rather than Session::start.
    h.state_mut().background_jobs = background_jobs;
    h.get_by_label(BUTTON).click();
    h.run();
    answer(&replies, Some(FileDialogAnswer::Contents("own.png".into(), bytes)));
    h.run();
    if h.state().ui.status_error {
        assert_eq!(*h.state().session.active().unwrap().doc, *before);
        assert_eq!(h.state().session.active().unwrap().history.entry_labels().count(), history);
        println!("NATIVE_BACKGROUND_REFUSAL={} exact_document_unchanged=true history_unchanged=true", h.state().ui.status);
    }
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let after = h.state().session.active().unwrap().doc.clone();
    assert_eq!(*after, *expected, "widget must match the separately qualified replacement/refresh route in complete document state");
    assert_ne!(after.layer(owner).unwrap().surface(), before.layer(owner).unwrap().surface());
    assert_eq!(h.state().session.active().unwrap().history.entry_labels().count(), history + 1);
    let state = after.xcf_state.as_ref().unwrap();
    let entry = state.resources.iter().find(|r| r.id == resource).unwrap();
    assert_eq!(entry.resolution.as_ref().unwrap().fidelity, photocraft_doc::xcf_resources::ResourceFidelity::Substituted);
    assert_eq!(state.provenance, before.xcf_state.as_ref().unwrap().provenance);
    assert_eq!(state.source_items[0].cache, before.xcf_state.as_ref().unwrap().source_items[0].cache);
    assert_eq!(after.icc_profile, before.icc_profile);
    assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&after, &Default::default()).unwrap()).unwrap(), *after);
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
        PhotocraftApp::setup_context(&h.ctx, theme);
        h.run_steps(2);
        if let Some(output) = &output {
            std::fs::create_dir_all(output).unwrap();
            h.render().unwrap().save(output.join(format!("{theme:?}-linked-properties-background-{background_jobs}.png"))).unwrap();
        }
    }
    h.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, *before);
    h.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, *after);
    assert_eq!(std::fs::read(root.join("link-none.xcf")).unwrap(), source);
    assert_eq!(std::fs::read(root.join("own-linked-resource.png")).unwrap(), original_png);
}

#[test]
fn linked_frame_fence_refuses_replacement_at_same_revision() {
    let (doc, owner) = document();
    let (mut app, replies) = app(doc);
    let captured = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().name = "Own replacement snapshot".into();
    let before = app.session.active().unwrap().doc.clone();
    assert_eq!(validate_frame(&app, &captured, revision, owner).unwrap_err(), "xcf.link.ui.stale-target");
    assert!(app.session.active().unwrap().doc.shares_state(&before));
    assert!(replies.borrow().is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
}

#[test]
fn linked_duplicate_source_or_resource_metadata_never_selects_first_match() {
    let (mut doc, owner) = document();
    assert_eq!(retained_resource(&doc, owner), Some((ResourceId(1), false)));
    let source = doc.xcf_state.as_ref().unwrap().source_items[0].clone();
    doc.xcf_state.as_mut().unwrap().source_items.push(source);
    assert!(retained_resource(&doc, owner).is_none());
    doc.xcf_state.as_mut().unwrap().source_items.pop();
    let resource = doc.xcf_state.as_ref().unwrap().resources[0].clone();
    doc.xcf_state.as_mut().unwrap().resources.push(resource);
    assert!(retained_resource(&doc, owner).is_none());
}

#[test]
fn linked_actual_ancestor_content_lock_disables_picker_without_mutation() {
    let (mut doc, owner) = document();
    let child = doc.layers.remove(0);
    let group = photocraft_doc::Layer::group("Own imported parent", vec![child]);
    let parent = group.id;
    doc.layers.push(group);
    let mut state = doc.xcf_state.take().unwrap();
    state.reconcile_layers(&doc).unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(parent)).unwrap().locks.content = true;
    doc.xcf_state = Some(state);
    let before = doc.clone();
    let (app, replies) = app(doc);
    assert!(live_context(&app, owner).is_err());
    let mut h = Harness::builder().with_size(egui::vec2(440., 260.)).build_ui_state(move |ui, app| properties(app, ui, owner), app);
    h.run();
    assert!(h.get_by_label(BUTTON).accesskit_node().is_disabled());
    assert!(replies.borrow().is_empty());
    assert_eq!(*h.state().session.active().unwrap().doc, before);
}

#[test]
fn refresh_uses_registered_command_and_reports_missing_owned_material_without_edits() {
    let (doc, owner) = document();
    let (mut app, replies) = app(doc);
    let captured = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let result = run_frame(&mut app, &captured, revision, owner, "xcf.link.refresh", serde_json::json!({"layerId":owner.0.to_string()}));
    assert!(result.is_err());
    show_error(&mut app, result);
    assert!(app.ui.status_error);
    assert!(!app.ui.status.is_empty());
    assert!(app.session.active().unwrap().doc.shares_state(&captured));
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
    assert!(replies.borrow().is_empty());
}

#[test]
fn linked_geometry_hints_preserve_current_override_and_exact_matrix_bits() {
    let (mut doc, owner) = document();
    let matrix =
        [1f64.to_bits(), 0f64.to_bits(), 0f64.to_bits(), (-0f64).to_bits(), 1f64.to_bits(), 0f64.to_bits(), 0f64.to_bits(), 0f64.to_bits(), 1f64.to_bits()];
    let transform = photocraft_doc::xcf_items::TransformState { matrix_f64_bits: matrix, interpolation: 2, pre_offset: [7, -3] };
    let source = &mut doc.xcf_state.as_mut().unwrap().source_items[0];
    source.transform = Some(transform.clone());
    let EditableSource::Linked { current, .. } = &mut source.source else { panic!("linked fixture") };
    *current = Some(photocraft_doc::xcf_items::LinkedCurrentState { bounds: photocraft_doc::Rect::new(-5, 9, -3, 11), transform: Some(transform.clone()) });
    let hints = geometry_hints(&doc, owner).unwrap();
    assert_eq!(hints.origin, [-5, 9]);
    assert_eq!(hints.transform, Some(transform));
    assert_eq!(hints.transform.unwrap().matrix_f64_bits[3], (-0f64).to_bits());
}

#[test]
fn linked_geometry_draft_is_view_only_serializable_and_invalid_metadata_is_not_normalized() {
    let draft = crate::state::XcfLinkedGeometryDraft { document: 3, layer: u64::MAX, revision: 0, origin: [-5, 9], interpolation: 3 };
    let value = serde_json::to_value(&draft).unwrap();
    let restored: crate::state::XcfLinkedGeometryDraft = serde_json::from_value(value).unwrap();
    assert_eq!(draft, restored);
    let (mut doc, owner) = document();
    doc.xcf_state.as_mut().unwrap().source_items[0].transform =
        Some(photocraft_doc::xcf_items::TransformState { matrix_f64_bits: [f64::NAN.to_bits(); 9], interpolation: 3, pre_offset: [0, 0] });
    assert!(geometry_hints(&doc, owner).is_none());
    for (id, name) in [(0, "none"), (1, "linear"), (2, "cubic"), (3, "nohalo")] {
        assert_eq!(interpolation_name(id), name);
    }
}
