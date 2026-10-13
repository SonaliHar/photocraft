//! Model files: where they live and how they are fetched.
//!
//! Models are never committed (they are large and licensed on their own terms). The first use
//! downloads the file into the user's cache folder; the SHA-256 is pinned, so a corrupted or
//! swapped file is rejected. `PHOTOCRAFT_MODEL_DIR` overrides the folder (offline installs: drop
//! the file there).

use std::path::PathBuf;

use crate::{MlError, Result};

/// A downloadable model file.
#[derive(Clone, Copy, Debug)]
pub struct ModelSpec {
    pub id: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// Lower-case hex SHA-256 of the file.
    pub sha256: &'static str,
    pub bytes: u64,
    pub licence: &'static str,
}

/// IS-Net "general use" (DIS, Xuebin Qin et al., ECCV 2022; Apache-2.0), exported to ONNX by the
/// rembg project (MIT). Salient-object matte at 1024 x 1024.
pub const SUBJECT: ModelSpec = ModelSpec {
    id: "isnet-general-use",
    file: "isnet-general-use.onnx",
    url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
    sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a",
    bytes: 178_648_008,
    licence: "Apache-2.0",
};

/// The folder models are kept in, if the platform has one.
pub fn cache_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = env("PHOTOCRAFT_MODEL_DIR") {
        return Some(d);
    }
    #[cfg(target_os = "windows")]
    let base = env("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let base = env("HOME").map(|h| h.join("Library").join("Caches"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")));
    base.map(|b| b.join("photocraft").join("models"))
}

/// Where `spec` is (or would be) stored.
pub fn path_of(spec: &ModelSpec) -> Option<PathBuf> {
    cache_dir().map(|d| d.join(spec.file))
}

/// The model file if it is already downloaded (size checked; the hash was verified at download).
pub fn present(spec: &ModelSpec) -> Option<PathBuf> {
    let p = path_of(spec)?;
    (std::fs::metadata(&p).ok()?.len() == spec.bytes).then_some(p)
}

/// Is the model available without a download?
pub fn is_present(spec: &ModelSpec) -> bool {
    present(spec).is_some()
}

/// The model file, downloading it first if needed. `progress` gets `0.0..=1.0`; `keep_going`
/// returning false aborts with [`MlError::Cancelled`] (a partial file is removed).
#[cfg(not(target_arch = "wasm32"))]
pub fn ensure(spec: &ModelSpec, progress: &dyn Fn(f32), keep_going: &dyn Fn() -> bool) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    if let Some(p) = present(spec) {
        return Ok(p);
    }
    let dest = path_of(spec).ok_or_else(|| MlError::msg("this system has no cache folder for models (set PHOTOCRAFT_MODEL_DIR)"))?;
    let dir = dest.parent().ok_or_else(|| MlError::msg("invalid model folder"))?;
    std::fs::create_dir_all(dir).map_err(|e| MlError::msg(format!("cannot create {}: {e}", dir.display())))?;
    let part = dest.with_extension("part");
    let result = (|| {
        let mut file = std::fs::File::create(&part).map_err(|e| MlError::msg(format!("cannot write {}: {e}", part.display())))?;
        let mut hash = Sha256::new();
        let mut sink = |chunk: &[u8]| -> Result<()> {
            hash.update(chunk);
            file.write_all(chunk).map_err(|e| MlError::msg(format!("cannot write the model file: {e}")))
        };
        crate::http::download(spec.url, &mut sink, &|got, total| progress(got as f32 / total.unwrap_or(spec.bytes).max(1) as f32), keep_going)?;
        file.flush().map_err(|e| MlError::msg(format!("cannot write the model file: {e}")))?;
        let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if digest != spec.sha256 {
            return Err(MlError::msg(format!("the downloaded {} does not match its expected checksum; it was discarded", spec.file)));
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&part, &dest).map_err(|e| MlError::msg(format!("cannot save the model: {e}")))?;
            Ok(dest)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn ensure(spec: &ModelSpec, _: &dyn Fn(f32), _: &dyn Fn() -> bool) -> Result<PathBuf> {
    Err(MlError::msg(format!("{} cannot be downloaded in the web build", spec.file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_hash_is_a_sha256() {
        assert_eq!(SUBJECT.sha256.len(), 64);
        assert!(SUBJECT.sha256.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
