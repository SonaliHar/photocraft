use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub const EPOCH: &str = "xcf-22-stock-video-degradation-options-rgb-f32-nonlinear-v1";
pub fn root() -> PathBuf {
    std::env::var_os("PHOTOCRAFT_VIDEO_OPTIONS_PACKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-video-expansion-author-20261012/packet"))
}
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
pub fn observations() -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(root().join("observations.json"))?;
    assert_eq!(hash(&bytes), "522cb39a8277c690cccf4bc977c818d32e8613f43e59abc73f620a576cc17bfb");
    let value: Value = serde_json::from_slice(&bytes)?;
    assert!(value["errors"].as_array().ok_or("missing packet errors list")?.is_empty());
    assert_eq!(value["records"].as_array().ok_or("missing packet records list")?.len(), 108);
    assert_eq!(value["generator_sha256"], "45f662ff4384baef7448342163261993bd7ebe03b573ffdf78c42d2ea92f8222");
    let fresh = std::fs::read(root().join("fresh.json"))?;
    assert_eq!(hash(&fresh), "6f1ce248cc1cc5cc93a4e86e25b8309682084e52adc1f8c534eea484c4db9bde");
    Ok(value)
}
pub fn blob(root: &Path, entry: &Value) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(root.join(entry["file"].as_str().ok_or("missing packet blob filename")?))?;
    assert_eq!(hash(&bytes), entry["sha256"]);
    if let Some(count) = entry["bytes"].as_u64() {
        assert_eq!(bytes.len() as u64, count);
    }
    Ok(bytes)
}
