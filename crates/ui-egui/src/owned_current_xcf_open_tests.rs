//! UNRUN source-selected reader capability and registered Open integration.
use super::*;
use crate::{Services, services::OwnedCurrentXcfOpenFn};
use photocraft_raster::backing::quota::{BackingQuota, QuotaLimits};
use std::{cell::RefCell, io::Cursor, rc::Rc, sync::Arc};

#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;

fn reader(bytes: Arc<Vec<u8>>, seen: Rc<RefCell<Vec<(String, bool)>>>) -> OwnedCurrentXcfOpenFn {
    Box::new(move |path, catalog, control| {
        seen.borrow_mut().push((path.into(), catalog.is_some()));
        let quota = catalog.cloned().unwrap_or_else(|| BackingQuota::new(QuotaLimits { resident: 512 << 20, index: 64 << 20, scratch: 0 }));
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        photocraft_io::import_owned_xcf_from_reader_in_quota(
            path.rsplit(['/', '\\']).next().unwrap(),
            &mut Cursor::new(bytes.as_slice()),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            quota,
            control,
        )
        .map_err(|e| e.to_string())
    })
}

#[test]
fn selected_plain_reader_opens_once_and_retains_catalog_after_source_tab_closes() {
    let bytes = Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        Services {
            owned_current_xcf_open: Some(reader(bytes, seen.clone())),
            owned_xcf_open: Some(Box::new(|_, _, _| panic!("legacy reader reached"))),
            ..Default::default()
        },
    );
    let first = invoke(&mut app, &json!({"path":"selected/first.xcf","xcfProfilePolicy":"keepCurrent"})).unwrap();
    assert_eq!(first["sourceReadOnly"], true);
    let quota = app.session.active().unwrap().doc.quota().unwrap().clone();
    app.session
        .add_document(
            photocraft_doc::Document::new("resident", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            None,
        )
        .unwrap();
    app.session.close(0).unwrap();
    assert!(app.session.accounted_open_quota().unwrap().shares_ledger(&quota));
    invoke(&mut app, &json!({"path":"selected/second.xcf","xcfProfilePolicy":"keepCurrent"})).unwrap();
    assert!(app.session.active().unwrap().doc.quota().unwrap().shares_ledger(&quota));
    assert_eq!(seen.borrow().as_slice(), &[("selected/first.xcf".into(), false), ("selected/second.xcf".into(), true)]);
}

#[test]
fn selected_refusal_is_final_for_each_authority_and_never_retries_a_legacy_reader() {
    for rooted in [false, true] {
        let seen = Rc::new(RefCell::new(0));
        let calls = seen.clone();
        let deny: OwnedCurrentXcfOpenFn = Box::new(move |_, _, _| {
            *calls.borrow_mut() += 1;
            Err("selected source refused".into())
        });
        let mut services = Services {
            owned_xcf_open: Some(Box::new(|_, _, _| panic!("legacy ambient retry"))),
            automation_owned_xcf_open: Some(Box::new(|_, _, _| panic!("legacy rooted retry"))),
            ..Default::default()
        };
        if rooted {
            services.automation_owned_current_xcf_open = Some(deny);
            services.owned_current_xcf_open = Some(Box::new(|_, _, _| panic!("ambient selected capability reached")));
        } else {
            services.owned_current_xcf_open = Some(deny);
            services.automation_owned_current_xcf_open = Some(Box::new(|_, _, _| panic!("rooted selected capability reached")));
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let params = json!({"path":"authorized/current.xcf","xcfProfilePolicy":"keepCurrent"});
        let result = if rooted { invoke_authorized(&mut app, &params) } else { invoke(&mut app, &params) };
        assert_eq!(result.unwrap_err(), "selected source refused");
        assert_eq!(*seen.borrow(), 1);
        assert!(app.session.documents().is_empty());
        assert!(app.ui.recent_files.is_empty());
    }
}

#[test]
#[ignore = "UNRUN: requires pinned independently authored Threshold Alpha source evidence"]
fn registered_selected_threshold_open_uses_the_same_reader_for_interactive_and_rooted_input() {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect::<String>();
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    let name = "rgb-double_non_linear.half.xcf";
    let bytes = Arc::new(std::fs::read(root.join(name)).unwrap());
    assert_eq!(hash(&bytes), manifest[name].as_str().unwrap());
    for rooted in [false, true] {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut services = Services::default();
        if rooted {
            services.automation_owned_current_xcf_open = Some(reader(bytes.clone(), seen.clone()));
            services.owned_current_xcf_open = Some(Box::new(|_, _, _| panic!("ambient capability reached")));
        } else {
            services.owned_current_xcf_open = Some(reader(bytes.clone(), seen.clone()));
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let params = json!({"path":"authorized/current.xcf","xcfProfilePolicy":"keepCurrent"});
        let reply =
            if rooted { invoke_authorized(&mut app, &params) } else { crate::menus::invoke(&mut app, &egui::Context::default(), "file.open", params) }.unwrap();
        assert_eq!(reply["sourceReadOnly"], true);
        assert!(reply["assessment"].is_object());
        let state = app.session.active().unwrap();
        assert!(state.doc.accounted().unwrap().is_readonly_threshold_alpha_storage());
        assert!(state.source_read_only);
        assert!(state.path.is_none());
        assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/current.xcf"));
        assert_eq!(seen.borrow().len(), 1);
        assert_eq!(app.ui.recent_files.is_empty(), rooted);
    }
}
