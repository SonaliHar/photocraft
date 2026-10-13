//! # photocraft-ml
//!
//! The two places PhotoCraft talks to machine learning:
//!
//! * [`subject`]: Select Subject / Remove Background with a local ONNX salient-object model
//!   (IS-Net general use, Apache-2.0), run on the CPU by the pure-Rust `tract` runtime. The model
//!   is not part of the repository: [`models`] downloads it once into the user's cache folder and
//!   checks its SHA-256.
//! * [`genfill`]: Generative Fill through a provider the user chooses (Cloudflare Workers AI,
//!   a local AUTOMATIC1111/Forge server, Google Gemini, OpenAI or Stability AI), with the user's
//!   own API key.
//!
//! This crate knows nothing about documents: it takes and returns plain pixel buffers and encoded
//! image bytes. The engine does the document work.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod genfill;
mod http;
pub mod models;
pub mod subject;

/// What can go wrong; the message is meant to be shown to the user as is.
#[derive(Debug, thiserror::Error)]
pub enum MlError {
    #[error("{0}")]
    Msg(String),
    #[error("cancelled")]
    Cancelled,
}

impl MlError {
    pub fn msg(m: impl Into<String>) -> Self {
        Self::Msg(m.into())
    }
}

pub type Result<T> = std::result::Result<T, MlError>;
