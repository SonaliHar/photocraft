//! Decode independently authored geometry controls before extending admission.
use photocraft_raster::Interrupt;
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
#[ignore = "requires reviewed unpublished original Component geometry controls"]
fn component_geometry_source_decode_observations() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-geometry-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "fe13672593245e18d8224faec99c18a3bfcd10cd5860427f60494e5d29396e88");
    let value: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let mut observations = Vec::new();
    for case in value["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(case["source"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), case["sha256"]);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(file.layers.len(), 1);
        let owner = &file.layers[0];
        assert_eq!(owner.effects.len(), 1);
        let effect = &owner.effects[0];
        let mask = effect.mask.as_ref().unwrap();
        let plane = mask.plane.as_ref().unwrap();
        assert_eq!(effect.operation, "gegl:component-extract");
        assert_eq!(file.sample, photocraft_xcf::Sample::F32);
        assert_eq!(file.encoding, photocraft_xcf::Encoding::NonlinearProfile);
        let geometry = ((file.width, file.height), (owner.width, owner.height), owner.offset);
        assert!(matches!(geometry, ((37, 29), (19, 13), (-7, 5)) | ((259, 17), (262, 9), (-3, -4))));
        // The exact observed geometry receives its own epoch; the earlier
        // decoder-only None checkpoint remains frozen in its historical log.
        assert!(
            super::classify(&file, &Interrupt::NONE, None).unwrap()
                == Some(crate::xcf_filters::NeighborhoodDomain::PointReplace(crate::xcf_filters::PointReplaceFamily::ComponentExtractGeometry))
        );
        observations.push(serde_json::json!({
            "id":case["id"], "canvas":[file.width,file.height],
            "drawable":[owner.width,owner.height],"offset":owner.offset,
            "owner_mode":owner.mode,"owner_tuple":[owner.blend_space,owner.composite_space,owner.composite_mode],
            "effect_tuple":[effect.state.blend_space,effect.state.composite_space,effect.state.composite_mode],
            "effect_mode":effect.state.mode,"clipping":effect.state.clipping,"region":effect.state.region,
            "mask_size":[mask.width,mask.height],"mask_offset":mask.offset,
            "mask_tuple":[mask.blend_space,mask.composite_space,mask.composite_mode],
            "mask_plane_size":[plane.width,plane.height],"mask_sample":format!("{:?}",plane.sample),
            "mask_channels":plane.channels,"mask_all_zero":plane.bytes.iter().all(|b|*b==0),
            "mask_bytes":plane.bytes.len()
        }));
    }
    assert_eq!(observations.len(), 132);
    println!("COMPONENT_GEOMETRY_DECODE={}", serde_json::to_string(&observations).unwrap());
}

#[test]
#[ignore = "requires reviewed unpublished original Component geometry controls"]
fn component_geometry_source_rejects_unobserved_contexts() {
    use photocraft_xcf::{Encoding, Model, Sample};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-component-extract-geometry-evidence");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for case in value["cases"].as_array().unwrap() {
        let path = root.join(case["source"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let key = (format!("{:?}", file.model), file.width, file.height);
        if !seen.insert(key) {
            continue;
        }
        assert!(super::classify(&file, &Interrupt::NONE, None).unwrap().is_some());
        for change in 0..28 {
            let mut bad = file.clone();
            match change {
                0 => bad.precision = 600,
                1 => bad.encoding = Encoding::Linear,
                2 => bad.sample = Sample::F64,
                3 => bad.icc = vec![1],
                4 => bad.model = Model::Indexed,
                5 => bad.width += 1,
                6 => bad.height += 1,
                7 => bad.layers[0].width += 1,
                8 => bad.layers[0].height += 1,
                9 => bad.layers[0].offset.0 += 1,
                10 => bad.layers[0].offset.1 += 1,
                11 => bad.layers[0].mode = 30,
                12 => bad.layers[0].opacity = 0.5,
                13 => bad.layers[0].effects[0].state.blend_space = Some(-305954223),
                14 => bad.layers[0].effects[0].state.composite_space = Some(-1710980527),
                15 => bad.layers[0].effects[0].state.composite_mode = Some(0),
                16 => bad.layers[0].effects[0].state.enabled = None,
                17 => bad.layers[0].effects[0].state.mode = Some(28),
                18 => bad.layers[0].effects[0].state.clipping = Some(1),
                19 => bad.layers[0].effects[0].state.region = Some(1),
                20 => bad.layers[0].plane.as_mut().unwrap().bytes[..4].copy_from_slice(&1.5f32.to_ne_bytes()),
                21 => bad.layers[0].plane.as_mut().unwrap().width += 1,
                22 => bad.layers[0].effects[0].mask.as_mut().unwrap().width += 1,
                23 => bad.layers[0].effects[0].mask.as_mut().unwrap().offset = (1, 0),
                24 => bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().bytes[0] = 1,
                25 => bad.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap().sample = Sample::U32,
                26 => bad.layers[0].blend_space = 2,
                _ => bad.layers[0].effects[0].mask.as_mut().unwrap().mask_shown = true,
            }
            assert!(super::classify(&bad, &Interrupt::NONE, None).unwrap().is_none(), "{} change{change}", case["id"]);
        }
        let cancel = || true;
        assert!(matches!(super::classify(&file, &Interrupt::cancel_only(&cancel), None), Err(crate::IoError::Cancelled)));
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", 0)]).unwrap();
        let budget = photocraft_xcf::Budget::new(limits, None);
        assert!(matches!(super::classify(&file,&Interrupt::NONE,Some(&budget)),Err(crate::IoError::Xcf(error))if error.kind==photocraft_xcf::ErrorKind::Limit));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    assert_eq!(seen.len(), 4);
}
