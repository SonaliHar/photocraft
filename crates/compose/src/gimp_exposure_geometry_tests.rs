use super::*;
use photocraft_doc::{
    import_compositing::GimpComposition,
    item_identity::FilterId,
    xcf_filters::{FilterArgument, OperationIdentity},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn read(root: &Path, descriptor: &Value) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let path = root.join(descriptor["file"].as_str().ok_or("missing sidecar path")?);
    if std::fs::metadata(&path)?.len() > 2 << 20 {
        return Err("compressed sidecar limit".into());
    }
    let raw = std::fs::read(path)?;
    assert_eq!(hash(&raw), descriptor["sha256"]);
    let limit = descriptor["decoded_bytes"].as_u64().ok_or("missing decoded length")?;
    assert!(limit <= 2 << 20);
    let mut output = Vec::new();
    flate2::read::ZlibDecoder::new(raw.as_slice()).take(limit + 1).read_to_end(&mut output)?;
    assert_eq!(output.len() as u64, limit);
    assert_eq!(hash(&output), descriptor["decoded_sha256"]);
    Ok(output)
}
#[test]
#[ignore = "local unpublished original Exposure offset/tile-boundary evidence"]
fn independent_offset_exposure_native_alpha_and_whole_drawable() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("exposure-geometry-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "bc993f6a3334c63fd5e8036409e8d5a3a955ef51de8b4949d6030d3b730233c2");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(hash(&std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap()), artifact["sha256"]);
    }
    let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut max: f64 = 0.;
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let gray = row["model"] == "GRAY";
        let channels = if gray { 1 } else { 3 };
        let precision = row["precision"].as_str().unwrap();
        let width = if precision.starts_with("U16") {
            2
        } else if precision.starts_with("FLOAT") {
            4
        } else {
            8
        };
        let nonlinear = precision.ends_with("NON_LINEAR");
        let encoding = if nonlinear { SampleEncoding::NonlinearProfile } else { SampleEncoding::Linear };
        let epoch = "xcf-22-stock-exposure-offset-geometry-v1";
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: epoch.into() },
            arguments: ["black-level", "exposure"]
                .into_iter()
                .map(|name| FilterArgument {
                    name: name.into(),
                    value: FilterValue::Number(ExactNumber::Float32Bits((row["requested"][name].as_f64().unwrap() as f32).to_bits())),
                })
                .collect(),
            source_display_name: None,
            source_icon: None,
            enabled: row["requested"]["enabled"].as_bool().unwrap(),
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: epoch.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -901479855,
                stored_composite_space: -901479855,
                stored_operator: -901479855,
                resolved: None,
                pass_through: false,
            }),
        };
        let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let prepared = PointStack::from_nodes(std::slice::from_ref(&node), profile, encoding, 22).unwrap();
        for kind in 0..7 {
            let mut invalid = node.clone();
            invalid.enabled = true;
            match kind {
                0 => invalid.operation.schema_epoch = "unknown".into(),
                1 => invalid.operation.stored_version = "0:0".into(),
                2 => invalid.opacity_f64_bits = 0.5_f64.to_bits(),
                3 => invalid.source_region = Some(1),
                4 => invalid.composition.as_mut().unwrap().stored_operator = 1,
                5 => invalid.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.0625_f64.to_bits())),
                _ => invalid.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
            }
            assert!(PointStack::from_nodes(&[invalid], profile, encoding, 22).is_err());
        }
        let source = read(&root, &row["reopened"]["layers"][0]["native"]).unwrap();
        let expected = read(&root, &row["baked"]["layers"][0]["native"]).unwrap();
        assert_eq!(source.len(), expected.len());
        let decode = |b: &[u8]| match width {
            2 => f64::from(u16::from_le_bytes(b[..2].try_into().unwrap())) / 65535.,
            4 => f64::from(f32::from_le_bytes(b[..4].try_into().unwrap())),
            _ => f64::from_le_bytes(b[..8].try_into().unwrap()),
        };
        for (a, b) in source.chunks_exact((channels + 1) * width).zip(expected.chunks_exact((channels + 1) * width)) {
            let alpha = decode(&a[channels * width..]);
            let input = if gray { [decode(a); 3] } else { [decode(a), decode(&a[width..]), decode(&a[2 * width..])] };
            let actual = prepared.apply([input[0], input[1], input[2], alpha], &mut |_| Ok(())).unwrap();
            assert_eq!(actual[3].to_bits(), alpha.to_bits());
            assert_eq!(&a[channels * width..], &b[channels * width..]);
            for (channel, value) in actual.iter().take(channels).enumerate() {
                // Integer generated bakes encode finite output in the unit
                // storage range. Preserve floating HDR; do not alter sources.
                let stored = if width == 2 { value.clamp(0., 1.) } else { *value };
                let delta = (stored - decode(&b[channel * width..])).abs();
                max = max.max(delta);
                assert!(delta <= 2. / 255., "{} native delta {delta}", row["id"]);
            }
        }
        count += 1;
    }
    assert_eq!(count, 72);
    println!("Exposure geometry72 full-drawable generated native max {}/255", max * 255.);
}
