//! Proposed test-only shared verifier; known public and historical private domains.
//! No evaluator, model, source count or corpus publication authority.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    PublicNoIcc,
    PrivateCaptureV2,
}
pub const PUBLIC_RECEIPT: &str = "0caae6abf0c43ab623cd6745d6b99810189c66eb46a4ff7856dc348f007d2ab9";
pub const PUBLIC_INDEX: &str = "520d8671b0b49b1c3601e5db79ad8e235992b1dadbfc7060fe98830212898c3e";
pub const PUBLIC_PNG_INDEX: &str = "2da2aded8a4c2b0a7c9581f5c1720a2c1ab58c553cc8d9e6f8d52e5c57389d55";
pub const PRIVATE_RECEIPT: &str = "5f97f5fd469137b1d8b5c1da5a0468bcaecb50571a526f8e55ee0abd3b7d2744";
pub const PRIVATE_INDEX: &str = "dbd302bed5593566b9be2ff5f6c935db91c555209fbc20c783373971e8207418";
pub const PRIVATE_PNG_INDEX: &str = "4d7db9468636988ade1833028f11dcd0a833d35aeca6737ec846d3617e44b721";
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
pub fn bounded(path: &Path, cap: u64) -> Result<Vec<u8>> {
    if cap > 8 << 20 || fs::metadata(path)?.len() > cap {
        return Err("evidence byte cap".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(cap.checked_add(1).ok_or("cap overflow")?).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > cap {
        return Err("evidence grew beyond cap".into());
    }
    Ok(bytes)
}
fn relative(root: &Path, name: &str, private: bool) -> Result<PathBuf> {
    let name = if private { name.replace('\\', "/") } else { name.to_owned() };
    if name.is_empty() || name.len() > 256 || name.contains(['\\', ':', '\0']) || Path::new(&name).components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("evidence relative path".into());
    }
    let path = root.join(name).canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("evidence path escape".into());
    }
    Ok(path)
}
fn verify(root: &Path, name: &str, d: &Value, private: bool) -> Result<()> {
    let bytes = bounded(&relative(root, name, private)?, 8 << 20)?;
    if d.get("bytes").and_then(Value::as_u64) != Some(u64::try_from(bytes.len())?) || d.get("sha256").and_then(Value::as_str) != Some(hash(&bytes).as_str()) {
        return Err("evidence artifact hash/length".into());
    }
    Ok(())
}
fn reject_reparse(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err("evidence link refused".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("evidence reparse point refused".into());
        }
    }
    Ok(())
}
pub fn verify_public_tree(root: &Path, expected: &BTreeSet<String>) -> Result<()> {
    if expected.len() > 384 {
        return Err("public expected file count cap".into());
    }
    reject_reparse(root)?;
    let root = root.canonicalize()?;
    let mut pending = vec![root.clone()];
    let mut actual = BTreeSet::new();
    let mut visited = 0usize;
    let mut directory_count = 1usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            visited = visited.checked_add(1).filter(|v| *v <= 384).ok_or("public total entry cap")?;
            let entry = entry?;
            reject_reparse(&entry.path())?;
            let path = entry.path().canonicalize()?;
            if !path.starts_with(&root) {
                return Err("public tree escape".into());
            }
            let name = path.strip_prefix(&root)?.to_str().ok_or("public UTF8")?.replace('\\', "/");
            if path.is_dir() {
                directory_count = directory_count.checked_add(1).filter(|v| *v <= 2).ok_or("public directory cap")?;
                if name != "packet" || !pending.is_empty() {
                    return Err("unexpected public directory".into());
                }
                pending.push(path);
            } else if path.is_file() {
                if !expected.contains(&name) || !actual.insert(name) {
                    return Err("unlisted public file".into());
                }
            } else {
                return Err("public special file".into());
            }
        }
    }
    if &actual != expected {
        return Err("public disk allowlist".into());
    }
    Ok(())
}
pub fn artifact(packet: &Path, d: &Value) -> Result<Vec<u8>> {
    let packet = packet.canonicalize()?;
    let name = d.get("file").and_then(Value::as_str).ok_or("artifact file")?;
    if name.contains(['/', '\\']) {
        return Err("artifact basename".into());
    }
    let bytes = bounded(&relative(&packet, name, false)?, 4 << 20)?;
    if d.get("bytes").and_then(Value::as_u64) != Some(u64::try_from(bytes.len())?) || d.get("sha256").and_then(Value::as_str) != Some(hash(&bytes).as_str()) {
        return Err("artifact hash/length".into());
    }
    Ok(bytes)
}
#[allow(dead_code)] // Shared test consumers use different fields.
pub struct Evidence {
    pub domain: Domain,
    pub root: PathBuf,
    pub packet: PathBuf,
    pub records: Vec<Value>,
    pub png_index: Value,
}
#[allow(dead_code)] // Shared test consumers use different accessors.
impl Evidence {
    pub fn record(&self, id: &str) -> Result<&Value> {
        self.records.iter().find(|r| r.get("id").and_then(Value::as_str) == Some(id)).ok_or_else(|| "closed control ID".into())
    }
    pub fn open(root: &Path, domain: Domain) -> Result<Self> {
        reject_reparse(root)?;
        let root = root.canonicalize()?;
        let (receipt_file, receipt_hash, index_hash, png_hash) = match domain {
            Domain::PublicNoIcc => ("public-receipt.json", PUBLIC_RECEIPT, PUBLIC_INDEX, PUBLIC_PNG_INDEX),
            Domain::PrivateCaptureV2 => ("actual-packet-receipt.json", PRIVATE_RECEIPT, PRIVATE_INDEX, PRIVATE_PNG_INDEX),
        };
        let bytes = bounded(&root.join(receipt_file), 4 << 20)?;
        if hash(&bytes) != receipt_hash {
            return Err("known evidence domain receipt identity".into());
        }
        let receipt: Value = serde_json::from_slice(&bytes)?;
        if domain == Domain::PublicNoIcc {
            let indices = receipt.get("public_indices").and_then(Value::as_array).ok_or("public indices")?;
            let artifacts = receipt.get("artifacts").and_then(Value::as_array).ok_or("public artifacts")?;
            if indices.len() != 5 || artifacts.len() != 222 {
                return Err("closed public packet count".into());
            }
            let mut seen = BTreeSet::new();
            for d in indices.iter().chain(artifacts) {
                let name = d.get("path").and_then(Value::as_str).ok_or("public path")?;
                if !seen.insert(name) || name.ends_with(".icc") || name.ends_with(".png-icc.bin") {
                    return Err("duplicate or prohibited public asset".into());
                }
                verify(&root, name, d, false)?;
            }
            seen.insert(receipt_file);
            verify_public_tree(&root, &seen.into_iter().map(str::to_owned).collect())?;
        } else {
            let files = receipt.get("files").and_then(Value::as_object).ok_or("historical private files")?;
            if files.is_empty() || files.len() > 384 {
                return Err("private file count cap".into());
            }
            for (name, d) in files {
                verify(&root, name, d, true)?;
            }
        }
        let index = bounded(&root.join("packet/reopen_edit.result.json"), 4 << 20)?;
        if hash(&index) != index_hash {
            return Err("known domain control index identity".into());
        }
        let index: Value = serde_json::from_slice(&index)?;
        let records = index.get("records").and_then(Value::as_array).ok_or("control records")?.clone();
        let expected = [
            "negative-owner.source",
            "negative-owner.edit",
            "negative-owner.disabled",
            "tile-crossing-owner.source",
            "tile-crossing-owner.edit",
            "tile-crossing-owner.disabled",
        ];
        if records.len() != expected.len() || records.iter().zip(expected).any(|(r, id)| r.get("id").and_then(Value::as_str) != Some(id)) {
            return Err("closed six control identities".into());
        }
        let bytes = bounded(&root.join("independent-png-decode.json"), 2 << 20)?;
        if hash(&bytes) != png_hash {
            return Err("known domain PNG index identity".into());
        }
        Ok(Self { domain, packet: root.join("packet"), root, records, png_index: serde_json::from_slice(&bytes)? })
    }
    pub fn state(&self, id: &str, stage: &str) -> Result<Value> {
        let row = self.records.iter().find(|r| r.get("id").and_then(Value::as_str) == Some(id)).ok_or("closed control ID")?;
        let v = match stage {
            "reopened" => row.get("current"),
            "baked" => row.get("baked").and_then(|v| v.get("after")),
            "baked.before" => row.get("baked").and_then(|v| v.get("before")),
            "live" => row.get("live"),
            _ => return Err("closed state stage".into()),
        }
        .ok_or("control state")?;
        if !v.is_object() {
            return Err("state object".into());
        }
        Ok(v.clone())
    }
    pub fn assert_implicit_profile(&self, state: &Value) -> Result<()> {
        let profiles = state.get("profiles").ok_or("profiles")?;
        if profiles.get("assigned") != Some(&Value::Null) {
            return Err("assigned profile forbidden".into());
        }
        match self.domain {
            Domain::PublicNoIcc => {
                let effective = profiles.get("effective").ok_or("implicit profile metadata")?;
                if effective
                    != &serde_json::json!({"semantic":"implicit GIMP built-in sRGB observed privately; no profile bytes enrolled","source_guard":"assigned profile absent"})
                {
                    return Err("public implicit-only profile contract".into());
                }
            }
            Domain::PrivateCaptureV2 => {
                let bytes = artifact(&self.packet, profiles.get("effective").ok_or("private effective profile")?)?;
                if hash(&bytes) != "2c41652d6d02673a1d161345907b4bd43dd24742465bae38271e00326a050b83" {
                    return Err("historical private effective profile identity".into());
                }
            }
        }
        Ok(())
    }
    pub fn assert_profile_png_contract(&self, state: &Value, png_row: &Value) -> Result<()> {
        self.assert_implicit_profile(state)?;
        match self.domain {
            Domain::PublicNoIcc => {
                if png_row.pointer("/observation/icc") != Some(&Value::Null)
                    || png_row.pointer("/provenance/kind").and_then(Value::as_str) != Some("owned deterministic derivative PNG, not GIMP-exported PNG")
                {
                    return Err("public derivative pixel-code oracle contract".into());
                }
            }
            Domain::PrivateCaptureV2 => {
                if png_row.pointer("/observation/icc/sha256").and_then(Value::as_str)
                    != Some("2c41652d6d02673a1d161345907b4bd43dd24742465bae38271e00326a050b83")
                {
                    return Err("historical private PNG/profile binding".into());
                }
            }
        }
        Ok(())
    }
}
pub fn selection(cargo_manifest_dir: &str) -> Result<(PathBuf, Domain)> {
    let public = std::env::var_os("PHOTOCRAFT_RED_EYE_GEOMETRY_PUBLIC_PACKET");
    let private_a = std::env::var_os("PHOTOCRAFT_RED_EYE_GEOMETRY_PACKET");
    let private_b = std::env::var_os("PHOTOCRAFT_RED_EYE_GEOMETRY");
    if public.is_some() && (private_a.is_some() || private_b.is_some()) || private_a.is_some() && private_b.is_some() && private_a != private_b {
        return Err("ambiguous evidence domain; never fallback on hash failure".into());
    }
    if let Some(root) = public {
        return Ok((PathBuf::from(root), Domain::PublicNoIcc));
    }
    if let Some(root) = private_a.or(private_b) {
        return Ok((PathBuf::from(root), Domain::PrivateCaptureV2));
    }
    Ok((PathBuf::from(cargo_manifest_dir).join("../../../photocraft-corpus/xcf/full/red-eye-owner-geometry-evidence"), Domain::PublicNoIcc))
}
pub fn selected(cargo_manifest_dir: &str) -> Result<Evidence> {
    let (root, domain) = selection(cargo_manifest_dir)?;
    Evidence::open(&root, domain)
}
