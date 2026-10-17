use photocraft_engine::Session;
use serde_json::{Value, json};
const COMMAND: &str = "xcf.vector.stroke.width.set";
#[test]
fn malformed_stroke_style_parameters_never_create_history() {
    for p in [
        Value::Null,
        json!({}),
        json!({"layerId":1,"pathId":"1","widthF32Bits":"41000000"}),
        json!({"layerId":"01","pathId":"1","widthF32Bits":"41000000"}),
        json!({"layerId":"18446744073709551616","pathId":"1","widthF32Bits":"41000000"}),
        json!({"layerId":"1","pathId":"1","widthF32Bits":"7fc00000"}),
        json!({"layerId":"1","pathId":"1","widthF32Bits":"40C00000"}),
        json!({"layerId":"1","pathId":"1","widthF32Bits":"41000000","locator":"file:///ambient"}),
    ] {
        let mut s = Session::new();
        let doc = photocraft_doc::Document::new(
            "Invalid style parameters",
            photocraft_doc::Size::new(2, 2),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
        );
        s.add_document(doc.clone(), None).unwrap();
        assert!(s.execute(COMMAND, p).is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
#[cfg(feature = "corpus")]
mod originals {
    use super::*;
    use photocraft_doc::{Document, item_identity::ItemRef};
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicBool, Ordering};
    fn packet_for(case: usize) -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("reviewed local full corpus required")).join(
            [
                "vector-stroke-evidence",
                "vector-aa-stroke-evidence",
                "vector-aa-even-stroke-evidence",
                "vector-bevel-stroke-evidence",
                "vector-bevel-stroke-evidence/reversed-negative",
                "vector-round-stroke-evidence",
                "vector-round-stroke-evidence/reversed-negative",
                "vector-odd-miter-stroke-evidence/noaa",
                "vector-odd-miter-stroke-evidence/noaa-negative",
                "vector-odd-miter-stroke-evidence/aa",
                "vector-open-axis-stroke-evidence/horizontal",
                "vector-open-axis-stroke-evidence/vertical",
                "vector-open-axis-stroke-evidence/reversed-negative",
                "vector-square-axis-stroke-evidence/horizontal",
                "vector-square-axis-stroke-evidence/vertical",
                "vector-square-axis-stroke-evidence/reversed-negative",
                "vector-round-axis-stroke-evidence/horizontal",
                "vector-round-axis-stroke-evidence/vertical",
                "vector-round-axis-stroke-evidence/reversed-negative",
                "vector-collinear-axis-stroke-evidence/horizontal",
                "vector-collinear-axis-stroke-evidence/vertical",
                "vector-collinear-caps-stroke-evidence/square-horizontal",
                "vector-collinear-caps-stroke-evidence/square-vertical",
                "vector-collinear-caps-stroke-evidence/round-horizontal",
                "vector-collinear-caps-stroke-evidence/round-vertical",
                "vector-odd-collinear-square-evidence/square-horizontal",
                "vector-odd-collinear-square-evidence/square-vertical",
                "vector-odd-round-body-evidence/horizontal",
                "vector-odd-round-body-evidence/vertical",
                "vector-odd-round-body-evidence/reversed-negative-horizontal",
                "vector-odd-round-body-evidence/reversed-negative-vertical",
                "vector-odd-butt-stroke-evidence/horizontal",
                "vector-odd-butt-stroke-evidence/vertical",
                "vector-odd-butt-negative-evidence/horizontal",
                "vector-odd-butt-negative-evidence/vertical",
            ][case],
        )
    }
    fn source_for(case: usize) -> Document {
        let bytes = std::fs::read(packet_for(case).join("source.xcf")).unwrap();
        assert_eq!(
            blake3::hash(&bytes).to_hex().as_str(),
            [
                "853b84c0869965437791a6767c0f7e80df5f9898955147a93b95d605ca51b851",
                "58f19000c0fd76b9cfab03253ba285e65eeccf0bf4eb599b20441a0c6794c520",
                "f538ebd6df6d9109183a3abe4ee2ac267c3a89dc79395de34a4f5dc0cb27eeca",
                "9b6bbc7641efa56d7da5ac05a58fd68b584b6424d27bc694d1567f1f102d7d29",
                "2eecb6365b515777c1c9fdb66ac158c3e6e86b8cd42a46ee60fb933f9d3187c7",
                "7cec68ef842fc04945fc9f874883af74d3e5ec141d6061d499d48c883c6e97eb",
                "4676879b743831bc38d29eb96042a50de4b2b6982e8e55d34ca64486a3a56bb6",
                "915d60197f028b89cab32717abca2daa74d37500d5bd2dd19cc66016837ca763",
                "7bb8fc5788812ca61aea6be2fd016cabf33e7a76a7b0c2c0d72b1c4871c2f9d5",
                "f411e9b077a818a51e0c0ad289a75501ad5a1daeff631a54229a79dbc24bebad",
                "798c972385bc603fc5723687bda76a289d093a04632f0a8476d737605445fb95",
                "d501c0fdbba2a5aa390dabe07e571eee111ea9dafc214fbb669f14302dd8000c",
                "4f852388d1b77abdaf96edf02123e95e10f63a934b1ff88c879375cdb0d66158",
                "7e702b7ad18fb57f1eebdc68e9e3016fb49d67fa01464b795dd7923704c9c1f2",
                "889cf6f9f29c17320b85e9de2c70806127ce4bbb382bc23f19c4e0418392e372",
                "aa8ce631e5c2f6e0bce28e08d4093b09d0f138a3d87bafcc99ced31f994b621a",
                "ed26fa7e73c81c5a13d3b6a71ca77f25f7c6a795af2e7cc9ee4a0d0bc162073b",
                "58f6c4ed5b80ac6e95438bc24d48a3097648f822537acf20ff41b5727cad6fc2",
                "70120624486c0190abd9ae96b56f3b165e5b9bd2d44e8f3414f45aea37811778",
                "276d9a14fd0ad7f331ad3583e6c8f45a3b8fc8998a9706a5a633006f31df6922",
                "e5581a89b221848031003c1ab77d18f882f7b6f042a0d82d7421ec8c27e8e24a",
                "bff3de1d9a74766491f8c695e45ffef4e027aa76e717385c608385d390e14ca3",
                "b1e022bb1153d05f5b9220fe583b5f7d3d7d7900ab75105b014e7855832bf655",
                "cabb94911c9e73e92798bb8b9773ceb6e895468766257377021e96bffc48ef40",
                "004132fd78e4f6dd57438f084a33903bb650f4c8f7cf53b0df9ae6fb4c2803b2",
                "5a8c84d3b540edef1b2d46aa539da5bc530842e2f2c0d99b8f587b719d42b145",
                "8fb5bd31ac574ac62697a8603afc07ef83c4e645d9e141b994413c9fa5c52ad7",
                "6db82c04a9a3ff694e4a50e5ce6ad2d420c48204adcf44b6ec4c411840868f0d",
                "f9e06dbad6c0d1c436d348b4a29a291e5e3e256df686c74852e2e716a5b09f09",
                "875aaf3168697f16981b6aa9752bd37cb9ec62b25091444a7c9e609121eb5d93",
                "d35708121258668e8537a7756cf5a6517be6489200d24a3de225345ab3344d18",
                "d3911670a96793110a3ab91fcb0f1721f281a680de2b042b0176e496bd2adb9a",
                "6daef14d22875a09bd41ffa77d6c11c68cebd3e98c9da8ca06b1f9783a747e8d",
                "d036f6d0bd7863d5c5fe1af52c049c6b3b96807f9605ccf5034458270011a834",
                "51bfe53b7839283c50bd74720f77078608560c537906e22010e42fe46464a06b",
            ][case]
        );
        photocraft_io::import("stroke.xcf", &bytes).unwrap().document
    }
    fn params(doc: &Document, width: &str) -> Value {
        let ItemRef::Layer(owner) = doc.xcf_state.as_ref().unwrap().source_items[0].owner else { panic!() };
        json!({"layerId":owner.0.to_string(),"pathId":doc.paths[0].id.0.to_string(),"widthF32Bits":width})
    }
    fn projection(doc: &Document) -> Vec<u8> {
        photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
            .unwrap()
            .px
            .iter()
            .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
            .collect()
    }
    #[test]
    #[ignore = "requires reviewed unpublished solid-stroke source/edit"]
    fn stroke_width_and_current_path_match_gimp_edit_preserve_evidence_undo_native() {
        verify_edit(0);
    }
    #[test]
    #[ignore = "requires reviewed unpublished AA solid-stroke source/edit"]
    fn aa_stroke_width_and_current_path_match_gimp_edit_preserve_evidence_undo_native() {
        verify_edit(1);
    }
    #[test]
    #[ignore = "requires reviewed unpublished even AA solid-stroke source/edit"]
    fn even_aa_stroke_width_and_current_path_match_gimp_edit_preserve_evidence_undo_native() {
        verify_edit(2);
    }
    #[test]
    #[ignore = "requires own bevel source/edit and negative-origin holdout"]
    fn bevel_stroke_width_path_native_and_undo_match_gimp_edits() {
        verify_edit(3);
        verify_edit(4);
    }
    #[test]
    #[ignore = "requires own round source/edit and negative-origin holdout"]
    fn round_stroke_width_path_native_and_undo_match_gimp_edits() {
        verify_edit(5);
        verify_edit(6);
    }
    #[test]
    #[ignore = "requires odd miter noAA/negative and AA independent source/edit packets"]
    fn odd_miter_width_current_path_native_history_matches_stock() {
        for case in 7..10 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires odd miter noAA/negative and AA independent sources"]
    fn odd_miter_locks_stale_ids_and_cancel_are_atomic() {
        for case in 7..10 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires open horizontal/vertical/reversednegative Butt native/source/edit packet"]
    fn open_axis_current_path_width_native_and_undo_match_stock() {
        for case in 10..13 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires open horizontal/vertical/reversednegative Butt source packet"]
    fn open_axis_locks_stale_ids_and_cancel_are_atomic() {
        for case in 10..13 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires own public Square cap source/edit pairs and negative origin"]
    fn square_axis_width_path_native_history_matches_stock() {
        for case in 13..16 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires own public Square cap original sources"]
    fn square_axis_locks_stale_ids_cancel_are_atomic() {
        for case in 13..16 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires own public Round cap source/edit and negative origin"]
    fn round_axis_width_path_native_history_matches_stock() {
        for case in 16..19 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires own public Round cap original sources"]
    fn round_axis_locks_stale_ids_cancel_are_atomic() {
        for case in 16..19 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires independent collinear Butt source/edit references"]
    fn collinear_width_path_native_history_match_stock() {
        for case in 19..21 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires independent collinear Butt source originals"]
    fn collinear_locks_stale_ids_cancel_atomic() {
        for case in 19..21 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires actual collinear Square/Round source/edit originals"]
    fn collinear_caps_path_width_history_native_match_stock() {
        for case in 21..25 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires actual collinear Square/Round originals"]
    fn collinear_caps_locks_stale_cancel_atomic() {
        for case in 21..25 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires independent odd Square cap source/edit pair"]
    fn odd_square_path_width_native_history_match_stock() {
        for case in 25..27 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires independent odd Square cap originals"]
    fn odd_square_locks_stale_cancel_atomic() {
        for case in 25..27 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires independent odd Round source/edit and negative holdouts"]
    fn odd_round_path_width_native_history_match_stock() {
        for case in 27..31 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires independent odd Round source originals"]
    fn odd_round_locks_stale_cancel_atomic() {
        for case in 27..31 {
            verify_atomic(case);
        }
    }
    #[test]
    #[ignore = "requires independent odd Butt source/edit and negative holdouts"]
    fn odd_butt_path_width_native_history_match_stock() {
        for case in 31..35 {
            verify_edit(case);
        }
    }
    #[test]
    #[ignore = "requires independent odd Butt sources"]
    fn odd_butt_locks_stale_cancel_atomic() {
        for case in 31..35 {
            verify_atomic(case);
        }
    }
    fn verify_edit(case: usize) {
        let doc = source_for(case);
        let initial = [
            "40c00000", "40400000", "40000000", "40c00000", "40c00000", "40c00000", "40c00000", "40e00000", "40e00000", "3f800000", "40c00000", "40c00000",
            "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000", "40c00000",
            "40c00000", "40400000", "40400000", "40400000", "40400000", "40400000", "40400000", "40400000", "40400000", "40400000", "40400000",
        ][case];
        let target = [
            "41000000", "40a00000", "40800000", "41000000", "41000000", "41000000", "41000000", "41100000", "41100000", "40e00000", "41000000", "41000000",
            "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000",
            "41000000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000",
        ][case];
        let expected = std::fs::read(packet_for(case).join("edit.ref.rgba-u8.bin")).unwrap();
        assert_eq!(
            blake3::hash(&expected).to_hex().as_str(),
            [
                "78fb96482f5d6b285f45145e9e072707d52e4c13b8cc1fee05f196ea5bad57ab",
                "cb2e47ebc4749c9ff47f9db488aa9a09a436cb072d9e3dbed2fe217f5d8f8b68",
                "627cb3d6bfb7709906a974007ea00b5f1d3b873d66b4e9e7849337b6d18ccefc",
                "53a360a2040120f1dcbd194a4ac55cf12690beeb441643d73b7dc428eccac6ce",
                "aea987017121b61806ce0cc7606e1760cbbbe2cec0d36f327e32c8d2c5ffba7a",
                "38074b6912fe9a2a793abdb8d54528f5fed338a5c5b83b5e019f9128e5cfbdb0",
                "dd3c466c4d92ae51dc73ea06aad2d06255d9d26184e0dc4a78c6425ae0f33129",
                "bba14b2ab82a52279a92f8d61caafc5af183a744fc636fa2514b676f3c82385c",
                "4a0bdb8de53f128c367f6748dee8a61baceb854a00940fa77b128831de2d9388",
                "67784ff863908e9300c1308d9ce237ec0adf95b704e978783653fd972bcabec3",
                "54bae46e5e7b37d3825a6e7b5f7f56953f43317d08a95be09bf5c76a5f5ed493",
                "002c74574025de001cf4636d67ebcbff1b5b72e4b36ec1e5c6647390103675dc",
                "47fdaeca5d67b037d90a7524d7958fe88b5ac43b9caba4a0f50185d55e0aed76",
                "6b6b6ae95fab549af10f42f4a7df57271687944bcdcacdb3bf7bfbe8987353b6",
                "8df8c0907db272b60ffd371a439c3b3510aac6a6bf9fd694e693c83ac54fedf0",
                "0e9c45aab479f00b482631c023a233d81a6cab45a6a9bdd5f1cf84396465dbef",
                "5c25d2676d706868e525af7a1998c311a8599239e1d93466500efe5f5362f10a",
                "4e7194725e1ced81613057f68a46ffaa6273bc73a2562317c0ccc614a7130e54",
                "c8107e37a14f78a16bd4b3d72e2612d512ce30825bc92eb093acb2bc502bbf2c",
                "54bae46e5e7b37d3825a6e7b5f7f56953f43317d08a95be09bf5c76a5f5ed493",
                "002c74574025de001cf4636d67ebcbff1b5b72e4b36ec1e5c6647390103675dc",
                "6b6b6ae95fab549af10f42f4a7df57271687944bcdcacdb3bf7bfbe8987353b6",
                "8df8c0907db272b60ffd371a439c3b3510aac6a6bf9fd694e693c83ac54fedf0",
                "5c25d2676d706868e525af7a1998c311a8599239e1d93466500efe5f5362f10a",
                "4e7194725e1ced81613057f68a46ffaa6273bc73a2562317c0ccc614a7130e54",
                "7040e87e5cac0c072e1a2516c7d81547d5a8a6d648268218954c502e9c7c09bd",
                "e285c2f320c12c51475a87e6d7bbaba04681bce27a4163cca5f60993f92d431c",
                "95c1f0a1b98943226e5268d8db3e7df427f295ffe1832954954a76b74742c4f5",
                "ec81895a3348e8ceeae2afccf95b06348a104112bffd5223e1b0c0eef9261c52",
                "149a6a3c947f269f96b82c0e3493fbcbe31693cc0dee61ce99360ffca73db104",
                "d3912498e1a91d9b7231040ea69f7bef4b8661332590ad1e41f82a51e3ae4e56",
                "8ab46bdefa278175f3e0f94dfe4c3b40c08b6b3cbcf8999ef5c468c5ae2dc507",
                "2c7d9de8890d9f33a097c507fc5cfc6ebc32cb228163cae8ec850f6fdaec654e",
                "42c74f3b43d2a4bb90e48bc08ec54b31673401920019f2c1b78e0092687ffb69",
                "02a277f4d6e3fa69a96ed0e65346e32edab41b767d4f0e12f17dc01566029d4e",
            ][case]
        );
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        assert_eq!(s.execute(COMMAND, params(&doc, initial)).unwrap()["updated"], false);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        s.execute("path.transform", json!({"pathId":doc.paths[0].id.0.to_string(),"translateX":2,"translateY":-1})).unwrap();
        let moved = (*s.active().unwrap().doc).clone();
        s.execute(COMMAND, params(&doc, target)).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        // The source color is untouched: the independently saved edit's tiny float drift is not needed.
        assert_eq!(projection(&edited), expected);
        assert_eq!(edited.layers, doc.layers);
        let mut expected_state = doc.xcf_state.clone().unwrap();
        let photocraft_doc::xcf_items::EditableSource::Vector { configuration, .. } = &mut expected_state.source_items[0].source else { panic!() };
        let photocraft_doc::xcf_filters::FilterValue::Config(stroke) = &mut configuration.iter_mut().find(|a| a.name == "stroke").unwrap().value else {
            panic!()
        };
        stroke.iter_mut().find(|a| a.name == "width").unwrap().value =
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(u32::from_str_radix(target, 16).unwrap()));
        assert_eq!(edited.xcf_state.as_ref().unwrap(), &expected_state);
        let native = photocraft_io::export(&edited, "stroke.pcraft", &Default::default()).unwrap();
        let mut native_expected = edited.clone();
        native_expected.name = "stroke.pcraft".into();
        assert_eq!(photocraft_io::import("stroke.pcraft", &native.bytes).unwrap().document, native_expected);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, moved);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        s.execute(COMMAND, params(&doc, initial)).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
    }
    #[test]
    #[ignore = "requires reviewed unpublished solid-stroke source"]
    fn stroke_locks_stale_ids_unsupported_and_late_cancel_are_atomic() {
        verify_atomic(0);
    }
    #[test]
    #[ignore = "requires reviewed unpublished AA solid-stroke source"]
    fn aa_stroke_locks_stale_ids_unsupported_and_late_cancel_are_atomic() {
        verify_atomic(1);
    }
    #[test]
    #[ignore = "requires reviewed unpublished even AA solid-stroke source"]
    fn even_aa_stroke_locks_stale_ids_unsupported_and_late_cancel_are_atomic() {
        verify_atomic(2);
    }
    #[test]
    #[ignore = "requires own bevel source and negative-origin holdout"]
    fn bevel_stroke_locks_stale_ids_and_cancel_are_atomic() {
        verify_atomic(3);
        verify_atomic(4);
    }
    #[test]
    #[ignore = "requires own round sources and negative-origin holdout"]
    fn round_stroke_locks_stale_ids_and_cancel_are_atomic() {
        verify_atomic(5);
        verify_atomic(6);
    }
    fn verify_atomic(case: usize) {
        let target = [
            "41000000", "40a00000", "40800000", "41000000", "41000000", "41000000", "41000000", "41100000", "41100000", "40e00000", "41000000", "41000000",
            "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000", "41000000",
            "41000000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000", "40a00000",
        ][case];
        for variant in 0..9 {
            let mut doc = source_for(case);
            if variant == 6 {
                let layers = std::mem::take(&mut doc.layers);
                doc.layers.push(photocraft_doc::Layer::group("Locked parent", layers));
                let parent = ItemRef::Layer(doc.layers[0].id);
                let mut state = doc.xcf_state.take().unwrap();
                state.reconcile_layers(&doc).unwrap();
                state.items.iter_mut().find(|i| i.item == parent).unwrap().locks.content = true;
                doc.xcf_state = Some(state);
            }
            if variant == 0 || variant == 1 {
                let owner = ItemRef::Layer(doc.layers[0].id);
                let locks = &mut doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == owner).unwrap().locks;
                if variant == 0 {
                    locks.content = true;
                } else {
                    locks.position = true;
                }
            }
            let mut p = params(&doc, target);
            if variant == 2 {
                p["pathId"] = json!("18446744073709551615");
            }
            if variant == 3 {
                p["layerId"] = json!("18446744073709551615");
            }
            if variant == 4 {
                p["widthF32Bits"] = json!("41300000");
            }
            if variant == 8 {
                // Each source stays in its independently qualified AA/style domain.
                p["widthF32Bits"] = json!(if matches!(case, 1 | 2 | 9) {
                    "41000000"
                } else if matches!(case, 10..=34) {
                    "3f800000"
                } else {
                    "40a00000"
                });
            }
            let mut s = Session::new();
            s.add_document(doc.clone(), None).unwrap();
            let cancelled = AtomicBool::new(variant == 7);
            let poll = || cancelled.load(Ordering::Relaxed);
            let progress = |_: f32| cancelled.store(true, Ordering::Relaxed);
            let ctl = Interrupt::new(&poll, &progress);
            let result = if variant == 5 || variant == 7 {
                photocraft_engine::xcf_vector_cmds::set_width_with_interrupt(&mut s, &p, &ctl)
            } else {
                s.execute(COMMAND, p)
            };
            assert!(result.is_err(), "variant {variant}");
            if variant == 6 {
                assert!(result.as_ref().unwrap_err().to_string().contains("locked"));
            }
            assert_eq!(*s.active().unwrap().doc, doc);
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            if variant == 5 || variant == 7 {
                assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
                assert!(cancelled.load(Ordering::Relaxed));
            }
        }
    }
}
