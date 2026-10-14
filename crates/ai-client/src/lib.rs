//! PhotoCraft's client for **Craft AI Manager** (CAIM): the engine's [`AiProvider`] spoken over
//! the loopback job protocol in `agents/SPEC.md` (v1.1 amendment 11).
//!
//! Native only - the web build has no local manager to talk to, and the engine's wasm path never
//! loads this crate. There is no HTTP dependency: HTTP/1.1 framing on [`std::net::TcpStream`],
//! `Content-Length` bodies only, `127.0.0.1` only, with the protocol's size caps and timeouts.
//! Nothing here trusts the reply, and nothing here panics: a manager that answers nonsense, or is
//! not running at all, produces an [`AiError`] the UI can show.
//!
//! [`AiProvider`]: photocraft_engine::ai::AiProvider
//! [`std::net::TcpStream`]: std::net::TcpStream
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod endpoint;
mod http;
mod pixels;
mod protocol;
mod provider;

pub use endpoint::{ENDPOINT_ENV, ENDPOINT_VERSION, Endpoint, default_endpoint_file, endpoint_file_from_env};
pub use provider::CaimProvider;

#[cfg(test)]
mod fake_server;
#[cfg(test)]
mod tests;
