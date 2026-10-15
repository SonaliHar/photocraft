//! UNRUN real Semi-Flatten source/current transaction controls. No numerical acceptance claim.
use super::*;
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits},
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Arc};
const INPUTS: [(&str, &str); 4] = [
    (
        "temp/sol1-semi-flatten-gimp-20261016/rgb-f32-author-v1/outputs/rgb-f32-background.source.xcf",
        "e6d705f52c19ac30c4e3d5f92eefe22ee6baab791ce42fdb879397cb140c61da",
    ),
    (
        "temp/sol1-semi-flatten-gimp-20261016/rgb-f64-author-v1/outputs/rgb-f64-background.source.xcf",
        "63278093e370c2e7d0bcea850c1fa3ef494f6975b91746ea71594d7b9bec5b71",
    ),
    (
        "temp/sol1-semi-flatten-gimp-20261016/rgb-f32-nonlinear-bytes-author-v1/outputs/rgb-f32-nonlinear-bytes.source.xcf",
        "d4debe389177edc01192b09fadb22f5d353b54cd27f7af357698ab5cfb223b89",
    ),
    (
        "temp/sol1-semi-flatten-gimp-20261016/rgb-f64-nonlinear-bytes-author-v1/outputs/rgb-f64-nonlinear-bytes.source.xcf",
        "8caac66d87e1ea9599c90600709989a87dd33b458beec4611a764971dae9212a",
    ),
];

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session(case: usize) -> Session {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(INPUTS[case].0);
    let bytes = std::fs::read(p).unwrap();
    assert_eq!(Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(), INPUTS[case].1);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let photocraft_io::OwnedXcfImport::Point(source) = photocraft_io::import_owned_xcf_from_reader_in_quota(
        "Own Semi.xcf",
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q,
        &ctl(),
    )
    .unwrap() else {
        panic!("Point")
    };
    let mut s = Session::new();
    s.add_owned_point_xcf_import(&source, "Own Semi", None, &ctl()).unwrap();
    s
}
fn node(s: &Session) -> &FilterNode {
    &s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
}
fn words(s: &Session) -> ([u32; 4], SampleEncoding) {
    let FilterValue::Color { components, encoding, profile: None } = &node(s).arguments[0].value else { panic!("color") };
    let mut out = [0; 4];
    for (a, b) in out.iter_mut().zip(components) {
        let ExactNumber::Float32Bits(v) = b else { panic!("word") };
        *a = *v
    }
    (out, *encoding)
}
fn patch(s: &Session, bits: [u32; 4], encoding: SampleEncoding) -> Value {
    json!({"filterId":node(s).id.0.to_string(),"arguments":[{"name":"color","value":{"Color":{"components":bits.into_iter().map(|v|json!({"Float32Bits":v})).collect::<Vec<_>>(),"encoding":encoding,"profile":null}}}]})
}
fn reopen(s: &Session) -> Session {
    let owner = s.active().unwrap().doc.accounted().unwrap();
    let q = owner.quota().clone();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(owner, Cursor::new(Vec::new()), Default::default(), &q, &ctl()).unwrap().into_inner();
    let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        Default::default(),
        q,
        &ctl(),
    )
    .unwrap();
    let mut out = Session::new();
    out.add_owned_native_point_archive(archive, "Semi.pcraft", None, &ctl()).unwrap();
    out
}
#[test]
#[ignore = "UNRUN: actual four source roles use tag-aware color+enable transactions, paid query, title/native and undo/noopredo without source mutation"]
fn tagged_literal_transactions_and_native() {
    for case in 0..4 {
        let mut s = session(case);
        let original = s.active().unwrap().doc.clone();
        let (bits, encoding) = words(&s);
        let id = node(&s).id.0.to_string();
        let before = s.active().unwrap().revision;
        let equal = patch(&s, bits, encoding);
        assert_eq!(journal_depth("xcf.filter.set", &equal), 7);
        assert_eq!(s.execute("xcf.filter.set", equal).unwrap()["updated"], false);
        assert!(s.active().unwrap().doc.shares_state(&original));
        assert_eq!(s.active().unwrap().revision, before);
        let other = if encoding == SampleEncoding::Linear { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        let mut changed = bits;
        changed[0] = 0.375f32.to_bits();
        changed[3] = (-0.0f32).to_bits();
        let mut p = patch(&s, changed, other);
        p.as_object_mut().unwrap().insert("enabled".into(), json!(false));
        let rev = node(&s).id;
        let stackrev = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].revision;
        assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
        assert_eq!(node(&s).id, rev);
        assert_eq!(words(&s), (changed, other));
        assert!(!node(&s).enabled);
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].revision, stackrev + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        let filter = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":id}), &ctl()).unwrap();
        assert_eq!(filter.value()["arguments"][0]["state"]["value"]["Color"]["encoding"], json!(other));
        drop(filter);
        let project = crate::accounted_xcf_inspect::prepare(&s, &Value::Null, &ctl()).unwrap();
        assert_eq!(project.value()["filters"]["total"], 1);
        drop(project);
        let current = s.active().unwrap().doc.clone();
        let mut native = reopen(&s);
        assert_eq!(words(&native), (changed, other));
        assert_eq!(native.active().unwrap().doc.xcf_state, current.xcf_state);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let h = s.active().unwrap().history.clone();
        let equal = patch(&s, bits, encoding);
        assert_eq!(s.execute("xcf.filter.set", equal).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&h));
        assert!(s.redo_with_control(&ctl()).unwrap());
        assert!(s.active().unwrap().doc.shares_state(&current));
        assert_eq!(native.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).unwrap()["updated"], true);
        assert_eq!(native.active().unwrap().doc.icc_profile, original.icc_profile);
        assert_eq!(native.active().unwrap().doc.source_color_context, original.source_color_context);
    }
}
#[test]
#[ignore = "UNRUN: exact current singleton reorder/removal/empty native lifecycle, individual properties admitted, direct compound API+Move remain closed"]
fn singleton_stack_and_individual_properties() {
    for case in 0..4 {
        let mut s = session(case);
        let id = node(&s).id.0.to_string();
        let original = s.active().unwrap().doc.clone();
        assert_eq!(s.execute("xcf.filter.move", json!({"filterId":id,"index":0})).unwrap()["updated"], false);
        assert!(s.active().unwrap().doc.shares_state(&original));
        s.execute("layer.setProps", json!({"name":"Own current Semi","visible":false,"opacity":0.375})).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let owner = s.active().unwrap().doc.accounted().unwrap().clone();
        assert!(
            s.edit_accounted(
                "closed",
                photocraft_doc::accounted_document::AccountedEdit::LayerMetadata { layer, name: None, visible: None, opacity: None },
                &ctl()
            )
            .is_err()
        );
        assert!(s.execute("layer.translate", json!({"dx":1,"dy":0})).is_err());
        assert!(s.active().unwrap().doc.accounted().unwrap().shares_model(&owner));
        assert_eq!(s.execute("xcf.filter.remove", json!({"filterId":id})).unwrap()["updated"], true);
        let x = s.active().unwrap().doc.xcf_state.as_ref().unwrap();
        assert!(x.filter_stacks[0].nodes.is_empty());
        assert!(x.filter_masks.is_empty());
        assert_eq!(x.items.len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Remove GIMP Stock Filter"));
        let r = reopen(&s);
        assert!(r.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        assert!(s.undo_with_control(&ctl()).unwrap());
        assert_eq!(node(&s).id.0.to_string(), id);
    }
}
struct CurrentRetention {
    _owner: AccountedDocument,
    _index: QuotaLease,
    _resident: QuotaLease,
}
fn current_archive(case: usize, change: impl FnOnce(&mut photocraft_doc::Document)) -> photocraft_format::owned_document::OwnedDocumentArchive {
    let original = session(case);
    let owner = original.active().unwrap().doc.accounted().unwrap().clone();
    let q = owner.quota().clone();
    let index = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let mut doc = owner.document().clone();
    change(&mut doc);
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(actual.retained_bytes() <= 2 << 20);
    let current =
        AccountedDocument::new_readonly_point_storage(doc, Arc::new(CurrentRetention { _owner: owner, _index: index, _resident: resident }), q, &ctl())
            .unwrap();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&current, Cursor::new(Vec::new()), Default::default(), current.quota(), &ctl())
            .unwrap()
            .into_inner();
    let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut Cursor::new(bytes.as_slice()),
        bytes.len() as u64,
        Default::default(),
        current.quota().clone(),
        &ctl(),
    )
    .unwrap();
    archive
}
fn controlled_current(case: usize, change: impl FnOnce(&mut photocraft_doc::Document)) -> Session {
    let archive = current_archive(case, change);
    let mut out = Session::new();
    out.add_owned_native_point_archive(archive, "Synthetic paid CURRENT", None, &ctl()).unwrap();
    out
}
#[test]
#[ignore = "UNRUN: content/all/pixels before no-op and changed-only visibility, tag-changing one revision, semantic roles andMAX are independent"]
fn locks_and_typed_encoding_policy() {
    for lock in 0..4 {
        let mut s = controlled_current(0, |d| match lock {
            0 => d.layers[0].locks.all = true,
            1 => d.layers[0].locks.pixels = true,
            2 => d.xcf_state.as_mut().unwrap().items[0].locks.content = true,
            _ => d.xcf_state.as_mut().unwrap().items[0].locks.visibility = true,
        });
        let before = s.active().unwrap().doc.clone();
        let (bits, enc) = words(&s);
        let p = patch(&s, bits, enc);
        if lock < 3 {
            assert!(s.execute("xcf.filter.set", p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        } else {
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            let id = node(&s).id.0.to_string();
            assert!(s.execute("xcf.filter.enable", json!({"filterId":id,"enabled":false})).is_err());
            let other = patch(&s, bits, SampleEncoding::NonlinearProfile);
            assert_eq!(s.execute("xcf.filter.set", other).unwrap()["updated"], true);
        }
    }
    let mut s = session(0);
    let (bits, enc) = words(&s);
    let p = patch(&s, bits, enc);
    s.active_mut().unwrap().revision = u64::MAX;
    assert!(s.execute("xcf.filter.set", p).is_err());
}
#[test]
#[ignore = "UNRUN: prepared samepaid source/history/view/channel/journal/lock/control fences and malformed colour rollback"]
fn stale_and_failure_fences() {
    for bad in 0..3 {
        let archive = current_archive(0, |d| {
            let node = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            match bad {
                0 => node.operation.name = "gegl:unknown".into(),
                1 => node.operation.schema_epoch = "bad".into(),
                _ => node.operation.stored_version = "999".into(),
            }
        });
        assert!(crate::accounted_native_open::is_current_point_candidate(archive.document(), &ctl()).unwrap());
        let mut target = Session::new();
        assert!(target.add_owned_native_point_archive(archive, "Malformed CURRENT", None, &ctl()).is_err());
        assert!(target.active().is_none());
    }

    let mut s = session(0);
    let (mut bits, enc) = words(&s);
    bits[0] = 0.5f32.to_bits();
    let p = patch(&s, bits, enc);
    for stale in 0..4 {
        let before = s.active().unwrap().doc.clone();
        let prepared = Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).unwrap();
        match stale {
            0 => s.active_mut().unwrap().view_revision += 1,
            1 => s.active_mut().unwrap().history.max_states += 1,
            2 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            _ => s.active_mut().unwrap().source.as_mut().unwrap().label.push('x'),
        };
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s = session(0);
    }
    let stop = || true;
    assert!(Request::parse(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) }).is_err());
    assert!(Request::parse(&s, "xcf.filter.set", &p, &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }).is_err());
    let before = s.active().unwrap().doc.clone();
    let q = before.accounted().unwrap().quota().clone();
    let usage = q.usage().unwrap();
    let hold = q.reserve(QuotaKind::Index, (32 << 20) - usage.index).unwrap();
    assert!(Request::parse(&s, "xcf.filter.set", &p, &ctl()).unwrap().prepare(&s, &ctl()).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    let mut bad = p;
    bad["arguments"][0]["value"]["Color"]["encoding"] = json!("perceptual");
    assert!(s.execute("xcf.filter.set", bad).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
}
