//! Inpainting services behind [`crate::ImageProvider`], one module each.

pub mod bfl;
pub mod bria;
pub mod common;
pub mod flux_fill;
pub mod ideogram;
pub mod openai;
pub mod stability;
