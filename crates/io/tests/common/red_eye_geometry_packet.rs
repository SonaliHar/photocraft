//! Exact owner-geometry witnesses; historical precision108 and synthetic controls remain separate.
use serde_json::Value;
#[path = "../../../xcf/tests/common/red_eye_geometry_evidence.rs"]
mod evidence;
pub use evidence::{bounded, hash};
use std::path::{Path, PathBuf};

pub type TestResult<T> = Result<T, Box<dyn std::error::Error>>;
pub const OLD_EPOCH: &str = "xcf-22-stock-red-eye-removal-precision-v1";
pub const NEW_EPOCH: &str = "xcf-22-stock-red-eye-removal-owner-geometry-v1";
pub const OLD_POLICY: &str = "xcf-22-stock-point-replace-fields-v1";

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub id: &'static str,
    pub canvas: (u32, u32),
    pub owner: (u32, u32),
    pub offset: (i32, i32),
}
pub const GEOMETRIES: [Geometry; 2] = [
    Geometry { id: "negative-owner", canvas: (37, 29), owner: (19, 13), offset: (-7, 5) },
    Geometry { id: "tile-crossing-owner", canvas: (259, 17), owner: (262, 9), offset: (-3, -4) },
];
#[allow(dead_code)] // Shared test consumers use different evidence accessors.
fn selected_for_packet(root: &Path) -> TestResult<evidence::Evidence> {
    let value = evidence::selected(env!("CARGO_MANIFEST_DIR"))?;
    if value.packet.canonicalize()? != root.canonicalize()? {
        return Err("selected evidence packet changed".into());
    }
    Ok(value)
}
pub fn fresh_records() -> TestResult<(PathBuf, Vec<Value>)> {
    let value = evidence::selected(env!("CARGO_MANIFEST_DIR"))?;
    Ok((value.packet, value.records))
}
pub fn fresh_artifact(root: &Path, descriptor: &Value) -> TestResult<Vec<u8>> {
    evidence::artifact(root, descriptor)
}
#[allow(dead_code)] // Shared test consumers use different evidence accessors.
pub fn fresh_png_index(root: &Path) -> TestResult<Value> {
    Ok(selected_for_packet(root)?.png_index)
}
#[allow(dead_code)] // Shared test consumers use different evidence accessors.
pub fn fresh_profile_png_contract(root: &Path, state: &Value, png_row: &Value) -> TestResult<()> {
    selected_for_packet(root)?.assert_profile_png_contract(state, png_row)
}
pub fn old_source() -> TestResult<Vec<u8>> {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("red-eye-removal-precision-evidence");
    let manifest = bounded(&root.join("batch-manifest.json"), 2 << 20)?;
    if hash(&manifest) != "821fad4b046cae0174291934fa61b01f5fc966cd4d612e4ec77eeb0cb6fb6e5f" {
        return Err("old Red Eye manifest identity".into());
    }
    let manifest: Value = serde_json::from_slice(&manifest)?;
    let observation = bounded(&root.join("observations.json"), 4 << 20)?;
    let entry = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or("old artifacts list")?
        .iter()
        .find(|item| item.get("path").and_then(Value::as_str) == Some("observations.json"))
        .ok_or("old observations identity")?;
    if entry.get("sha256").and_then(Value::as_str) != Some(hash(&observation).as_str()) {
        return Err("old observations hash".into());
    }
    let observation: Value = serde_json::from_slice(&observation)?;
    let rows = observation.get("records").and_then(Value::as_array).ok_or("old records")?;
    if rows.len() != 108 {
        return Err("old record count".into());
    }
    let row = rows.iter().find(|row| row.get("id").and_then(Value::as_str) == Some("rgb.float_non_linear.source")).ok_or("old RGB Float nonlinear control")?;
    let bytes = bounded(&root.join("rgb.float_non_linear.source.xcf"), 2 << 20)?;
    if row.get("file").and_then(Value::as_str) != Some("rgb.float_non_linear.source.xcf")
        || row.get("sha256").and_then(Value::as_str) != Some(hash(&bytes).as_str())
    {
        return Err("old source identity".into());
    }
    Ok(bytes)
}

/// Procedural unit pixels only. They are not a GIMP reference or source claim.
pub fn synthetic_native(owner: (u32, u32)) -> TestResult<Vec<u8>> {
    let pixels = usize::try_from(owner.0)?.checked_mul(usize::try_from(owner.1)?).ok_or("synthetic pixel overflow")?;
    let bytes = pixels.checked_mul(16).filter(|bytes| *bytes <= 128 << 10).ok_or("synthetic byte cap")?;
    let mut native = Vec::new();
    native.try_reserve_exact(bytes)?;
    for index in 0..pixels {
        let alpha = if index % 11 == 0 { 0.0f32 } else { 1.0f32 };
        for value in [0.8f32, 0.08, 0.15, alpha] {
            native.extend_from_slice(&value.to_ne_bytes());
        }
    }
    Ok(native)
}

/// Resize a decoded OLD control solely to observe its present admission refusal.
/// Stored property offsets/payloads remain old; this is not serialized evidence.
pub fn synthetic_decoded(geometry: Geometry, enabled: bool) -> TestResult<photocraft_xcf::XcfFile> {
    let mut file = photocraft_xcf::decode(&old_source()?, &Default::default())?;
    file.width = geometry.canvas.0;
    file.height = geometry.canvas.1;
    let owner = file.layers.first_mut().ok_or("old owner")?;
    owner.width = geometry.owner.0;
    owner.height = geometry.owner.1;
    owner.offset = geometry.offset;
    let native = owner.plane.as_mut().ok_or("old native plane")?;
    native.width = geometry.owner.0;
    native.height = geometry.owner.1;
    native.bytes = synthetic_native(geometry.owner)?;
    let effect = owner.effects.first_mut().ok_or("old effect")?;
    effect.state.enabled = Some(enabled);
    let mask = effect.mask.as_mut().ok_or("old filter mask")?;
    mask.width = geometry.canvas.0;
    mask.height = geometry.canvas.1;
    let native = mask.plane.as_mut().ok_or("old mask plane")?;
    native.width = geometry.canvas.0;
    native.height = geometry.canvas.1;
    let bytes = usize::try_from(geometry.canvas.0)?
        .checked_mul(usize::try_from(geometry.canvas.1)?)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= 128 << 10)
        .ok_or("synthetic mask bytes")?;
    native.bytes = vec![0; bytes];
    Ok(file)
}
