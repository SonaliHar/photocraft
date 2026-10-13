#[cfg(test)]
mod tests {
    use crate::gimp_filters::PointStack;
    use photocraft_doc::{
        SampleEncoding,
        import_compositing::GimpComposition,
        item_identity::FilterId,
        xcf_filters::{FilterArgument, FilterNode, FilterValue, OperationIdentity},
    };
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    fn data(root: &std::path::Path, descriptor: &Value) -> Vec<u8> {
        let encoded = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
        let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(hash(&encoded), descriptor["sha256"].as_str().unwrap());
        let limit = descriptor["decoded_bytes"].as_u64().unwrap();
        assert!(limit <= 2 << 20);
        let mut output = Vec::new();
        flate2::read::ZlibDecoder::new(encoded.as_slice()).take(limit + 1).read_to_end(&mut output).unwrap();
        assert_eq!(output.len() as u64, limit);
        assert_eq!(hash(&output), descriptor["decoded_sha256"].as_str().unwrap());
        output
    }
    #[test]
    #[ignore = "local unpublished132 independent Component Extract geometry native controls"]
    fn original_geometry_native_opaque_and_disabled_controls() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/point-component-extract-geometry-evidence");
        let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(
            Sha256::digest(&manifest).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "fe13672593245e18d8224faec99c18a3bfcd10cd5860427f60494e5d29396e88"
        );
        let observations: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
        let rows = observations["records"].as_array().unwrap();
        assert_eq!(rows.len(), 132);
        let mut maximum = 0f64;
        for row in rows {
            let component = match row["component"].as_str().unwrap() {
                "rgb-r" => 0,
                "rgb-g" => 1,
                "rgb-b" => 2,
                "alpha" => 20,
                "cmyk-c" => 8,
                "cmyk-m" => 9,
                "cmyk-y" => 10,
                "cmyk-k" => 11,
                "ycbcr-y" => 12,
                "ycbcr-cb" => 13,
                "ycbcr-cr" => 14,
                _ => panic!("unqualified geometry selector"),
            };
            let node = FilterNode {
                id: FilterId(1),
                operation: OperationIdentity {
                    name: "gegl:component-extract".into(),
                    stored_version: "0:0".into(),
                    schema_epoch: "xcf-22-stock-component-extract-geometry-v1".into(),
                },
                arguments: vec![
                    FilterArgument {
                        name: "component".into(),
                        value: FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: component },
                    },
                    FilterArgument { name: "invert".into(), value: FilterValue::Bool(row["invert"].as_bool().unwrap()) },
                    FilterArgument { name: "linear".into(), value: FilterValue::Bool(row["linear"].as_bool().unwrap()) },
                ],
                source_display_name: None,
                source_icon: None,
                enabled: row["enabled"].as_bool().unwrap(),
                opacity_f64_bits: 1f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: Some(GimpComposition {
                    compatibility_epoch: "xcf-22-stock-point-replace-fields-v1".into(),
                    source_mode: 62,
                    legacy: false,
                    stored_blend_space: 87327313,
                    stored_composite_space: 87327313,
                    stored_operator: 87327313,
                    resolved: None,
                    pass_through: false,
                }),
            };
            let before = node.clone();
            let gray = row["model"] == "GRAY";
            let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
            let stack = PointStack::from_nodes(std::slice::from_ref(&node), profile, SampleEncoding::NonlinearProfile, 22).unwrap();
            let extent = photocraft_geom::Rect::from_xywh(
                row["offset"][0].as_i64().unwrap() as i32,
                row["offset"][1].as_i64().unwrap() as i32,
                row["drawable"][0].as_u64().unwrap() as u32,
                row["drawable"][1].as_u64().unwrap() as u32,
            );
            let ordered = crate::gimp_neighborhood::OrderedStack::new(
                &photocraft_doc::xcf_filters::FilterStack {
                    owner: photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
                    nodes: vec![node.clone()],
                    revision: 0,
                },
                profile,
                SampleEncoding::NonlinearProfile,
                22,
                extent,
                [false; 3],
            )
            .unwrap();
            let abyss = [0.75, 0.25, -0.0, -0.0];
            let outside = ordered.point_at(abyss, (extent.x0 - 1, extent.y0), &mut |_| Ok(())).unwrap();
            assert_eq!(outside.map(f64::to_bits), abyss.map(f64::to_bits));
            let inside = ordered.point_at([0.25, 0.5, 0.75, 0.125], (extent.x0, extent.y0), &mut |_| Ok(())).unwrap();
            assert_eq!(inside[3], if node.enabled { 1. } else { 0.125 });

            let source = data(&root, &row["current"]["layers"][0]["native"]);
            let baked = data(&root, &row["baked"]["layers"][0]["native"]);
            let channels = if gray { 2 } else { 4 };
            assert_eq!(source.len(), baked.len());
            for (input, expected) in source.chunks_exact(channels * 4).zip(baked.chunks_exact(channels * 4)) {
                let sample = |bytes: &[u8], index: usize| f64::from(f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap()));
                let input = if gray {
                    [sample(input, 0), sample(input, 0), sample(input, 0), sample(input, 1)]
                } else {
                    std::array::from_fn(|axis| sample(input, axis))
                };
                let actual = stack.apply_current(input, false, &mut |_| Ok(())).unwrap().map(|value| f64::from(value as f32));
                let expected_alpha = sample(expected, channels - 1);
                assert_eq!(actual[3], expected_alpha, "{} alpha", row["id"]);
                for (axis, actual) in actual.iter().take(if gray { 1 } else { 3 }).enumerate() {
                    let delta = (actual * expected_alpha - sample(expected, axis) * expected_alpha).abs();
                    maximum = maximum.max(delta);
                    assert!(delta <= 2. / 255., "{} {} /255", row["id"], delta * 255.);
                }
                if !node.enabled {
                    assert_eq!(actual.map(f64::to_bits), input.map(f64::to_bits));
                }
            }
            assert_eq!(node, before);
        }
        println!("132 Component geometry native/disabled controls max {}/255", maximum * 255.);
    }
}
