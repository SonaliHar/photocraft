//! UNRUN readonly flat blend handoffs. Encoded/source/native captures are external harness data; no new numerical oracle.
use super::*;
use photocraft_doc::{Layer, LayerContent};
use photocraft_raster::{
    Interrupt,
    backing::{
        NativePlaneDescriptor,
        quota::{BackingQuota, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
const CASES: &[(&str, &str, usize)] = &[
    ("composition-matrix/mode-screen.xcf", "5d938125138a0006dac2aa1045893973602891d9795e3a52df91acc75722fdeb", 5542),
    ("composition-matrix/mode-screen.edit.xcf", "3bb182cfcbe5fbf7f42551c09f235aab09557cd41cf3560807eb84d97339ce61", 5542),
    ("composition-matrix/mode-difference.xcf", "7e458bfc4f4420a116cfa32d456c565f4b2bde49fe37f8be35c382e365871abf", 5542),
    ("composition-matrix/mode-difference.edit.xcf", "ac93e43a92ee9aa758cc82d095406c59606395219e00baf71a83283691452aec", 5542),
    ("composition-matrix/mode-addition.xcf", "ca3f396b585f4bb8d4058bdfd36877c372da48cf3bf9e1dac79a5ee610f27821", 5425),
    ("composition-matrix/mode-addition.edit.xcf", "e1584e72c80ebc975e06e795c4c27704b9af4b676a2d32f585b333f5c1b47bcb", 5425),
    ("composition-matrix/mode-subtract.xcf", "1d4c73e0065691533fc1face74d667fbc569ce6799d5457e7ffd6d96f3066d8a", 5425),
    ("composition-matrix/mode-subtract.edit.xcf", "619b2bf4c1d0d51e072a7c86152fa00531c1c912ca96a8b5ad72f514773e49b0", 5425),
    ("composition-matrix/mode-exclusion.xcf", "ae201ec48252f63674f646287cf54222fb1d5f2ddf6fa3cedf8a1828b9e1eb66", 5542),
    ("composition-matrix/mode-exclusion.edit.xcf", "5235ecc99bc1f7d84f50db8f8c7f9d9e5120cd2d50cfeff16ef0ea44d111d4cb", 5542),
    ("composition-matrix/mode-overlay.xcf", "490bdedea33440bfc5d7b803479219206b5f6fa7a09962a8f7a2a9d26da752dd", 5542),
    ("composition-matrix/mode-overlay.edit.xcf", "4fa5f4a73aad9dc01e2209ef527d1445ef5dbbc0bb59fa6d3f3f1028283b24fe", 5542),
    ("composition-matrix/mode-grain-merge.xcf", "c6d004d91d69fae7a76a7173d70f4f885e8a6d7cf2384b4de4582dec7f7f2c3f", 5542),
    ("composition-matrix/mode-grain-merge.edit.xcf", "3320ded33d8bb7a66aebf88c5c991001619a170fec0f112df35ba7efa8f1b975", 5542),
    ("composition-matrix/mode-grain-extract.xcf", "3a36bf4bb2617f861e18cfdafe436e2b94644ff9edbb12162f4212efaac8cc85", 5542),
    ("composition-matrix/mode-grain-extract.edit.xcf", "ee2bcab5ab63d13a287ebe38c88bb2266ec40e0f45c1f3488b6252ac37565dd9", 5542),
    ("composition-matrix/mode-linear-light.xcf", "13f7e044fca3ad0cdab9839c044eeee189a4cf01fea8197a8fac397793e02a6e", 5542),
    ("composition-matrix/mode-linear-light.edit.xcf", "6cc2027292a70894c15ce0786f163df920d4c8f9915a5255a89070c0b037f398", 5542),
    ("composition-matrix/mode-hardlight.xcf", "86b6f3a21ac3204164ca2bdeb628bd0b26932b86d71b38cfcfabf58add1b05b8", 5542),
    ("composition-matrix/mode-hardlight.edit.xcf", "a5ea6a911396211d3160de881590c0705178fc107ef5491e733cd67c7909cc70", 5542),
    ("composition-matrix/mode-softlight.xcf", "ba4e9de7d1f973e01d56503542874a863960fa77eb98dc9ee073fe7773838b8c", 5542),
    ("composition-matrix/mode-softlight.edit.xcf", "f62c4b5d1b8ef3480861ac624831311953298933be6968b5566abba81b725640", 5542),
    ("composition-matrix/mode-pin-light.xcf", "7565dedc49fd81e466d28c7b61fa95ff4ddc17d1092ced3dabae146dab2d5dc0", 5542),
    ("composition-matrix/mode-pin-light.edit.xcf", "8979ff9cf2502e58a6d0281c941e01dd4aeb7682ed7fa51ef60d95c8d0972b44", 5542),
    ("composition-matrix/mode-hard-mix.xcf", "11bef2c38c7dc33e734c6cf8a863c97fdd7c453a6ed391b476a626fe0805e935", 5542),
    ("composition-matrix/mode-hard-mix.edit.xcf", "73eea4cd5d1fd460d96061df23c9184046c2c9e22c0bda059605c213f51b02d9", 5542),
    ("composition-matrix/mode-dodge.xcf", "18d5a8ec1893f342faf11700fb6fcd140caacffda7d5d7b0bf8b9609f0d6d430", 5542),
    ("composition-matrix/mode-dodge.edit.xcf", "6fd3033dc96967cbf20310a8f24aa8fa2d4854cdaa6dc8b4d204d9a3d0cc5cef", 5542),
    ("composition-matrix/mode-burn.xcf", "0e16658db9a191b1ca0f9edf2c86630429dfa8c56d54c9cd6acb4cb44813b85a", 5542),
    ("composition-matrix/mode-burn.edit.xcf", "927b4f17803299326b4ac954d11bff1b6d40253c3988956bca182f67e277e201", 5542),
    ("composition-matrix/mode-vivid-light.xcf", "01e5cd44bc309d5e65d854b036ceb0636dc71f004778998b7db6c15740e0fbdd", 5542),
    ("composition-matrix/mode-vivid-light.edit.xcf", "08c7b856016db71a7308048d4af976e0cd4b61b1ac40bd1cc58478a7e0b0a8fa", 5542),
    ("composition-matrix/mode-divide.xcf", "3c505dd0ae565b8dd9d657d49b3b6badd5a3069f34d039401f42282410500bf3", 5425),
    ("composition-matrix/mode-divide.edit.xcf", "c12f320e82f0a7a7903cfcd6582c623e9200310508f2a742d7dfb20b9764f048", 5425),
    ("composition-matrix/mode-linear-burn.xcf", "90f4d6baf0e028e8925127970bc5df29c33ee2ec37d7fb992547c7e9f5e68809", 5542),
    ("composition-matrix/mode-linear-burn.edit.xcf", "121960e1a52647b844c9413f387df561925d0c4d964aca6d8cf739a50329e147", 5542),
    ("composition-matrix/mode-darken-only.xcf", "d422cdbec10b07aa128af0ea920d0ed09ce5b904011b0954fdc588de306dce1f", 5425),
    ("composition-matrix/mode-darken-only.edit.xcf", "6cddf742aaed2d36defe7b54025f83073a41fbdcb1ce0ab3f435e3637acfd18c", 5425),
    ("composition-matrix/mode-lighten-only.xcf", "a4a528ca04e0b429cdd65d92c246c841d9ecc89740069f1a23d580d1d25b4656", 5425),
    ("composition-matrix/mode-lighten-only.edit.xcf", "9b0e09e5b2629da4acbd5771ad97125e7150b7619c1e28c58b72f4ab05d73953", 5425),
    ("composition-matrix/mode-multiply.xcf", "8dd84fb49c3c723aaf2c402f8350f6f04aea5bc1f0ce1bb67ac48d3f29aacea2", 5425),
    ("composition-matrix/mode-multiply.edit.xcf", "4cf565f8a5fe928c972dd97968ba9d765f56e454562762f48d65e43b7d0e9a1a", 5425),
    ("composition-matrix/blend-space-rgb-non-linear.xcf", "d9ab42a23f6d8cf6f3b8ae0e5320b28cd974382ea288713458ee75f14a9c3679", 5425),
    ("composition-matrix/blend-space-rgb-non-linear.edit.xcf", "b079bc2f4bab90d55b48d9cc4ac3890b3aa04b652ddb04c7f84bf59a52bd9e8e", 5425),
    ("composition-matrix/blend-space-rgb-perceptual.xcf", "91fb20c48517671a59b9d2b82491dcb05d29c062d2facd8bc8a2fa06565cbf4c", 5542),
    ("composition-matrix/blend-space-rgb-perceptual.edit.xcf", "2760fe473a0ecdd76c938d099babf239c39e0956f24c9be2745169738846437d", 5542),
    ("composition-matrix/composite-space-rgb-non-linear.xcf", "d58495366628bce198a21cfdb9158c5cce0bef03cc28bcf03d9558e23c0ffa65", 5425),
    ("composition-matrix/composite-space-rgb-non-linear.edit.xcf", "b1e8fa62ecc45880e62f56054daa9762871d90007b2ebbf93a962de69c5a4fc5", 5425),
    ("composition-matrix/composite-space-rgb-perceptual.xcf", "35e362f9b0c77f9da8f5fd6084211d4da3b09f0498fe54d2173c2c6fb905860c", 5542),
    ("composition-matrix/composite-space-rgb-perceptual.edit.xcf", "fc9510da6f1963f1651478d49c4b8c179d9e35b591bb91a3cf7276054afe70a4", 5542),
];

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 })
}
fn source(i: usize, q: &BackingQuota) -> photocraft_io::OwnedSeparableBlendXcfImport {
    let (name, pin, len) = CASES[i];
    let bytes = std::fs::read(PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(name)).unwrap();
    assert_eq!(bytes.len(), len);
    assert_eq!(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(), pin);
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        name,
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::SeparableBlend(owner) = result else { panic!("one strict route") };
    owner
}
fn native(s: &Session) -> (Vec<NativePlaneDescriptor>, String) {
    fn visit(layers: &[Layer], depth: usize, descriptors: &mut Vec<NativePlaneDescriptor>, digest: &mut Sha256) {
        assert!(depth <= 1);
        for layer in layers {
            match &layer.content {
                LayerContent::BackedRaster(backed) => {
                    descriptors.push(backed.descriptor());
                    digest.update(backed.descriptor().default_bytes());
                    for coord in backed.operative_bounds().tiles() {
                        let pin = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
                        digest.update(coord.tx.to_ne_bytes());
                        digest.update(coord.ty.to_ne_bytes());
                        match pin.encoded() {
                            NativeTileBytes::FullTile(bytes) => digest.update(bytes),
                            NativeTileBytes::DefaultPixel(pixel) => {
                                for _ in 0..65536 {
                                    digest.update(pixel);
                                }
                            }
                        }
                    }
                }
                LayerContent::Group(group) => visit(&group.children, depth + 1, descriptors, digest),
                _ => panic!("closed group content"),
            }
            if let Some(mask) = &layer.mask {
                digest.update(mask.surface.default_bytes());
                for (coordinate, tile) in mask.surface.tiles() {
                    digest.update(coordinate.tx.to_ne_bytes());
                    digest.update(coordinate.ty.to_ne_bytes());
                    digest.update(tile.bytes());
                }
            }
        }
    }
    let mut descriptors = Vec::new();
    let mut digest = Sha256::new();
    visit(&s.active().unwrap().doc.layers, 0, &mut descriptors, &mut digest);
    (descriptors, digest.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
#[test]
#[ignore = "UNRUN: all50 genuine source/title/native/current project paths retain raw native, source/current profiles and canonical policies; no GIMP numeric claim"]
fn genuine_readonly_source_title_native_and_project() {
    for i in 0..CASES.len() {
        let q = quota();
        let imported = source(i, &q);
        let original = imported.document().clone();
        let mut s = Session::new();
        s.add_owned_separable_blend_xcf_import(&imported, "flat", Some(CASES[i].0), &ctl()).unwrap();
        drop(imported);
        let raw = native(&s);
        assert!(s.active().unwrap().doc.accounted().unwrap().is_readonly_flat_composite_storage());
        assert!(crate::accounted_native_open::is_current_separable_blend_candidate(&s.active().unwrap().doc, &ctl()).unwrap());
        let reply = crate::accounted_xcf_inspect::prepare(&s, &serde_json::json!({}), &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 2);
        assert_eq!(reply.value()["filters"]["total"], 0);
        drop(reply);
        assert!(crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":"1"}), &ctl()).is_err());
        assert!(s.execute("layer.setProps", serde_json::json!({"name":"refuse"})).is_err());
        assert!(s.execute("layer.translate", serde_json::json!({"dx":1})).is_err());
        let bytes = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "FlatTitle.pcraft", &ctl())
            .unwrap()
            .write_and_publish(&mut s, &ctl(), |doc, _| {
                photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|w| w.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            Default::default(),
            q.clone(),
            &ctl(),
        )
        .unwrap();
        let mut next = Session::new();
        next.add_owned_native_separable_blend_archive(archive, "native", None, &ctl()).unwrap();
        assert_eq!(native(&next), raw);
        assert_eq!(next.active().unwrap().doc.icc_profile, original.document().icc_profile);
        assert_eq!(next.active().unwrap().doc.xcf_state, original.document().xcf_state);
    }
}
#[test]
#[ignore = "UNRUN: structural tag cannot bypass typed source/native validation and finite work/quota refusal does not publish tabs"]
fn generic_adoption_and_failure_have_no_prefix() {
    let q = quota();
    let owner = source(0, &q);
    let mut s = Session::new();
    assert!(s.add_accounted_document(owner.document().clone(), None, &ctl()).is_err());
    assert!(s.docs.is_empty());
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(s.add_owned_separable_blend_xcf_import(&owner, "flat", None, &c).is_err());
    assert!(s.docs.is_empty());
    let hold = q.reserve(photocraft_raster::backing::quota::QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.add_owned_separable_blend_xcf_import(&owner, "flat", None, &ctl()).is_err());
    drop(hold);
    assert!(s.docs.is_empty());
}
