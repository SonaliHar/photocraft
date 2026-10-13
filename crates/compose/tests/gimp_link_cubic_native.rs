#![cfg(feature = "corpus")]
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_raster::Interrupt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
fn bytes(root: &Path, value: &Value) -> Vec<u8> {
    let data = std::fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
    assert_eq!(Sha256::digest(&data).iter().map(|value| format!("{value:02x}")).collect::<String>(), value["sha256"].as_str().unwrap());
    data
}
#[test]
#[ignore = "requires isolated independently captured RGBA8 Cubic packet; set PHOTOCRAFT_XCF_CUBIC_PACKET"]
fn rgba8_cubic_link_sampler_matches_independent_hidden_alpha_native_geometry() {
    let packet_root = std::env::var("PHOTOCRAFT_XCF_CUBIC_PACKET").unwrap();
    let base = Path::new(&packet_root);
    check_packet(&base.join("packet"));
    check_packet(&base.join("hidden-packet"));
}
fn check_packet(root: &Path) {
    let packet: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for record in packet["records"].as_array().unwrap() {
        if !record["id"].as_str().unwrap().starts_with("u8_non_linear-rgb-") {
            continue;
        }
        let source = &record["fresh_source"];
        let edited = &record["fresh_edit"];
        let raw = bytes(root, &source["native"]);
        assert!(raw.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0 && pixel[..3].iter().any(|value| *value != 0)));
        let expected = bytes(root, &edited["native"]);
        let profile = bytes(root, &source["profiles"]["effective"]);
        let offset = |state: &Value, index: usize| state["offset"][index].as_i64().unwrap() as i32;
        let size = |state: &Value, index: usize| state["size"][index].as_u64().unwrap() as u32;
        let bounds = Rect::new(offset(edited, 0), offset(edited, 1), offset(edited, 0) + size(edited, 0) as i32, offset(edited, 1) + size(edited, 1) as i32);
        let state = TransformState {
            matrix_f64_bits: record["matrix"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap().to_bits()).collect::<Vec<_>>().try_into().unwrap(),
            pre_offset: [offset(source, 0), offset(source, 1)],
            interpolation: 2,
        };
        let actual = photocraft_compose::gimp_link_cubic::transform(
            &raw,
            size(source, 0),
            size(source, 1),
            PixelFormat::RGBA8,
            &state,
            bounds,
            &profile,
            &Interrupt::NONE,
        )
        .unwrap()
        .try_read_interleaved(bounds)
        .unwrap();
        assert_eq!(actual.len(), expected.len());
        let maximum = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(maximum <= 2, "{} native {maximum}/255", record["id"]);
        count += 1;
    }
    assert_eq!(count, 3);
}

