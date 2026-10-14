//! The HTTP/1.1 framing the protocol allows: `Content-Length` bodies, never chunked, one request
//! per connection (`Connection: close`) - the mirror of `apps/photocraft/src/control_server.rs`.
//!
//! Nothing here trusts the reply: an over-long line, a missing or dishonest `Content-Length`, a
//! chunked transfer, a non-UTF-8 header or a body that stops short is an error, never a panic.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use photocraft_engine::ai::AiError;

use crate::endpoint::Endpoint;

/// SPEC amendment 11: connect 2 s, read 30 s per request.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// SPEC amendment 11: JSON bodies at most 64 KiB (PNG caps come from the request's pixel count).
pub const MAX_JSON_BODY_BYTES: usize = 64 * 1024;
const MAX_STATUS_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// A whole reply: the status, the two headers the client looks at, and a body within the cap.
pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// One request, one reply, then the connection is finished. `max_body_bytes` caps the reply and is
/// chosen per call: 64 KiB for JSON, and what the requested PNG can be for `result.png`.
pub fn request(endpoint: &Endpoint, method: &str, path: &str, content_type: Option<&str>, body: &[u8], max_body_bytes: usize) -> Result<Reply, AiError> {
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, CONNECT_TIMEOUT).map_err(|_| AiError::NoProvider)?;
    if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err() || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return Err(AiError::Other("PhotoCraft could not put a timeout on the Craft AI Manager connection.".into()));
    }
    write_request(&mut stream, endpoint, method, path, content_type, body)?;
    read_reply(stream, max_body_bytes)
}

fn write_request(stream: &mut TcpStream, endpoint: &Endpoint, method: &str, path: &str, content_type: Option<&str>, body: &[u8]) -> Result<(), AiError> {
    let mut out = Vec::with_capacity(body.len().saturating_add(256));
    out.extend_from_slice(method.as_bytes());
    out.extend_from_slice(b" ");
    out.extend_from_slice(path.as_bytes());
    out.extend_from_slice(b" HTTP/1.1\r\n");
    out.extend_from_slice(b"Host: ");
    out.extend_from_slice(endpoint.host_header().as_bytes());
    out.extend_from_slice(b"\r\nAuthorization: Bearer ");
    out.extend_from_slice(endpoint.token.as_bytes());
    out.extend_from_slice(b"\r\nAccept: application/json, image/png\r\nConnection: close\r\n");
    if let Some(content_type) = content_type {
        out.extend_from_slice(b"Content-Type: ");
        out.extend_from_slice(content_type.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    out.extend_from_slice(body);
    stream.write_all(&out).map_err(|error| io_error(error, true))?;
    stream.flush().map_err(|error| io_error(error, true))
}

fn read_reply(stream: TcpStream, max_body_bytes: usize) -> Result<Reply, AiError> {
    let mut reader = BufReader::new(stream);
    let status_line =
        next_line(&mut reader, MAX_STATUS_LINE_BYTES)?.ok_or_else(|| AiError::Other("Craft AI Manager closed the connection without answering.".into()))?;
    let status = parse_status_line(&status_line)?;
    let mut content_length: Option<u64> = None;
    let mut content_type: Option<String> = None;
    let mut header_bytes = status_line.len();
    loop {
        let Some(line) = next_line(&mut reader, MAX_HEADER_LINE_BYTES)? else {
            return Err(AiError::Other("Craft AI Manager stopped in the middle of its headers.".into()));
        };
        if line.is_empty() {
            break;
        }
        header_bytes = header_bytes.saturating_add(line.len()).saturating_add(2);
        if header_bytes > MAX_HEADER_BYTES {
            return Err(AiError::Other("Craft AI Manager sent more headers than the protocol allows.".into()));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(AiError::Other("Craft AI Manager sent a header line without a name.".into()));
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(AiError::Other("Craft AI Manager sent a header with an unusable name.".into()));
        }
        let value = value.trim().to_string();
        match name.as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(AiError::Other("Craft AI Manager sent two Content-Length headers.".into()));
                }
                let Ok(length) = value.parse::<u64>() else {
                    return Err(AiError::Other("Craft AI Manager sent a Content-Length that is not a number.".into()));
                };
                content_length = Some(length);
            }
            "transfer-encoding" => return Err(AiError::Other("Craft AI Manager answered with a chunked body, which the protocol does not allow.".into())),
            "content-type" => content_type = Some(value),
            _ => {}
        }
    }
    let length = match content_length {
        Some(length) => length,
        // 204 and 304 carry no body by definition, and the protocol's DELETE answer is 204.
        None if matches!(status, 204 | 304) => 0,
        None => return Err(AiError::Other("Craft AI Manager answered without a Content-Length.".into())),
    };
    let Ok(size) = usize::try_from(length) else {
        return Err(AiError::Other("Craft AI Manager announced a body larger than this machine can hold.".into()));
    };
    if size > max_body_bytes {
        return Err(AiError::Other(format!("Craft AI Manager sent a body over the {max_body_bytes} byte limit for this call.")));
    }
    let mut body = vec![0u8; size];
    reader.read_exact(&mut body).map_err(|error| io_error(error, false))?;
    Ok(Reply { status, content_type, body })
}

/// One CRLF-terminated line, capped. `Ok(None)` is a clean end of stream before any byte arrived.
fn next_line(reader: &mut BufReader<TcpStream>, max_bytes: usize) -> Result<Option<String>, AiError> {
    let mut bytes = Vec::new();
    // Read past the cap so an over-long line is caught without allocating for it first.
    let mut limited = (&mut *reader).take((max_bytes + 3) as u64);
    let read = limited.read_until(b'\n', &mut bytes).map_err(|error| io_error(error, false))?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > max_bytes + 2 {
        return Err(AiError::Other("Craft AI Manager sent a line longer than the protocol allows.".into()));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes).map(Some).map_err(|_| AiError::Other("Craft AI Manager sent a header that is not UTF-8.".into()))
}

fn parse_status_line(line: &str) -> Result<u16, AiError> {
    let malformed = || AiError::Other("Craft AI Manager sent a malformed status line.".into());
    let rest = line.strip_prefix("HTTP/1.1 ").or_else(|| line.strip_prefix("HTTP/1.0 ")).ok_or_else(malformed)?;
    let code = rest.split(' ').next().unwrap_or_default();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    code.parse::<u16>().map_err(|_| malformed())
}

fn io_error(error: std::io::Error, writing: bool) -> AiError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            AiError::Other(format!("Craft AI Manager did not {} in time.", if writing { "read the request" } else { "answer" }))
        }
        _ => AiError::Other("PhotoCraft lost the connection to Craft AI Manager.".into()),
    }
}
