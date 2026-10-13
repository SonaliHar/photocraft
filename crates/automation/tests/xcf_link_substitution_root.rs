//! Explicit held-root substitution: immediate snapshot capture, one registered transaction.
//! Default fixtures are owned synthetic PNG/documents; the GIMP witness is opt-in below.
use photocraft_automation::{AuthorizedWorkspace, Headless};
use photocraft_doc::{
    Color, ColorMode, Document, LayerId, SampleType, Size,
    item_identity::{ItemRef, ResourceId},
    xcf_items::EditableSource,
    xcf_resources::ResourceKind,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
const METHOD: &str = "xcf.link.substituteFromRoot";
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Case {
    h: Headless,
    owner: LayerId,
    resource: ResourceId,
    dir: PathBuf,
    png: Vec<u8>,
}
fn case_for(doc: Document, png: Vec<u8>, owner: LayerId, resource: ResourceId, authority: u8) -> Case {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol6-root-substitution-test-runtime-20261013").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("replacement.png"), &png).unwrap();
    std::fs::write(dir.join("invalid.png"), b"own malformed PNG").unwrap();
    let f = std::fs::File::create(dir.join("overcap.png")).unwrap();
    f.set_len((8 << 20) + 1).unwrap();
    let mut h = match authority {
        1 => Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir), None).unwrap()),
        2 => Headless::trusted_local(),
        _ => Headless::new(),
    };
    h.session.open_document(doc, None).unwrap();
    h.session
        .edit("Real redo seed", |d, _| {
            d.name = "Own redo successor".into();
            Ok(())
        })
        .unwrap();
    assert!(h.session.undo());
    Case { h, owner, resource, dir, png }
}
fn own_case(mode: ColorMode, depth: SampleType, authority: u8) -> Case {
    let (d, p, o, r) = synthetic_link(mode, depth);
    case_for(d, p, o, r, authority)
}
fn request(c: &Case) -> Value {
    let st = c.h.session.active().unwrap();
    json!({"documentId":st.doc.id.0.to_string(),"expectedRevision":st.revision.to_string(),"layerId":c.owner.0.to_string(),"resourceId":c.resource.0.to_string(),"path":"replacement.png","policy":"substitute"})
}
fn direct_request(c: &Case) -> Value {
    use photocraft_engine::xcf_link_substitution_cmds::{capture_target, request_for_target};
    let target = capture_target(&c.h.session, c.owner, c.resource).unwrap();
    request_for_target(&c.h.session, &target, &c.png).unwrap()
}
#[derive(Debug, PartialEq)]
struct Evidence {
    native: Vec<u8>,
    revision: u64,
    saved: u64,
    past: Vec<String>,
    future: Vec<String>,
    journal: String,
}
fn evidence(h: &Headless) -> Evidence {
    let s = h.session.active().unwrap();
    Evidence {
        native: photocraft_format::save_to_bytes(&s.doc, &Default::default()).unwrap(),
        revision: s.revision,
        saved: s.saved_revision,
        past: s.history.entry_labels().map(str::to_string).collect(),
        future: s.history.redo_labels().map(str::to_string).collect(),
        journal: format!("{:?}", h.session.journal),
    }
}
fn same(h: &Headless, b: &Evidence) {
    assert!(evidence(h) == *b, "native/revision/history/redo/journal changed on refusal");
}
fn real_redo(h: &mut Headless, b: &Evidence) {
    same(h, b);
    let old = h.session.active().unwrap().doc.clone();
    assert!(h.session.redo());
    assert_eq!(h.session.active().unwrap().doc.name, "Own redo successor");
    assert!(h.session.undo());
    assert!(h.session.active().unwrap().doc.shares_state(&old));
}
fn success(mut c: Case) {
    let before = evidence(&c.h);
    let original = c.h.session.active().unwrap().doc.clone();
    let mut reference = photocraft_engine::Session::new();
    reference.open_document((*original).clone(), None).unwrap();
    let target = photocraft_engine::xcf_link_substitution_cmds::capture_target(&reference, c.owner, c.resource).unwrap();
    let p = photocraft_engine::xcf_link_substitution_cmds::request_for_target(&reference, &target, &c.png).unwrap();
    reference.execute("xcf.link.substitute", p).unwrap();
    let p = request(&c);
    let result = c.h.handle(METHOD, p).unwrap();
    assert_eq!(result["assessment"], "Reduced");
    assert_eq!(result["byteIdentity"], "Substituted");
    assert_eq!(result["applied"], true);
    let current = &c.h.session.active().unwrap().doc;
    assert_eq!(**current, *reference.active().unwrap().doc);
    assert_ne!(current.layer(c.owner).unwrap().surface(), original.layer(c.owner).unwrap().surface());
    assert_eq!(current.xcf_state.as_ref().unwrap().source_items[0].cache, original.xcf_state.as_ref().unwrap().source_items[0].cache);
    assert_eq!(c.h.session.active().unwrap().revision, before.revision + 1);
    assert_eq!(c.h.session.active().unwrap().history.entry_labels().count(), before.past.len() + 1);
    assert_eq!(c.h.session.journal.iter().filter(|x| x.0 == "xcf.link.substitute").count(), 0);
    let after = evidence(&c.h);
    assert_eq!(photocraft_format::load_from_bytes(&after.native).unwrap(), **current);
    assert!(c.h.session.undo());
    assert_eq!(evidence(&c.h).native, before.native);
    assert!(c.h.session.redo());
    assert_eq!(evidence(&c.h).native, after.native);
    assert_eq!(std::fs::read(c.dir.join("replacement.png")).unwrap(), c.png);
}
fn synthetic_link(mode: ColorMode, depth: SampleType) -> (Document, Vec<u8>, photocraft_doc::LayerId, ResourceId) {
    use photocraft_doc::{
        XcfState,
        xcf_items::{SourceCache, SourceItem},
        xcf_resources::ResourceReference,
    };
    let mut d = Document::with_background("Own synthetic linked samples", Size::new(2, 1), mode, depth, Color::WHITE);
    d.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
    d.icc_profile = Some(if mode == ColorMode::Rgb {
        photocraft_cms::builtin::srgb().to_bytes()
    } else {
        photocraft_cms::builtin::Builtin::GrayGamma22.profile().to_bytes()
    });
    let owner = d.layers[0].id;
    let bounds = d.bounds();
    let format = d.pixel_format();
    let raw: Vec<u8> = if depth == SampleType::U8 {
        (0..format.bytes_per_pixel() * 2).map(|i| (i * 23 + 7) as u8).collect()
    } else {
        (0..format.channels() * 2).flat_map(|i| ((i * 4243 + 29) as u16).to_ne_bytes()).collect()
    };
    let image = photocraft_codecs::Image::from_raw(
        2,
        1,
        if mode == ColorMode::Rgb { photocraft_codecs::ChannelLayout::Rgba } else { photocraft_codecs::ChannelLayout::GrayA },
        if depth == SampleType::U8 { photocraft_codecs::SampleType::U8 } else { photocraft_codecs::SampleType::U16 },
        raw,
    )
    .unwrap()
    .with_icc(d.icc_profile.as_deref().cloned());
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let original = d.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    let mut state = XcfState::default();
    state.reconcile_layers(&d).unwrap();
    state.items[0].raster_bounds = Some(bounds);
    state.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own substitute".into(),
        collection: None,
        internal: false,
        locator: Some("file:///inert-never-read".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    state.source_items.push(SourceItem {
        owner: ItemRef::Layer(owner),
        source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
        transform: None,
        cache: Some(SourceCache { blake3: blake3::hash(&original).to_hex().to_string(), bytes: original.into(), width: 2, height: 1, format }),
    });
    d.xcf_state = Some(state);
    d.validate_xcf_state().unwrap();
    (d, png, owner, ResourceId(1))
}

#[test]
fn authorized_root_rgb_gray_u8_u16_is_one_undoable_registered_edit() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            success(own_case(mode, depth, 1));
        }
    }
}
#[test]
fn legacy_root_recovery_stays_staging_only_and_undoable() {
    let mut c = own_case(ColorMode::Rgb, SampleType::U8, 1);
    let old = c.h.session.active().unwrap().doc.clone();
    let result = c.h.handle("xcf.resource.recover", json!({"resource":c.resource.0.to_string(),"path":"replacement.png","policy":"substitute"})).unwrap();
    assert_eq!(result["activated"], false);
    assert_eq!(result["byteIdentity"], "Substituted");
    assert_eq!(c.h.session.active().unwrap().doc.layer(c.owner).unwrap().surface(), old.layer(c.owner).unwrap().surface());
    assert!(c.h.session.undo());
    assert!(c.h.session.active().unwrap().doc.shares_state(&old));
}
#[test]
fn no_root_or_trusted_local_does_not_derive_authority() {
    for auth in [0, 2] {
        let mut c = own_case(ColorMode::Rgb, SampleType::U8, auth);
        let before = evidence(&c.h);
        let p = request(&c);
        let error = c.h.handle(METHOD, p).unwrap_err().to_string();
        assert!(!error.contains("unknown method"), "{error}");
        real_redo(&mut c.h, &before);
    }
}
#[test]
fn invalid_requests_and_root_paths_preserve_native_and_real_redo() {
    for path in [
        "../replacement.png",
        "C:/replacement.png",
        "https://example.invalid/a",
        "replacement\\png",
        "/replacement.png",
        "a//b",
        "a/./b",
        "CON",
        "replacement.png:stream",
    ] {
        let mut c = own_case(ColorMode::Rgb, SampleType::U8, 1);
        let mut p = request(&c);
        p["path"] = json!(path);
        let b = evidence(&c.h);
        let e = c.h.handle(METHOD, p).unwrap_err().to_string();
        assert!(!e.contains("unknown method"), "{e}");
        real_redo(&mut c.h, &b);
    }
    for lane in 0..11 {
        let mut c = own_case(ColorMode::Rgb, SampleType::U8, 1);
        let mut p = request(&c);
        match lane {
            0 => p["expectedRevision"] = json!("999999"),
            1 => p["documentId"] = json!("999999"),
            2 => p["unexpected"] = json!(true),
            3 => p["path"] = json!("overcap.png"),
            4 => p["path"] = json!("invalid.png"),
            5 => p["policy"] = json!("exact"),
            6 => {
                p.as_object_mut().unwrap().remove("path");
            }
            7 => p["resourceId"] = json!(1),
            8 => p["layerId"] = json!("01"),
            9 => p["expectedRevision"] = json!("03"),
            _ => p["documentId"] = json!("0"),
        };
        let b = evidence(&c.h);
        let e = c.h.handle(METHOD, p).unwrap_err().to_string();
        assert!(!e.contains("unknown method"), "{e}");
        real_redo(&mut c.h, &b);
    }
}
fn blocker(h: &mut Headless, complete: bool) -> (photocraft_engine::jobs::JobId, mpsc::Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let start = h
        .session
        .start_job(
            "own-root-blocker",
            json!({}),
            "Own root blocker",
            false,
            move |_| {
                if !complete {
                    rx.recv_timeout(Duration::from_secs(5)).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))?;
                }
                done_tx.send(()).unwrap();
                Ok(())
            },
            |s, ()| {
                s.edit("Must remain unapplied", |d, _| {
                    d.name = "JOB APPLIED".into();
                    Ok(())
                })?;
                Ok(json!({"done":true}))
            },
        )
        .unwrap();
    if complete {
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }
    let photocraft_engine::jobs::Started::Job(id) = start else { panic!("native job") };
    (id, tx)
}
#[test]
fn root_route_refuses_running_and_completed_unapplied_jobs_before_sync() {
    for complete in [false, true] {
        let mut c = own_case(ColorMode::Rgb, SampleType::U8, 1);
        let p = request(&c);
        let b = evidence(&c.h);
        let (id, tx) = blocker(&mut c.h, complete);
        let result = c.h.handle(METHOD, p);
        if !complete {
            tx.send(()).unwrap();
        }
        same(&c.h, &b);
        assert!(c.h.session.has_jobs());
        c.h.session.cancel_job(id);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("context"), "{error}");
        real_redo(&mut c.h, &b);
    }
}
#[test]
fn registered_command_start_and_rpc_never_sync_on_pending_refusal() {
    for rpc in [false, true] {
        for wait in [false, true] {
            let mut c = own_case(ColorMode::Rgb, SampleType::U8, 1);
            let p = direct_request(&c);
            let b = evidence(&c.h);
            let (id, _) = blocker(&mut c.h, true);
            let result = if rpc {
                c.h.handle("engine.execute", json!({"command":"xcf.link.substitute","params":p,"wait":wait}))
            } else {
                c.h.command_start("xcf.link.substitute", p, wait)
            };
            same(&c.h, &b);
            assert!(c.h.session.has_jobs());
            c.h.session.cancel_job(id);
            let error = result.unwrap_err().to_string();
            assert!(error.contains("context"), "{error}");
            real_redo(&mut c.h, &b);
        }
    }
}
#[test]
fn ordinary_rpc_keeps_existing_completed_job_synchronization() {
    let mut c = own_case(ColorMode::Rgb, SampleType::U8, 1);
    let (id, _) = blocker(&mut c.h, true);
    let result = c.h.handle("methods", json!({}));
    assert!(result.is_ok());
    assert_eq!(c.h.session.active().unwrap().doc.name, "JOB APPLIED");
    assert!(!c.h.session.has_jobs());
    assert!(c.h.session.job(id).is_none());
}
#[test]
#[ignore = "explicit real GIMP linked witness; requires the local qualified transform-interpolation corpus"]
fn actual_link_none_authorized_root_one_registered_undo_redo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/transform-interpolation");
    let doc = photocraft_io::import("link-none.xcf", &std::fs::read(root.join("link-none.xcf")).unwrap()).unwrap().document;
    let s = doc.xcf_state.as_ref().unwrap().source_items.iter().find(|s| matches!(s.source, EditableSource::Linked { .. })).unwrap();
    let ItemRef::Layer(owner) = s.owner else { panic!("layer") };
    let EditableSource::Linked { resource, .. } = s.source else { panic!("linked") };
    let mut image = photocraft_codecs::decode(&std::fs::read(root.join("own-linked-resource.png")).unwrap()).unwrap();
    for pixel in image.data_mut().as_chunks_mut::<4>().0 {
        for value in &mut pixel[..3] {
            *value = value.wrapping_add(41);
        }
    }
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    success(case_for(doc, png, owner, resource, 1));
}

fn own_exact_case(mode: ColorMode, depth: SampleType) -> Case {
    let (mut doc, png, owner, resource) = synthetic_link(mode, depth);
    doc.xcf_state.as_mut().unwrap().resources[0].content_hash = Some(blake3::hash(&png).to_hex().to_string());
    doc.validate_xcf_state().unwrap();
    case_for(doc, png, owner, resource, 1)
}

#[test]
fn exact_root_recovery_matches_retained_hash_and_has_one_undo() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            let mut c = own_exact_case(mode, depth);
            let before = evidence(&c.h);
            let original = c.h.session.active().unwrap().doc.clone();
            let target = photocraft_engine::xcf_link_substitution_cmds::capture_recovery_target(&c.h.session, c.owner, c.resource).unwrap();
            let params = photocraft_engine::xcf_link_substitution_cmds::request_for_recovery_target(&c.h.session, &target, &c.png).unwrap();
            let mut reference = photocraft_engine::Session::new();
            reference.open_document((*original).clone(), None).unwrap();
            // Rebuild the immediate request for the reference's own current revision.
            let target = photocraft_engine::xcf_link_substitution_cmds::capture_recovery_target(&reference, c.owner, c.resource).unwrap();
            let expected_params = photocraft_engine::xcf_link_substitution_cmds::request_for_recovery_target(&reference, &target, &c.png).unwrap();
            reference.execute("xcf.link.recover", expected_params).unwrap();
            assert_eq!(params["policy"], "exact");
            let mut p = request(&c);
            p["policy"] = json!("exact");
            let result = c.h.handle("xcf.link.recoverFromRoot", p).unwrap();
            assert_eq!(result["byteIdentity"], "Exact");
            assert_eq!(result["assessmentCommand"], "xcf.assess");
            assert!(result.get("assessment").is_none(), "byte identity must not claim whole-document Full");
            assert_eq!(*c.h.session.active().unwrap().doc, *reference.active().unwrap().doc);
            assert_eq!(c.h.session.active().unwrap().revision, before.revision + 1);
            assert_eq!(c.h.session.active().unwrap().history.entry_labels().count(), before.past.len() + 1);
            let after = evidence(&c.h);
            assert_eq!(photocraft_format::load_from_bytes(&after.native).unwrap(), *c.h.session.active().unwrap().doc);
            assert!(c.h.session.undo());
            assert_eq!(evidence(&c.h).native, before.native);
            assert!(c.h.session.redo());
            assert_eq!(evidence(&c.h).native, after.native);
        }
    }
}

#[test]
fn exact_root_missing_hash_mismatch_and_wrong_policy_preserve_real_redo() {
    for lane in 0..3 {
        let mut c = if lane == 0 { own_case(ColorMode::Rgb, SampleType::U8, 1) } else { own_exact_case(ColorMode::Rgb, SampleType::U8) };
        if lane == 1 {
            std::fs::write(c.dir.join("replacement.png"), b"own mismatching bytes").unwrap();
        }
        let mut p = request(&c);
        if lane != 2 {
            p["policy"] = json!("exact");
        }
        let before = evidence(&c.h);
        assert!(c.h.handle("xcf.link.recoverFromRoot", p).is_err());
        real_redo(&mut c.h, &before);
    }
}

#[test]
fn exact_registered_id_and_root_route_refuse_unapplied_jobs_before_sync() {
    for route in 0..3 {
        for wait in [false, true] {
            let mut c = own_exact_case(ColorMode::Rgb, SampleType::U8);
            let target = photocraft_engine::xcf_link_substitution_cmds::capture_recovery_target(&c.h.session, c.owner, c.resource).unwrap();
            let params = photocraft_engine::xcf_link_substitution_cmds::request_for_recovery_target(&c.h.session, &target, &c.png).unwrap();
            let mut root_params = request(&c);
            root_params["policy"] = json!("exact");
            let before = evidence(&c.h);
            let (id, _) = blocker(&mut c.h, true);
            let result = match route {
                0 => c.h.handle("xcf.link.recoverFromRoot", root_params),
                1 => c.h.command_start("xcf.link.recover", params, wait),
                _ => c.h.handle("engine.execute", json!({"command":"xcf.link.recover","params":params,"wait":wait})),
            };
            same(&c.h, &before);
            assert!(c.h.session.has_jobs());
            c.h.session.cancel_job(id);
            let error = result.unwrap_err().to_string();
            assert!(error.contains("context"), "{error}");
            real_redo(&mut c.h, &before);
        }
    }
}
