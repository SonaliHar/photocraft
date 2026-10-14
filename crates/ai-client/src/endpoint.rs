//! Where Craft AI Manager is, and the token it asked for (SPEC > Job protocol v1 > Discovery).

use std::fs::File;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};

use photocraft_engine::ai::AiError;
use serde::Deserialize;

/// The `version` an endpoint file must declare.
pub const ENDPOINT_VERSION: u64 = 1;
/// Environment variable naming the endpoint file instead of the platform default (amendment 11).
pub const ENDPOINT_ENV: &str = "CRAFT_AI_ENDPOINT";
/// The endpoint file is JSON, and the protocol caps JSON at 64 KiB.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// A token is a header value: bounded, printable ASCII, and never carrying CR or LF.
const MAX_TOKEN_BYTES: usize = 512;

/// The manager's loopback address plus the token it wrote for this start.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub addr: SocketAddr,
    pub token: String,
}

impl std::fmt::Debug for Endpoint {
    /// The token is a credential, so it never appears in a log line or a debug print.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint").field("addr", &self.addr).field("token", &"[redacted]").finish()
    }
}

impl Endpoint {
    /// A loopback endpoint, for tests and for callers that already hold a port and token.
    pub fn loopback(port: u16, token: impl Into<String>) -> Self {
        Self { addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)), token: token.into() }
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The `Host` header the protocol asks for.
    pub fn host_header(&self) -> String {
        format!("127.0.0.1:{}", self.addr.port())
    }
}

#[derive(Deserialize)]
struct EndpointFile {
    version: u64,
    port: u32,
    token: String,
    /// Optional in v1; the client only ever connects to loopback, so any other value is refused.
    #[serde(default)]
    host: Option<String>,
}

/// Read and check the endpoint file at `path`.
///
/// An unusable file is `AiError::NoProvider`: the answer the user needs is the same one as for a
/// manager that never started, and a half-written file must not look like a working connection.
pub fn load(path: &Path) -> Result<Endpoint, AiError> {
    let text = read_file(path)?;
    let Ok(parsed) = serde_json::from_str::<EndpointFile>(&text) else {
        return Err(AiError::NoProvider);
    };
    if parsed.version != ENDPOINT_VERSION {
        return Err(AiError::NoProvider);
    }
    if let Some(host) = parsed.host.as_deref().map(str::trim)
        && host != "127.0.0.1"
    {
        return Err(AiError::Other(
            "Craft AI Manager's endpoint file names a host other than 127.0.0.1; PhotoCraft only talks to the manager over loopback.".into(),
        ));
    }
    let Ok(port) = u16::try_from(parsed.port) else {
        return Err(AiError::NoProvider);
    };
    if port == 0 || !is_usable_token(&parsed.token) {
        return Err(AiError::NoProvider);
    }
    Ok(Endpoint::loopback(port, parsed.token))
}

fn read_file(path: &Path) -> Result<String, AiError> {
    let mut file = File::open(path).map_err(|_| AiError::NoProvider)?;
    let Ok(meta) = file.metadata() else {
        return Err(AiError::NoProvider);
    };
    if meta.len() > MAX_FILE_BYTES {
        return Err(AiError::Other("The Craft AI Manager endpoint file is larger than 64 KiB.".into()));
    }
    let mut bytes = Vec::new();
    // Read past the cap so a file whose metadata lied is still caught before it is parsed.
    if file.by_ref().take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).is_err() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(AiError::Other("The Craft AI Manager endpoint file is larger than 64 KiB.".into()));
    }
    String::from_utf8(bytes).map_err(|_| AiError::NoProvider)
}

/// A token that can go into a header: non-empty, bounded, and printable ASCII (no CR, no LF).
fn is_usable_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN_BYTES && token.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// The file the manager writes on this platform (amendment 11). `None` when the platform has no
/// place for it, which the caller reports as "no manager".
pub fn default_endpoint_file() -> Option<PathBuf> {
    platform_data_dir().map(|dir| dir.join("CraftAI").join("endpoint.json"))
}

fn platform_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()).map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").filter(|value| !value.is_empty()).map(|home| PathBuf::from(home).join("Library").join("Application Support"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var_os("HOME").filter(|value| !value.is_empty()).map(|home| PathBuf::from(home).join(".local").join("share")))
            .map(PathBuf::from)
    }
}

/// Which endpoint file to read: `CRAFT_AI_ENDPOINT` when it names one, else the platform default.
pub fn endpoint_file_from_env() -> Option<PathBuf> {
    match std::env::var_os(ENDPOINT_ENV) {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => default_endpoint_file(),
    }
}
