//! PC-03 tests: the client against an in-process fake Craft AI Manager (SPEC amendment 11).
//! Every test uses its own endpoint file, so they run in parallel without touching the environment.

use std::io::Cursor;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_engine::ai::{AiError, AiJobId, AiJobState, AiProvider, AiRequest, AiResult, AiTask};
use photocraft_geom::Rect;

use crate::fake_server::{FakeServer, response};
use crate::pixels;
use crate::provider::CaimProvider;

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// A temp endpoint file that deletes itself, so no test leaves state behind.
struct TempEndpoint {
    path: PathBuf,
}

impl TempEndpoint {
    fn new(text: &str) -> Self {
        let endpoint = Self { path: unique_path() };
        endpoint.set(text);
        endpoint
    }

    fn missing() -> Self {
        Self { path: unique_path() }
    }

    fn set(&self, text: &str) {
        std::fs::write(&self.path, text).expect("the temp endpoint file should be writable");
    }

    fn provider(&self) -> CaimProvider {
        CaimProvider::with_endpoint_file(self.path.clone())
    }
}

impl Drop for TempEndpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn unique_path() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("photocraft-caim-test-{}-{n}.json", std::process::id()))
}

fn endpoint_json(port: u16, token: &str) -> String {
    format!(r#"{{"version": 1, "port": {port}, "token": "{token}"}}"#)
}

/// A port nothing is listening on: bind one, then let go of it.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the test should be able to bind a loopback port");
    listener.local_addr().expect("a bound listener has an address").port()
}

const RGB8: PixelFormat = PixelFormat::new(ColorMode::Rgb, SampleType::U8, false);
const RGB16: PixelFormat = PixelFormat::new(ColorMode::Rgb, SampleType::U16, false);
const RGB32F: PixelFormat = PixelFormat::new(ColorMode::Rgb, SampleType::F32, false);
const RGBA8: PixelFormat = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);

/// A deterministic window: every sample a different value in 0..=1, so a round trip is visible.
fn request(width: u32, height: u32, format: PixelFormat) -> AiRequest {
    let samples = usize::try_from(width).expect("test width") * usize::try_from(height).expect("test height") * format.channels();
    let pixels: Vec<f32> = (0..samples).map(|index| ((index * 7) % 251) as f32 / 250.0).collect();
    let coverage: Vec<f32> =
        (0..(usize::try_from(width).expect("test width") * usize::try_from(height).expect("test height"))).map(|index| (index % 101) as f32 / 100.0).collect();
    AiRequest {
        task: AiTask::FillMaskedArea,
        window: Rect::new(10, 20, 10 + width as i32, 20 + height as i32),
        pixels,
        format,
        coverage,
        prompt: None,
        client_version: "photocraft/0.0.0-test".into(),
    }
}

const DONE: &str = r#"{"state": "done", "progress": 1.0, "message": ""}"#;
const PROVENANCE: &str = r#"{"model": "lama-onnx", "device": "cpu", "provider": "local", "elapsed_ms": 1234, "image_size": [4, 3]}"#;

/// Submit `req` and fetch its result, where the manager answers with `png` as `result.png`.
fn submit_and_fetch(server: &FakeServer, endpoint: &TempEndpoint, req: &AiRequest, png: &[u8], content_type: &str) -> Result<AiResult, AiError> {
    server.queue_json(202, r#"{"id": "job-1"}"#);
    let provider = endpoint.provider();
    let id = provider.submit(req).expect("the job should be accepted");
    server.queue_json(200, PROVENANCE);
    server.queue(response(200, content_type, png));
    provider.result(&id)
}

fn header_value(content_type: Option<&str>) -> Option<String> {
    content_type.and_then(|value| value.split("boundary=").nth(1)).map(str::to_string)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// The named parts of a captured `multipart/form-data` body, in order.
fn multipart_parts(body: &[u8], boundary: &str) -> Vec<(String, Vec<u8>)> {
    let marker = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut at = 0;
    while let Some(offset) = find_bytes(&body[at..], marker.as_bytes()) {
        let start = at + offset + marker.len();
        let end = find_bytes(&body[start..], marker.as_bytes()).unwrap_or(body.len() - start) + start;
        let chunk = &body[start..end];
        if let Some(head_end) = find_bytes(chunk, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&chunk[..head_end]).to_string();
            let name = head.split("name=\"").nth(1).and_then(|rest| rest.split('"').next()).unwrap_or_default().to_string();
            let mut data = &chunk[head_end + 4..];
            if data.ends_with(b"\r\n") {
                data = &data[..data.len() - 2];
            }
            parts.push((name, data.to_vec()));
        }
        at = end;
    }
    parts
}

fn png_header(png: &[u8]) -> (u32, u32, png::ColorType, png::BitDepth) {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    let info = decoder.read_header_info().expect("the part should be a PNG");
    (info.width, info.height, info.color_type, info.bit_depth)
}

#[test]
fn happy_path_round_trips_8_bit_pixels() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-8"));
    let provider = endpoint.provider();
    let req = request(4, 3, RGB8);

    server.queue_json(202, r#"{"id": "job-8"}"#);
    let id = provider.submit(&req).expect("the job should be accepted");
    assert_eq!(id.as_str(), "job-8");

    server.queue_json(200, r#"{"state": "running", "progress": 0.5, "message": "inpainting"}"#);
    let state = provider.poll(&id).expect("the job should be pollable");
    assert!(matches!(&state, AiJobState::Running { progress, message } if (progress - 0.5).abs() < 1e-6 && message == "inpainting"));

    server.queue_json(200, DONE);
    assert_eq!(provider.poll(&id).expect("the job should report done"), AiJobState::Done);

    let png = pixels::encode_image(&req.pixels, req.format, 4, 3).expect("the client should be able to encode its own window");
    server.queue_json(200, PROVENANCE);
    server.queue(response(200, "image/png", &png));
    let result = provider.result(&id).expect("the result should be readable");

    assert_eq!(result.window, req.window);
    assert_eq!(result.format, req.format);
    assert_eq!(result.pixels.len(), req.pixels.len());
    for (got, want) in result.pixels.iter().zip(&req.pixels) {
        assert!((got - want).abs() <= 1.0 / 255.0, "{got} is not within 1/255 of {want}");
    }
    assert_eq!(result.model, "lama-onnx");
    assert_eq!(result.device, "cpu");
    assert_eq!(result.provider, "local");
    assert_eq!(result.elapsed_ms, 1234);
    // Health is not required before a job: the first request is the job itself.
    assert_eq!(
        server.captured().iter().map(|call| (call.method.as_str(), call.path.as_str())).collect::<Vec<_>>(),
        vec![
            ("POST", "/v1/jobs"),
            ("GET", "/v1/jobs/job-8"),
            ("GET", "/v1/jobs/job-8"),
            ("GET", "/v1/jobs/job-8/result"),
            ("GET", "/v1/jobs/job-8/result.png")
        ]
    );
}

#[test]
fn happy_path_round_trips_16_bit_pixels() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-16"));
    let provider = endpoint.provider();
    let req = request(3, 5, RGB16);

    server.queue_json(202, r#"{"id": "job-16"}"#);
    let id = provider.submit(&req).expect("the job should be accepted");
    let png = pixels::encode_image(&req.pixels, req.format, 3, 5).expect("the client should be able to encode its own window");
    server.queue_json(200, r#"{"state": "done"}"#);
    assert_eq!(provider.poll(&id).expect("the job should report done"), AiJobState::Done);
    server.queue_json(200, r#"{"model": "m", "device": "cuda", "provider": "local", "elapsed_ms": 7, "image_size": [3, 5]}"#);
    server.queue(response(200, "image/png", &png));
    let result = provider.result(&id).expect("the result should be readable");
    for (got, want) in result.pixels.iter().zip(&req.pixels) {
        assert!((got - want).abs() <= 1.0 / 65535.0, "{got} is not within 1/65535 of {want}");
    }
}

#[test]
fn a_32_bit_document_sends_16_bit_png() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-32"));
    let req = request(2, 2, RGB32F);
    server.queue_json(202, r#"{"id": "job-32"}"#);
    endpoint.provider().submit(&req).expect("the job should be accepted");
    let captured = server.captured();
    let post = &captured[0];
    let boundary = header_value(post.header("content-type")).expect("the upload should name a boundary");
    let parts = multipart_parts(&post.body, &boundary);
    assert_eq!(parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), vec!["job", "image", "mask"]);
    let job: serde_json::Value = serde_json::from_slice(&parts[0].1).expect("the job part should be JSON");
    assert_eq!(job["bit_depth"], 16);
    assert_eq!(png_header(&parts[1].1), (2, 2, png::ColorType::Rgb, png::BitDepth::Sixteen));
}
#[test]
fn the_multipart_upload_has_the_three_parts_in_order() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-parts"));
    let req = request(4, 3, RGBA8);
    server.queue_json(202, r#"{"id": "job-parts"}"#);
    endpoint.provider().submit(&req).expect("the job should be accepted");

    let captured = server.captured();
    let post = &captured[0];
    assert_eq!((post.method.as_str(), post.path.as_str()), ("POST", "/v1/jobs"));
    let content_type = post.header("content-type").expect("the upload should have a content type");
    assert!(content_type.starts_with("multipart/form-data; boundary="), "{content_type}");
    let boundary = header_value(Some(content_type)).expect("the upload should name a boundary");
    let parts = multipart_parts(&post.body, &boundary);
    assert_eq!(parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), vec!["job", "image", "mask"]);

    let job: serde_json::Value = serde_json::from_slice(&parts[0].1).expect("the job part should be JSON");
    let mut keys: Vec<&str> = job.as_object().expect("the job part should be an object").keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["bit_depth", "channels", "client", "image_size", "prompt", "provider", "region", "settings", "task"]);
    // The wire order is the protocol's order, so a manager can read the parts as they arrive.
    let job_text = String::from_utf8_lossy(&parts[0].1);
    let mut seen = 0usize;
    for key in ["task", "image_size", "region", "bit_depth", "channels", "prompt", "settings", "provider", "client"] {
        let found = job_text.find(&format!("\"{key}\":")).expect("the job part should name every protocol key");
        assert!(found > seen, "the job part should name its keys in protocol order: {job_text}");
        seen = found;
    }
    assert_eq!(job["task"], "fill_masked_area");
    assert_eq!(job["image_size"], serde_json::json!([4, 3]));
    assert_eq!(job["region"], serde_json::json!([10, 20, 4, 3]));
    assert_eq!(job["bit_depth"], 8);
    assert_eq!(job["channels"], 4);
    assert!(job["prompt"].is_null());
    assert_eq!(job["settings"], serde_json::json!({}));
    assert_eq!(job["provider"], "auto");
    assert_eq!(job["client"], "photocraft/0.0.0-test");

    assert_eq!(png_header(&parts[1].1), (4, 3, png::ColorType::Rgba, png::BitDepth::Eight));
    assert_eq!(png_header(&parts[2].1), (4, 3, png::ColorType::Grayscale, png::BitDepth::Eight));
    let mut mask = png::Decoder::new(Cursor::new(&parts[2].1)).read_info().expect("the mask should be a PNG");
    let mut mask_pixels = vec![0u8; mask.output_buffer_size().expect("the mask should have a size")];
    mask.next_frame(&mut mask_pixels).expect("the mask should decode");
    for (value, want) in mask_pixels.iter().zip(&req.coverage) {
        assert!((f32::from(*value) / 255.0 - *want).abs() <= 1.0 / 255.0, "{value} is not coverage {want}");
    }
}
#[test]
fn every_request_carries_the_token_from_the_file() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-secret"));
    let provider = endpoint.provider();
    let req = request(2, 2, RGB8);
    server.queue_json(202, r#"{"id": "job-auth"}"#);
    let id = provider.submit(&req).expect("the job should be accepted");
    server.queue_json(200, DONE);
    provider.poll(&id).expect("the job should be pollable");
    server.queue(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec());
    provider.cancel(&id).expect("the job should be cancellable");

    let captured = server.captured();
    assert_eq!(captured.len(), 3);
    for call in &captured {
        assert_eq!(call.header("authorization"), Some("Bearer tok-secret"), "{} {} lost the token", call.method, call.path);
        assert_eq!(call.header("host"), Some(&format!("127.0.0.1:{}", server.port())[..]));
        assert_eq!(call.header("connection"), Some("close"));
    }
}

#[test]
fn an_unusable_endpoint_file_means_no_manager() {
    let server = FakeServer::start();
    let port = server.port();
    let req = request(2, 2, RGB8);
    let cases: [(&str, TempEndpoint); 8] = [
        ("a missing file", TempEndpoint::missing()),
        ("an empty file", TempEndpoint::new("")),
        ("a file that is not JSON", TempEndpoint::new("{")),
        ("a protocol version PhotoCraft does not know", TempEndpoint::new(r#"{"version": 2, "port": 1, "token": "t"}"#)),
        ("port 0", TempEndpoint::new(&endpoint_json(0, "t"))),
        ("a port that is not a port", TempEndpoint::new(r#"{"version": 1, "port": 70000, "token": "t"}"#)),
        ("an empty token", TempEndpoint::new(&endpoint_json(port, ""))),
        (r#"a token carrying a newline"#, TempEndpoint::new(&format!(r#"{{"version": 1, "port": {port}, "token": "a\nb"}}"#))),
    ];
    for (what, endpoint) in cases {
        let error = endpoint.provider().submit(&req).expect_err(&format!("{what} must not start a job"));
        assert_eq!(error, AiError::NoProvider, "{what}");
    }
    assert_eq!(server.capture_count(), 0, "a bad endpoint file must not get a request");
}

#[test]
fn an_over_large_endpoint_file_is_reported() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&format!(r#"{{"version": 1, "port": {}, "token": "{}"}}"#, server.port(), "t".repeat(70 * 1024)));
    let error = endpoint.provider().submit(&request(2, 2, RGB8)).expect_err("an over-large endpoint file is not a manager");
    assert!(!matches!(error, AiError::NoProvider), "the real reason should be reported: {error}");
    assert_eq!(server.capture_count(), 0);
}

#[test]
fn a_closed_port_is_no_manager() {
    let endpoint = TempEndpoint::new(&endpoint_json(closed_port(), "tok-gone"));
    let error = endpoint.provider().submit(&request(2, 2, RGB8)).expect_err("nothing is listening");
    assert_eq!(error, AiError::NoProvider);
    assert_eq!(error.to_string(), "Craft AI Manager isn't running");
}

#[test]
fn a_wrong_token_says_to_restart_the_manager() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-wrong"));
    server.queue_json(401, r#"{"error": "that token is not mine"}"#);
    let error = endpoint.provider().submit(&request(2, 2, RGB8)).expect_err("401 should fail the job");
    assert_eq!(error.to_string(), "Craft AI Manager refused the request (token mismatch); restart Craft AI Manager");
}

#[test]
fn other_errors_use_the_manager_s_sentence() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-err"));
    let provider = endpoint.provider();
    server.queue_json(202, r#"{"id": "job-err"}"#);
    let id = provider.submit(&request(2, 2, RGB8)).expect("the job should be accepted");
    for (status, sentence) in [(404u16, "No job by that id."), (409, "The result is not ready yet."), (500, "The model crashed.")] {
        server.queue_json(status, &format!(r#"{{"error": "{sentence}"}}"#));
        assert_eq!(provider.poll(&id).expect_err("the manager said no").to_string(), sentence, "HTTP {status}");
    }
    server.queue(response(400, "text/plain", b"nope"));
    assert_eq!(provider.poll(&id).expect_err("a body that is not JSON still needs a message").to_string(), "Craft AI Manager answered HTTP 400.");
}

#[test]
fn a_malformed_reply_is_an_error_and_never_a_panic() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-bad"));
    let mut non_utf8 = b"HTTP/1.1 200 OK\r\nX-Strange: ".to_vec();
    non_utf8.extend_from_slice(&[0xff, 0xfe, 0x80]);
    non_utf8.extend_from_slice(b"\r\nContent-Length: 2\r\n\r\n{}");
    let cases: [(&str, Vec<u8>); 10] = [
        ("no status line", Vec::new()),
        ("a status line that is not HTTP", b"HTTP/2.0 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
        ("a status that is not a number", b"HTTP/1.1 abc OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
        ("no Content-Length", b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n".to_vec()),
        ("a chunked body", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
        ("two Content-Length headers", b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n{}".to_vec()),
        ("a Content-Length that is not a number", b"HTTP/1.1 200 OK\r\nContent-Length: two\r\n\r\n{}".to_vec()),
        ("a body over the cap", b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999\r\n\r\n{}".to_vec()),
        ("a body shorter than it promised", b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{}".to_vec()),
        ("a header that is not UTF-8", non_utf8),
    ];
    for (what, reply) in cases {
        server.queue(reply);
        let error = endpoint.provider().submit(&request(2, 2, RGB8)).expect_err(&format!("{what} must not be accepted"));
        assert!(!matches!(error, AiError::NoProvider), "{what} should be reported, not blamed on the manager being away");
    }
}

#[test]
fn the_result_png_must_match_the_window_that_was_sent() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-png"));
    let req = request(4, 3, RGB8);
    let good = pixels::encode_image(&req.pixels, req.format, 4, 3).expect("the client should encode its own window");

    let channels = pixels::channels(req.format).expect("an RGB window has a channel count");
    let wide: Vec<f32> = req.pixels.iter().cycle().take(5 * 3 * channels).copied().collect();
    let wrong_size = pixels::encode_image(&wide, req.format, 5, 3).expect("a 5x3 PNG is a valid PNG");
    let error = submit_and_fetch(&server, &endpoint, &req, &wrong_size, "image/png").expect_err("a result of the wrong size must not be used");
    assert!(error.to_string().contains("size"), "{error}");

    let deep16 = pixels::encode_image(&req.pixels, RGB16, 4, 3).expect("a 16-bit PNG is a valid PNG");
    let error = submit_and_fetch(&server, &endpoint, &req, &deep16, "image/png").expect_err("a result of the wrong depth must not be used");
    assert!(error.to_string().contains("bit depth"), "{error}");

    let gray = pixels::encode_mask(&req.coverage, 4, 3).expect("a grayscale PNG is a valid PNG");
    submit_and_fetch(&server, &endpoint, &req, &gray, "image/png").expect_err("a grayscale result must not be used");

    submit_and_fetch(&server, &endpoint, &req, &good, "application/octet-stream").expect_err("the result has to be labelled a PNG");
    submit_and_fetch(&server, &endpoint, &req, b"not a png at all", "image/png").expect_err("a body that is not a PNG must not be used");

    let huge = vec![7u8; 2 * 1024 * 1024];
    let error = submit_and_fetch(&server, &endpoint, &req, &huge, "image/png").expect_err("a 2 MiB result for a 12-pixel window is refused");
    assert!(error.to_string().contains("limit"), "{error}");

    // And the good one still works, so the checks above are not just failing everything.
    let result = submit_and_fetch(&server, &endpoint, &req, &good, "image/png").expect("the result should be readable");
    assert_eq!(result.pixels.len(), req.pixels.len());
}

#[test]
fn cancel_goes_to_the_manager_that_took_the_job() {
    let first = FakeServer::start();
    let second = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(first.port(), "tok-first"));
    let provider = endpoint.provider();
    first.queue_json(202, r#"{"id": "job-stay"}"#);
    let id = provider.submit(&request(2, 2, RGB8)).expect("the job should be accepted");
    // The manager restarts: a new port, a new token, a new file.
    endpoint.set(&endpoint_json(second.port(), "tok-second"));
    first.queue(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec());
    provider.cancel(&id).expect("the job should be cancelled where it was started");
    let captured = first.captured();
    let paths: Vec<(&str, &str)> = captured.iter().map(|call| (call.method.as_str(), call.path.as_str())).collect();
    assert!(paths.contains(&("DELETE", "/v1/jobs/job-stay")), "{paths:?}");
    assert_eq!(second.capture_count(), 0, "the new manager must not be told about the old job");
}

#[test]
fn the_endpoint_file_is_read_again_on_every_submit() {
    let first = FakeServer::start();
    let second = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(first.port(), "tok-one"));
    let provider = endpoint.provider();
    let req = request(2, 2, RGB8);
    first.queue_json(202, r#"{"id": "job-one"}"#);
    provider.submit(&req).expect("the first job should be accepted");
    endpoint.set(&endpoint_json(second.port(), "tok-two"));
    second.queue_json(202, r#"{"id": "job-two"}"#);
    provider.submit(&req).expect("the second job should reach the new manager");
    assert_eq!(first.captured()[0].header("authorization"), Some("Bearer tok-one"));
    assert_eq!(second.captured()[0].header("authorization"), Some("Bearer tok-two"));
    assert_eq!(second.capture_count(), 1);
}
#[test]
fn an_endpoint_file_naming_another_host_is_refused() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&format!(r#"{{"version": 1, "host": "127.0.0.2", "port": {}, "token": "t"}}"#, server.port()));
    let error = endpoint.provider().submit(&request(2, 2, RGB8)).expect_err("PhotoCraft must not leave loopback");
    assert!(error.to_string().contains("127.0.0.1"), "{error}");
    assert_eq!(server.capture_count(), 0, "a non-loopback host must not get a request");
    let ok = TempEndpoint::new(&format!(r#"{{"version": 1, "host": "127.0.0.1", "port": {}, "token": "t"}}"#, server.port()));
    server.queue_json(202, r#"{"id": "job-loop"}"#);
    ok.provider().submit(&request(2, 2, RGB8)).expect("loopback is the address the protocol allows");
}

#[test]
fn non_rgb_documents_are_refused_before_any_request() {
    let server = FakeServer::start();
    let provider = TempEndpoint::new(&endpoint_json(server.port(), "tok-mode")).provider();
    for format in [
        PixelFormat::GRAY8,
        PixelFormat::GRAYA8,
        PixelFormat::CMYKA8,
        PixelFormat::new(ColorMode::Lab, SampleType::F32, true),
        PixelFormat::new(ColorMode::Indexed, SampleType::U8, false),
    ] {
        let error = provider.submit(&request(2, 2, format)).expect_err("only RGB is in scope for M1");
        assert!(error.to_string().contains("RGB"), "{format:?}: {error}");
    }
    assert_eq!(server.capture_count(), 0, "an unsupported document should not reach the manager");
}

#[test]
fn a_request_that_does_not_match_its_window_is_refused() {
    let server = FakeServer::start();
    let provider = TempEndpoint::new(&endpoint_json(server.port(), "tok-shape")).provider();
    let mut short = request(4, 3, RGB8);
    short.pixels.pop();
    assert!(provider.submit(&short).is_err(), "too few samples for the window");
    let mut wide = request(4, 3, RGB8);
    wide.coverage.pop();
    assert!(provider.submit(&wide).is_err(), "too few coverage values for the window");
    let empty = AiRequest {
        task: AiTask::FillMaskedArea,
        window: Rect::EMPTY,
        pixels: Vec::new(),
        format: RGB8,
        coverage: Vec::new(),
        prompt: None,
        client_version: "photocraft/test".into(),
    };
    assert!(provider.submit(&empty).is_err(), "an empty window cannot be filled");
    let mut poisoned = request(4, 3, RGB8);
    poisoned.pixels[0] = f32::NAN;
    let error = provider.submit(&poisoned).expect_err("a NaN cannot be sent as a pixel");
    assert!(error.to_string().contains("not a number"), "{error}");
    let mut outside = request(4, 3, RGB8);
    outside.coverage[0] = 1.4;
    assert!(provider.submit(&outside).is_err(), "coverage outside 0..=1 is not a selection");
    assert_eq!(server.capture_count(), 0, "a bad request should not reach the manager");
}

#[test]
fn a_prompt_is_sent_as_text_or_refused() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-prompt"));
    let mut req = request(2, 2, RGB8);
    req.prompt = Some("remove the cat\nplease".into());
    server.queue_json(202, r#"{"id": "job-prompt"}"#);
    endpoint.provider().submit(&req).expect("a plain-text prompt should be sent");
    let captured = server.captured();
    let boundary = header_value(captured[0].header("content-type")).expect("the upload should name a boundary");
    let job: serde_json::Value = serde_json::from_slice(&multipart_parts(&captured[0].body, &boundary)[0].1).expect("the job part should be JSON");
    assert_eq!(job["prompt"], "remove the cat\nplease");

    let mut control = request(2, 2, RGB8);
    control.prompt = Some("remove\0the cat".into());
    assert!(endpoint.provider().submit(&control).is_err(), "a prompt with a NUL is not plain text");
    let mut long = request(2, 2, RGB8);
    long.prompt = Some("q".repeat(5 * 1024));
    assert!(endpoint.provider().submit(&long).is_err(), "a prompt over 4 KiB is refused");
}

#[test]
fn a_job_id_that_cannot_go_in_a_url_is_refused() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-id"));
    for body in [r#"{"id": ""}"#, r#"{"id": "a b"}"#, r#"{"id": "a/b"}"#, r#"{"id": ".."}"#, r#"{"id": "a?b"}"#, r#"{"notAnId": 1}"#, "not json"] {
        server.queue_json(202, body);
        assert!(endpoint.provider().submit(&request(2, 2, RGB8)).is_err(), "{body} is not a usable job id");
    }
    server.queue_json(202, &format!(r#"{{"id": "{}"}}"#, "j".repeat(600)));
    assert!(endpoint.provider().submit(&request(2, 2, RGB8)).is_err(), "an absurdly long job id is refused");
    let calls = server.captured();
    assert_eq!(calls.iter().filter(|call| call.path != "/v1/jobs").count(), 0, "a refused job id must not be sent back to the manager");
}

#[test]
fn tasks_lists_what_the_manager_offers() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-tasks"));
    let provider = endpoint.provider();
    server.queue_json(200, r#"{"tasks": [{"id": "fill_masked_area", "needs_prompt": false, "max_pixels": 400000000}, {"id": "select_subject"}]}"#);
    assert_eq!(provider.tasks().expect("the task list should parse"), vec![AiTask::FillMaskedArea]);
    server.queue_json(200, r#"{"tasks": []}"#);
    assert!(provider.tasks().expect("an empty task list is a valid answer").is_empty());
    server.queue_json(200, r#"{"nonsense": true}"#);
    assert!(provider.tasks().is_err(), "an answer that is not a task list is an error");
    assert_eq!(TempEndpoint::missing().provider().tasks().expect_err("no manager, no tasks"), AiError::NoProvider);
}

#[test]
fn job_states_map_to_the_engine_states() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-state"));
    let provider = endpoint.provider();
    server.queue_json(202, r#"{"id": "job-state"}"#);
    let id = provider.submit(&request(2, 2, RGB8)).expect("the job should be accepted");
    let cases: [(&str, AiJobState); 5] = [
        (
            r#"{"state": "queued", "progress": 0.0, "message": "waiting for the model"}"#,
            AiJobState::Queued { progress: 0.0, message: "waiting for the model".into() },
        ),
        (r#"{"state": "running", "progress": 1.5, "message": "more than whole"}"#, AiJobState::Running { progress: 1.0, message: "more than whole".into() }),
        (r#"{"state": "done", "progress": 1.0, "message": ""}"#, AiJobState::Done),
        (r#"{"state": "failed", "message": "the model ran out of memory"}"#, AiJobState::Failed("the model ran out of memory".into())),
        (r#"{"state": "cancelled"}"#, AiJobState::Cancelled),
    ];
    for (body, want) in cases {
        server.queue_json(200, body);
        assert_eq!(provider.poll(&id).expect("the state should map"), want, "{body}");
    }
    server.queue_json(200, r#"{"state": "levitating"}"#);
    assert!(provider.poll(&id).is_err(), "a state PhotoCraft does not know is an error");
    server.queue_json(200, r#"{"progress": 0.5}"#);
    assert!(provider.poll(&id).is_err(), "an answer with no state is an error");
}

#[test]
fn a_job_this_session_never_started_is_an_error() {
    let server = FakeServer::start();
    let provider = TempEndpoint::new(&endpoint_json(server.port(), "tok-unknown")).provider();
    for id in ["never-submitted", "a b", ""] {
        let error = provider.poll(&AiJobId::new(id)).expect_err("PhotoCraft must not guess job ids");
        assert!(matches!(error, AiError::Other(_)), "{id:?}: {error}");
    }
    assert!(provider.cancel(&AiJobId::new("never-submitted")).is_err());
    assert!(provider.result(&AiJobId::new("never-submitted")).is_err());
    assert_eq!(server.capture_count(), 0, "an id this session never got must not be sent to the manager");
}

#[test]
fn a_json_answer_over_the_cap_is_refused() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-cap"));
    let provider = endpoint.provider();
    server.queue_json(202, r#"{"id": "job-cap"}"#);
    let id = provider.submit(&request(2, 2, RGB8)).expect("the job should be accepted");
    server.queue(response(200, "application/json", format!(r#"{{"state": "running", "message": "{}"}}"#, "m".repeat(100 * 1024)).as_bytes()));
    let error = provider.poll(&id).expect_err("a 100 KiB status answer is over the protocol cap");
    assert!(error.to_string().contains("limit"), "{error}");
}

#[test]
fn provenance_that_disagrees_with_the_window_is_refused() {
    let server = FakeServer::start();
    let endpoint = TempEndpoint::new(&endpoint_json(server.port(), "tok-size"));
    let req = request(4, 3, RGB8);
    let png = pixels::encode_image(&req.pixels, req.format, 4, 3).expect("the client should encode its own window");
    server.queue_json(202, r#"{"id": "job-size"}"#);
    let provider = endpoint.provider();
    let id = provider.submit(&req).expect("the job should be accepted");
    server.queue_json(200, r#"{"model": "m", "device": "cpu", "provider": "local", "elapsed_ms": 1, "image_size": [99, 99]}"#);
    let error = provider.result(&id).expect_err("a result that claims another size is not this window");
    assert!(error.to_string().contains("size"), "{error}");
    // Missing provenance says "unknown" rather than failing a finished job.
    server.queue_json(200, r#"{}"#);
    server.queue(response(200, "image/png", &png));
    let result = provider.result(&id).expect("a result with no provenance is still a result");
    assert_eq!((result.model.as_str(), result.device.as_str(), result.provider.as_str(), result.elapsed_ms), ("unknown", "unknown", "unknown", 0));
}
