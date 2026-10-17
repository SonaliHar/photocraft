//! Learned-model inference: SAM 2 (Segment Anything 2) promptable segmentation through ONNX
//! Runtime, behind Object Selection.
//!
//! Nothing here is required to run PhotoCraft. ONNX Runtime and the model files are found at run
//! time ([`Assets::locate`]); when they are missing, [`Sam2::load`] returns an error and callers
//! fall back to the classical algorithms in `photocraft-algo`. On wasm the loader always reports
//! [`Error::Unsupported`].
//!
//! SAM 2 runs in two steps. The *encoder* turns a 1024×1024 image into an [`Embedding`] (about
//! 0.1–0.2 s on a GPU); keep it per document pixels. The *decoder* turns an embedding plus prompts
//! (points, a box) into mask logits in about 6 ms, so a hover preview can run on every pointer
//! stop.

mod assets;
mod enhance;
mod remote;
mod sam2;

pub use assets::{Assets, ModelSize};
pub use enhance::{EnhancerConfig, MAX_PROMPT_CHARS};
pub use remote::{DecomposeMode, DecomposeParams, DecomposedLayer, Decomposition, InpaintParams, Inpainting, JobState, JobStatus, RemoteClient, RemoteConfig, RemoteMask, SegmentPrompt, ServerInfo, method_is_valid};
pub use sam2::{Backend, Embedding, Prompt, Sam2, SamMask};

/// Side of the square image the encoder takes.
pub const INPUT_SIZE: usize = 1024;
/// Side of the decoder's mask logits.
pub const MASK_SIZE: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("machine learning is not available in this build")]
    Unsupported,
    #[error("ML assets not found: {0}")]
    MissingAssets(String),
    #[error("ONNX Runtime: {0}")]
    Runtime(String),
    #[error("bad input: {0}")]
    BadInput(String),
    #[error("remote service: {0}")]
    Remote(String),
}

pub type Result<T> = std::result::Result<T, Error>;
