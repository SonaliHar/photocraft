//! Desktop-only transport to an installed plugin's bundled engine. No shell or guest paths.
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::{EngineError, Result, jobs::JobCtx};

/// Transport metadata is validated by the shared host service before starting a worker.
pub(crate) struct ImageJob {
    pub options: Value,
    pub expected: (u32, u32),
    pub result_name: String,
    pub label: &'static str,
    pub opening: &'static str,
}

fn error(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

const DIAGNOSTIC_LINES: usize = 16;
const DIAGNOSTIC_BYTES: usize = 8192;
const DIAGNOSTIC_LINE_BYTES: usize = 2048;
const DIAGNOSTIC_CONTEXT_LINES: usize = 3;
const WORKER_LOG_BYTES: u64 = 16 * 1024 * 1024;
const WORKER_LOG_POLL_BYTES: u64 = 64 * 1024;

/// Preserve the first actual failure even when a backend prints GPU capabilities after it.
/// Keep a little following context and a bounded tail, never an unbounded copy of stderr.
#[derive(Default)]
struct WorkerDiagnostics {
    primary: Option<String>,
    tail: VecDeque<String>,
    tail_bytes: usize,
}

impl WorkerDiagnostics {
    fn record(&mut self, text: &str) {
        let text = text.trim();
        // Progress is machine-readable status, not part of the failure explanation.
        if text.is_empty() {
            return;
        }
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            if let Some(message) = value.get("error").and_then(Value::as_str) {
                self.record(&format!("error: {message}"));
            }
            return;
        }
        let mut end = text.len().min(DIAGNOSTIC_LINE_BYTES);
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let line = text.get(..end).unwrap_or_default().to_owned();
        let lower = line.to_ascii_lowercase();
        if self.primary.is_none() && (lower.contains("error") || lower.contains("failed")) {
            self.primary = Some(line);
            // Older diagnostics preceded the primary error. Retain details following it.
            self.tail.clear();
            self.tail_bytes = 0;
            return;
        }
        self.tail_bytes = self.tail_bytes.saturating_add(line.len() + 1);
        self.tail.push_back(line);
        let reserved = self.primary.as_ref().map_or(0, |line| line.len() + 1);
        let max_lines = DIAGNOSTIC_LINES.saturating_sub(usize::from(self.primary.is_some()));
        while self.tail.len() > max_lines || self.tail_bytes > DIAGNOSTIC_BYTES.saturating_sub(reserved) {
            // An OS error directly after the primary line is more useful than a long GPU
            // capability dump. Keep that initial context while rotating the later tail.
            let remove = if self.primary.is_some() { DIAGNOSTIC_CONTEXT_LINES.min(self.tail.len().saturating_sub(1)) } else { 0 };
            let Some(line) = self.tail.remove(remove) else { break };
            self.tail_bytes = self.tail_bytes.saturating_sub(line.len() + 1);
        }
    }

    fn message(&self) -> String {
        if let Some(primary) = &self.primary {
            let mut message = primary.clone();
            for line in &self.tail {
                message.push('\n');
                message.push_str(line);
            }
            message
        } else {
            self.tail.back().cloned().unwrap_or_else(|| "The processing engine exited without an error message".into())
        }
    }
}

/// Read a regular file with an independent cursor instead of waiting for pipe EOF. A native
/// worker can exit while a descendant still owns its stderr handle; no reader thread or
/// unbounded join may keep the image job and installation lease alive in that case.
struct WorkerLog {
    file: std::fs::File,
    position: u64,
    line: Vec<u8>,
    diagnostics: WorkerDiagnostics,
}

impl WorkerLog {
    fn open(path: &Path) -> Result<Self> {
        Ok(Self { file: std::fs::File::open(path).map_err(error)?, position: 0, line: Vec::new(), diagnostics: WorkerDiagnostics::default() })
    }

    fn length(&self) -> Result<u64> {
        let length = self.file.metadata().map_err(error)?.len();
        if length > WORKER_LOG_BYTES {
            return Err(error("Plugin processing stopped because its diagnostic output exceeded 16 MiB"));
        }
        Ok(length)
    }

    fn record_line(&mut self, ctx: &JobCtx) {
        let text = String::from_utf8_lossy(&self.line);
        self.diagnostics.record(&text);
        if let Ok(value) = serde_json::from_str::<Value>(&text)
            && let (Some(progress), Some(message)) = (value.get("progress").and_then(Value::as_f64), value.get("message").and_then(Value::as_str))
            && progress.is_finite()
        {
            ctx.progress(0.1 + 0.85 * (progress as f32 / 100.0).clamp(0.0, 1.0), message);
        }
        self.line.clear();
    }

    fn read_until(&mut self, ctx: &JobCtx, end: u64, budget: u64) -> Result<()> {
        let end = end.min(self.position.saturating_add(budget));
        let mut chunk = [0u8; 2048];
        while self.position < end {
            ctx.check()?;
            let length = end.saturating_sub(self.position).min(chunk.len() as u64) as usize;
            let bytes = chunk.get_mut(..length).ok_or_else(|| error("Invalid plugin diagnostic read length"))?;
            let count = self.file.read(bytes).map_err(error)?;
            if count == 0 {
                break;
            }
            self.position = self.position.saturating_add(count as u64);
            for byte in chunk.iter().take(count) {
                if *byte == b'\n' {
                    self.record_line(ctx);
                } else if self.line.len() < 8192 {
                    self.line.push(*byte);
                }
            }
        }
        Ok(())
    }

    fn poll(&mut self, ctx: &JobCtx) -> Result<()> {
        self.read_until(ctx, self.length()?, WORKER_LOG_POLL_BYTES)
    }

    fn finish(&mut self, ctx: &JobCtx) -> Result<()> {
        // Capture one bounded length. A descendant still writing cannot extend this drain.
        self.read_until(ctx, self.length()?, WORKER_LOG_BYTES)?;
        if !self.line.is_empty() {
            self.record_line(ctx);
        }
        Ok(())
    }
}

/// Runs the package-owned processor using the bounded RGBA8 sRGB image protocol.
pub(crate) fn process(doc: &Document, worker: &Path, job: &ImageJob, ctx: &JobCtx) -> Result<Document> {
    ctx.check()?;
    let temp = tempfile::Builder::new().prefix("photocraft-plugin-image-").tempdir().map_err(error)?;
    let input = temp.path().join("source.png");
    let output = temp.path().join("result.png");
    let request = temp.path().join("job.json");
    let exported = {
        use photocraft_codecs::{ChannelLayout, Format, Image, SampleType};
        let surface = doc.layers.first().and_then(|layer| layer.surface()).ok_or_else(|| error("The plugin input has no pixel surface"))?;
        if surface.format() != photocraft_color::PixelFormat::RGBA8 || doc.mode != photocraft_color::ColorMode::Rgb || doc.depth.bits() != 8 {
            return Err(error("The plugin input must be an 8-bit RGBA sRGB image"));
        }
        let image = Image::from_raw(doc.size.width, doc.size.height, ChannelLayout::Rgba, SampleType::U8, surface.to_interleaved(doc.bounds()))
            .map_err(error)?
            .with_icc(Some(photocraft_cms::Builtin::Srgb.profile().to_bytes().to_vec()));
        photocraft_codecs::encode(&image, Format::Png, &Default::default()).map_err(error)?
    };
    ctx.check()?;
    std::fs::write(&input, exported).map_err(error)?;
    let payload = json!({"apiVersion":1,"input":input,"output":output,"options":job.options,
        "outputSize":{"width":job.expected.0,"height":job.expected.1},"colorSpace":"srgb"});
    std::fs::write(&request, serde_json::to_vec(&payload).map_err(error)?).map_err(error)?;
    run_worker(worker, "--job", &request, temp.path(), job.label, ctx)?;
    ctx.progress(0.97, job.opening);
    read_result(doc, &output, job, ctx)
}

/// A task result is data only: it cannot open documents, apply edits or dispatch commands.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskResult {
    api_version: u32,
    pub message: String,
    pub data: Value,
}

const TASK_RESULT_BYTES: u64 = 128 << 10;

pub(crate) fn task(worker: &Path, options: &Value, ctx: &JobCtx) -> Result<TaskResult> {
    ctx.check()?;
    let temp = tempfile::Builder::new().prefix("photocraft-plugin-task-").tempdir().map_err(error)?;
    let output = temp.path().join("result.json");
    let request = temp.path().join("task.json");
    std::fs::write(&request, serde_json::to_vec(&json!({"apiVersion":1,"options":options,"output":output})).map_err(error)?).map_err(error)?;
    run_worker(worker, "--task", &request, temp.path(), "Plugin task", ctx)?;
    read_task_result(&output, ctx)
}

fn read_task_result(output: &Path, ctx: &JobCtx) -> Result<TaskResult> {
    ctx.check()?;
    let metadata = std::fs::symlink_metadata(output).map_err(error)?;
    if !metadata.file_type().is_file() || metadata.len() > TASK_RESULT_BYTES {
        return Err(error("Native plugin task result must be a regular JSON file of at most 128 KiB"));
    }
    let file = std::fs::File::open(output).map_err(error)?;
    if !file.metadata().map_err(error)?.is_file() {
        return Err(error("Native plugin task result must be a regular JSON file"));
    }
    let mut bytes = Vec::new();
    file.take(TASK_RESULT_BYTES + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > TASK_RESULT_BYTES {
        return Err(error("Native plugin task result grew beyond 128 KiB"));
    }
    let result: TaskResult = serde_json::from_slice(&bytes).map_err(error)?;
    if result.api_version != 1 || !result.data.is_object() || result.message.trim().is_empty() || result.message.len() > 8192 {
        return Err(error("Native plugin task requires apiVersion 1, object data, and a nonempty message of at most 8192 UTF-8 bytes"));
    }
    ctx.check()?;
    Ok(result)
}

fn run_worker(worker: &Path, flag: &str, request: &Path, directory: &Path, label: &str, ctx: &JobCtx) -> Result<()> {
    ctx.check()?;
    let log_path = directory.join("worker.log");
    let log_writer = std::fs::File::create(&log_path).map_err(error)?;
    // File::try_clone shares its cursor, so open a separate read handle before passing the
    // write handle to the child. A regular-file read never waits for another handle to close.
    let mut worker_log = WorkerLog::open(&log_path)?;
    let mut command = Command::new(worker);
    command.arg(flag).arg(request).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(log_writer);
    if let Some(parent) = worker.parent() {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| error(format!("Could not start the {label} plugin's processing engine: {e}")))?;
    drop(command);
    wait_for_worker(&mut child, &mut worker_log, label, ctx)
}

fn wait_for_worker(child: &mut std::process::Child, worker_log: &mut WorkerLog, label: &str, ctx: &JobCtx) -> Result<()> {
    let started = Instant::now();
    let mut stopping = None;
    let mut timed_out = false;
    let status = loop {
        // Keep log work bounded so cancellation and the worker deadline are checked promptly.
        if let Err(e) = worker_log.poll(ctx)
            && !matches!(e, EngineError::Cancelled)
        {
            let _ = child.kill();
            let _ = child.wait();
            break Err(e);
        }
        if stopping.is_none() && (ctx.cancelled() || started.elapsed() > Duration::from_secs(45 * 60)) {
            timed_out = !ctx.cancelled();
            // A cooperative worker stops inference (and any child engine) before exiting.
            // Closing this pipe also cancels it if PhotoCraft exits unexpectedly.
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(b"cancel\n");
            }
            stopping = Some(Instant::now());
        }
        if stopping.is_some_and(|t| t.elapsed() > Duration::from_secs(10)) {
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error(e));
            }
        }
    };
    let log_result = worker_log.finish(ctx);
    ctx.check()?;
    if timed_out {
        return Err(error(format!("{label} worker exceeded the 45-minute time limit")));
    }
    let status = status?;
    log_result?;
    if !status.success() {
        return Err(error(format!("{label} failed ({status}): {}", worker_log.diagnostics.message())));
    }
    Ok(())
}

fn read_result(doc: &Document, output: &Path, job: &ImageJob, ctx: &JobCtx) -> Result<Document> {
    ctx.check()?;
    // Bound compressed output by the expected raw pixels plus finite metadata overhead.
    let max_bytes = (u64::from(job.expected.0) * u64::from(job.expected.1)).saturating_mul(4).saturating_add(64 << 20).min(1024 << 20);
    // Check the PNG header before decoding. A buggy worker cannot allocate an oversized
    // document by returning dimensions different from the requested bounded result.
    let mut file = std::fs::File::open(output).map_err(error)?;
    let metadata = file.metadata().map_err(error)?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(error(format!("Plugin output exceeds {} MiB", max_bytes / (1024 * 1024))));
    }
    let mut header = [0u8; 26];
    file.read_exact(&mut header).map_err(error)?;
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" || (width, height) != job.expected || header[24] != 8 {
        return Err(error(format!("{} returned an invalid PNG or unexpected dimensions/depth", job.label)));
    }
    if !matches!(header[25], 2 | 6) {
        return Err(error("The plugin must return an 8-bit RGB or RGBA PNG"));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(metadata.len() as usize).map_err(error)?;
    bytes.extend_from_slice(&header);
    file.take(max_bytes + 1).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > max_bytes {
        return Err(error(format!("Plugin output grew beyond {} MiB", max_bytes / (1024 * 1024))));
    }
    ctx.check()?;
    let mut result = photocraft_io::import("plugin-result.png", &bytes).map_err(error)?.document;
    if (result.size.width, result.size.height) != job.expected || result.depth.bits() != 8 {
        return Err(error(format!("{} decoded to unexpected dimensions/depth", job.label)));
    }
    result.name.clone_from(&job.result_name);
    result.resolution_dpi = doc.resolution_dpi;
    ctx.check()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKER_FIXTURE: &str = "plugin_runtime::tests::native_worker_diagnostic_fixture";

    #[test]
    fn native_task_result_is_bounded_typed_and_cancellable() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("result.json");
        let ctx = JobCtx::new();
        std::fs::write(&output, br#"{"apiVersion":1,"message":"Ready","data":{"count":2}}"#).unwrap();
        let result = read_task_result(&output, &ctx).unwrap();
        assert_eq!(result.message, "Ready");
        assert_eq!(result.data["count"], 2);
        for malformed in [
            json!({"apiVersion":2,"message":"Ready","data":{}}),
            json!({"apiVersion":1,"message":" ","data":{}}),
            json!({"apiVersion":1,"message":"Ready","data":[]}),
            json!({"apiVersion":1,"message":"x".repeat(8193),"data":{}}),
            json!({"apiVersion":1,"message":"Ready","data":{},"commands":[]}),
        ] {
            std::fs::write(&output, malformed.to_string()).unwrap();
            assert!(read_task_result(&output, &ctx).is_err());
        }
        std::fs::write(&output, b"not json").unwrap();
        assert!(read_task_result(&output, &ctx).is_err());
        std::fs::File::create(&output).unwrap().set_len(TASK_RESULT_BYTES + 1).unwrap();
        assert!(read_task_result(&output, &ctx).unwrap_err().to_string().contains("128 KiB"));
        assert!(read_task_result(directory.path(), &ctx).unwrap_err().to_string().contains("regular JSON"));
        ctx.cancel();
        assert!(matches!(read_task_result(&output, &ctx), Err(EngineError::Cancelled)));
    }

    struct FixtureChild(std::process::Child);

    impl Drop for FixtureChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn diagnostic_fixture(path: &Path, mode: &str) -> FixtureChild {
        let writer = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", WORKER_FIXTURE, "--ignored", "--nocapture"])
            .env("PHOTOCRAFT_DIAGNOSTIC_FIXTURE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(writer);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        FixtureChild(command.spawn().unwrap())
    }

    #[test]
    #[ignore = "subprocess fixture used by the diagnostic transport tests"]
    fn native_worker_diagnostic_fixture() {
        match std::env::var("PHOTOCRAFT_DIAGNOSTIC_FIXTURE").unwrap_or_default().as_str() {
            "hold" => {
                // This process retains an inherited stderr handle until the owner stops it.
                let mut input = Vec::new();
                std::io::stdin().read_to_end(&mut input).unwrap();
            }
            "fail" => {
                eprintln!("processing ERROR: fixture failure");
                std::process::exit(7);
            }
            _ => eprintln!(r#"{{"progress":100,"message":"Fixture complete"}}"#),
        }
    }

    #[test]
    fn native_log_drain_finishes_while_another_process_retains_the_inherited_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.log");
        std::fs::write(&path, b"").unwrap();
        // A descendant retaining stderr after its worker exits has this same handle-lifetime
        // behavior. Keep both process handles here so the test always reaps its fixtures.
        let mut holder = diagnostic_fixture(&path, "hold");
        let mut worker = diagnostic_fixture(&path, "success");
        let mut log = WorkerLog::open(&path).unwrap();
        let ctx = JobCtx::new();
        wait_for_worker(&mut worker.0, &mut log, "Fixture", &ctx).unwrap();
        assert!(holder.0.try_wait().unwrap().is_none());
        assert_eq!(ctx.message(), "Fixture complete");
        assert!((ctx.fraction() - 0.95).abs() < 0.001);
        // A second final drain is bounded as well; it does not require a closed writer.
        log.finish(&ctx).unwrap();
        drop(holder);
    }

    #[test]
    fn native_log_reads_are_bounded_and_bursts_retain_primary_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.log");
        let mut writer = std::fs::File::create(&path).unwrap();
        writeln!(writer, r#"{{"progress":50,"message":"Halfway"}}"#).unwrap();
        writeln!(writer, "inference ERROR: primary fixture failure").unwrap();
        writeln!(writer, "could not open the model").unwrap();
        for index in 0..1024 {
            writeln!(writer, "GPU {index}: {}", "details ".repeat(32)).unwrap();
        }
        writeln!(writer, r#"{{"progress":90,"message":"Finishing"}}"#).unwrap();
        writer.write_all(b"last line without newline").unwrap();
        let mut log = WorkerLog::open(&path).unwrap();
        let ctx = JobCtx::new();
        log.poll(&ctx).unwrap();
        assert_eq!(log.position, WORKER_LOG_POLL_BYTES);
        assert_eq!(ctx.message(), "Halfway");
        log.finish(&ctx).unwrap();
        let message = log.diagnostics.message();
        assert!(message.starts_with("inference ERROR: primary fixture failure"));
        assert!(message.contains("could not open the model"));
        assert!(message.contains("GPU 1023"));
        assert!(message.contains("last line without newline"));
        assert!(message.len() <= DIAGNOSTIC_BYTES);
        assert!(message.lines().count() <= DIAGNOSTIC_LINES);
        assert_eq!(ctx.message(), "Finishing");
        assert!((ctx.fraction() - 0.865).abs() < 0.001);
    }

    #[test]
    fn excessive_native_diagnostic_output_fails_and_reaps_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.log");
        let writer = std::fs::File::create(&path).unwrap();
        writer.set_len(WORKER_LOG_BYTES + 1).unwrap();
        let mut worker = diagnostic_fixture(&path, "hold");
        let mut log = WorkerLog::open(&path).unwrap();
        let error = wait_for_worker(&mut worker.0, &mut log, "Fixture", &JobCtx::new()).unwrap_err().to_string();
        assert!(error.contains("diagnostic output exceeded 16 MiB"), "{error}");
        assert_eq!(log.position, 0);
        assert!(worker.0.try_wait().unwrap().is_some());
    }

    #[test]
    fn native_log_transport_preserves_failure_and_cooperative_cancellation() {
        for mode in ["fail", "hold"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("worker.log");
            std::fs::write(&path, b"").unwrap();
            let mut worker = diagnostic_fixture(&path, mode);
            let mut log = WorkerLog::open(&path).unwrap();
            let ctx = JobCtx::new();
            if mode == "hold" {
                ctx.cancel();
            }
            let result = wait_for_worker(&mut worker.0, &mut log, "Fixture", &ctx);
            assert!(worker.0.try_wait().unwrap().is_some());
            if mode == "hold" {
                assert!(matches!(result, Err(EngineError::Cancelled)));
            } else {
                assert!(result.unwrap_err().to_string().contains("processing ERROR: fixture failure"));
            }
        }
    }

    #[test]
    fn native_failure_keeps_its_primary_error_before_gpu_capability_details() {
        let mut diagnostics = WorkerDiagnostics::default();
        diagnostics.record("[0 GPU] driver=1 compute=1");
        diagnostics.record(r#"{"progress":50,"message":"Reconstructing image"}"#);
        diagnostics.record("Neural inference FAILED (exit code: -9)");
        diagnostics.record("could not open model.param");
        for i in 0..32 {
            diagnostics.record(&format!("[{i} GPU] subgroup=64 basic=1 vote=1 ballot=1 shuffle=1"));
        }
        diagnostics.record(r#"{"progress":100,"message":"Finished"}"#);
        let message = diagnostics.message();
        assert!(message.starts_with("Neural inference FAILED (exit code: -9)"));
        assert!(message.contains("could not open model.param"));
        assert!(message.contains("[31 GPU]"));
        assert!(!message.contains("Reconstructing image"));
        assert!(!message.contains("Finished"));
        assert!(message.lines().count() <= DIAGNOSTIC_LINES);
        assert!(message.len() <= DIAGNOSTIC_BYTES);
    }

    #[test]
    fn structured_native_errors_remain_visible_and_bounded() {
        let mut diagnostics = WorkerDiagnostics::default();
        diagnostics.record(r#"{"progress":10,"message":"Preparing"}"#);
        diagnostics.record(r#"{"error":"The bundled model could not be opened"}"#);
        diagnostics.record(r#"{"progress":100,"message":"Finished"}"#);
        assert_eq!(diagnostics.message(), "error: The bundled model could not be opened");
        let mut oversized = WorkerDiagnostics::default();
        oversized.record(&json!({"error":"x".repeat(DIAGNOSTIC_BYTES * 2)}).to_string());
        assert!(oversized.message().starts_with("error: "));
        assert!(oversized.message().len() <= DIAGNOSTIC_BYTES);
    }

    #[test]
    fn native_diagnostic_bounds_preserve_utf8_and_fall_back_to_the_last_text() {
        let mut diagnostics = WorkerDiagnostics::default();
        diagnostics.record("initial backend message");
        diagnostics.record("last backend message");
        assert_eq!(diagnostics.message(), "last backend message");
        diagnostics.record(&format!("error: {}", "🦀é".repeat(4096)));
        for _ in 0..100 {
            diagnostics.record(&"模型🦀".repeat(4096));
        }
        let message = diagnostics.message();
        assert!(message.starts_with("error: "));
        assert!(message.len() <= DIAGNOSTIC_BYTES);
        assert!(message.lines().count() <= DIAGNOSTIC_LINES);
        assert_eq!(String::from_utf8(message.as_bytes().to_vec()).unwrap(), message);
        assert!(diagnostics.tail.iter().all(|line| line.len() <= DIAGNOSTIC_LINE_BYTES));
    }

    #[test]
    fn native_result_must_match_bounded_dimensions_and_eight_bit_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.png");
        let mut source = Document::new("Source", photocraft_doc::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        source.resolution_dpi = 144.0;
        let job = ImageJob { options: json!({}), expected: (8, 6), result_name: "Processed copy".into(), label: "Image processing", opening: "Opening copy" };
        let valid = photocraft_io::export(&source, "png", &Default::default()).unwrap().bytes;
        std::fs::write(&path, &valid).unwrap();
        let result = read_result(&source, &path, &job, &JobCtx::new()).unwrap();
        assert_eq!(result.name, "Processed copy");
        assert_eq!(result.resolution_dpi, 144.0);
        assert_eq!(result.size, source.size);
        assert_eq!(result.depth.bits(), 8);
        for (index, replacement) in [(0, b'X'), (16, 127), (20, 127), (24, 16)] {
            let mut broken = valid.clone();
            broken[index] = replacement;
            std::fs::write(&path, broken).unwrap();
            let error = read_result(&source, &path, &job, &JobCtx::new()).err().unwrap().to_string();
            assert!(error.contains("unexpected dimensions/depth"), "{error}");
        }
        std::fs::write(&path, b"truncated").unwrap();
        assert!(read_result(&source, &path, &job, &JobCtx::new()).is_err());
        std::fs::write(&path, valid).unwrap();
        let ctx = JobCtx::new();
        ctx.cancel();
        assert!(matches!(read_result(&source, &path, &job, &ctx), Err(EngineError::Cancelled)));
    }

    #[test]
    fn generic_native_result_rejects_gray_palette_and_oversized_files_before_decoding() {
        use photocraft_codecs::{ChannelLayout, Format, Image, SampleType};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.png");
        let source = Document::new("Source", photocraft_doc::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let job = ImageJob { options: json!({}), expected: (8, 6), result_name: "Processed".into(), label: "Image processing", opening: "Opening copy" };
        let gray = Image::from_raw(8, 6, ChannelLayout::Gray, SampleType::U8, vec![128; 48]).unwrap();
        let bytes = photocraft_codecs::encode(&gray, Format::Png, &Default::default()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let message = read_result(&source, &path, &job, &JobCtx::new()).unwrap_err().to_string();
        assert!(message.contains("RGB or RGBA"), "{message}");
        let mut palette = bytes;
        palette[25] = 3;
        std::fs::write(&path, palette).unwrap();
        assert!(read_result(&source, &path, &job, &JobCtx::new()).unwrap_err().to_string().contains("RGB or RGBA"));
        std::fs::File::create(&path).unwrap().set_len((512 << 20) + 1).unwrap();
        assert!(read_result(&source, &path, &job, &JobCtx::new()).unwrap_err().to_string().contains("exceeds 64 MiB"));
    }
}
