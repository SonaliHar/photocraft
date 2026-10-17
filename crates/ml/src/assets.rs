//! Where the ONNX Runtime library and the model files live.
//!
//! Layout under the assets root:
//!
//! ```text
//! models/sam2/sam2_hiera_{tiny,small,base_plus,large}.{encoder,decoder}.onnx
//! runtime/ort-gpu/onnxruntime/capi/onnxruntime.dll      (or ORT_DYLIB_PATH)
//! runtime/cuda13/nvidia/cu13/bin/x86_64                 (optional: CUDA 13 for the GPU)
//! runtime/cuda13/nvidia/cudnn/bin                       (optional: cuDNN 9)
//! ```
//!
//! The root is `PHOTOCRAFT_ML_DIR`, else `<data dir>/PhotoCraft/ml`.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// A SAM 2 Hiera size, fastest first. Bigger is more accurate on thin or cluttered objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSize {
    Tiny,
    Small,
    BasePlus,
    Large,
}

impl ModelSize {
    /// The file stem the ONNX exports use.
    pub fn stem(self) -> &'static str {
        match self {
            Self::Tiny => "sam2_hiera_tiny",
            Self::Small => "sam2_hiera_small",
            Self::BasePlus => "sam2_hiera_base_plus",
            Self::Large => "sam2_hiera_large",
        }
    }

    /// Parses `tiny|small|base_plus|large` (the `PHOTOCRAFT_SAM2_SIZE` values).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "tiny" => Self::Tiny,
            "small" => Self::Small,
            "base_plus" | "base+" | "base" => Self::BasePlus,
            "large" => Self::Large,
            _ => return None,
        })
    }
}

/// Resolved paths for one model size.
#[derive(Clone, Debug)]
pub struct Assets {
    pub root: PathBuf,
    pub size: ModelSize,
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    /// The ONNX Runtime shared library.
    pub ort_lib: PathBuf,
    /// CUDA 13 and cuDNN 9 folders, when present (GPU use).
    pub cuda_dir: Option<PathBuf>,
    pub cudnn_dir: Option<PathBuf>,
}

fn default_root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PHOTOCRAFT_ML_DIR").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))
    };
    base.map(|b| b.join("PhotoCraft").join("ml"))
}

fn lib_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

fn existing(p: PathBuf) -> Option<PathBuf> {
    p.is_dir().then_some(p)
}

impl Assets {
    /// Finds the assets for `size`, or says what is missing.
    pub fn locate(size: ModelSize) -> Result<Self> {
        let root = default_root().ok_or_else(|| Error::MissingAssets("no data folder; set PHOTOCRAFT_ML_DIR".into()))?;
        Self::locate_in(root, size)
    }

    /// [`Assets::locate`] under an explicit `root`.
    pub fn locate_in(root: PathBuf, size: ModelSize) -> Result<Self> {
        let dir = root.join("models").join("sam2");
        let (encoder, decoder) = (dir.join(format!("{}.encoder.onnx", size.stem())), dir.join(format!("{}.decoder.onnx", size.stem())));
        for f in [&encoder, &decoder] {
            if !f.is_file() {
                return Err(Error::MissingAssets(format!("{} (download the SAM 2 ONNX models, see docs/ml.md)", f.display())));
            }
        }
        let ort_lib = std::env::var_os("ORT_DYLIB_PATH")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .into_iter()
            .chain([root.join("runtime/ort-gpu/onnxruntime/capi").join(lib_name()), root.join("runtime/ort").join(lib_name())])
            .find(|p| p.is_file())
            .ok_or_else(|| Error::MissingAssets(format!("{} under {}/runtime (or set ORT_DYLIB_PATH)", lib_name(), root.display())))?;
        let cuda_dir = existing(root.join("runtime/cuda13/nvidia/cu13/bin/x86_64")).or_else(|| existing(root.join("runtime/cuda13/nvidia/cu13/bin")));
        let cudnn_dir = existing(root.join("runtime/cuda13/nvidia/cudnn/bin"));
        Ok(Self { root, size, encoder, decoder, ort_lib, cuda_dir, cudnn_dir })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_names_round_trip() {
        for s in [ModelSize::Tiny, ModelSize::Small, ModelSize::BasePlus, ModelSize::Large] {
            assert!(s.stem().starts_with("sam2_hiera_"));
        }
        assert_eq!(ModelSize::parse(" Base_Plus "), Some(ModelSize::BasePlus));
        assert_eq!(ModelSize::parse("huge"), None);
    }

    #[test]
    fn missing_models_are_an_error_not_a_panic() {
        let dir = std::env::temp_dir().join("photocraft-ml-test-missing");
        assert!(matches!(Assets::locate_in(dir, ModelSize::Small), Err(Error::MissingAssets(_))));
    }
}
