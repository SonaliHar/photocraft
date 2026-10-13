# Prompt for Codex: PC-01 (AI provider interface in the engine)

Run from `E:\CraftML\work\photocraft` with plain `codex` (cloud model), or as a Codex cloud task on `erikerikerik/photocraft`. Paste everything below the line.

---

You are working on a fork of PhotoCraft, a Rust image editor. Before writing code, read: `AGENTS.md` (all of it), `agents/SPEC.md` (especially "v1.1 amendments" §1–§5), the PC-01 row in `agents/TASKS.md`, and the rules at the top of `agents/FEATURES.md`.

**Task PC-01:** add the AI provider interface to the engine. Interface and tests only: no command, no menu item, no network code, no new dependencies.

1. Work on a new branch `ai/pc-01-provider-trait` from `origin/main`. Use `CARGO_TARGET_DIR=target/agent-codex`.
2. New module `crates/engine/src/ai.rs` (register it in `crates/engine/src/lib.rs` as `pub mod ai;`). It defines:
   - `AiTask` (an enum; for now only `FillMaskedArea`, plus a stable string id `"fill_masked_area"` for the wire protocol).
   - `AiRequest`: task, the window rectangle in document coordinates, the window pixels as `Vec<f32>` with their `photocraft_color::PixelFormat`, the selection coverage over the window as `Vec<f32>` (0–1, one value per pixel), an optional prompt, and the client version string.
   - `AiJobId` (opaque string), `AiJobState` (`Queued`, `Running`, `Done`, `Failed(String)`, `Cancelled`) with a progress fraction and message.
   - `AiResult`: pixels for the same window (same size and format as the request), plus where it ran (`model`, `device`, `provider`, `elapsed_ms`) for FEATURES rule B08.
   - `AiError` with plain-sentence messages; the "no provider" case reads exactly: `Craft AI Manager isn't running`. Convert into `EngineError` (add a `From` impl or map to `EngineError::Other`; don't add an `EngineError` variant unless you must).
   - `pub trait AiProvider: Send + Sync` with `tasks(&self) -> Result<Vec<AiTask>, AiError>`, `submit(&self, req: &AiRequest) -> Result<AiJobId, AiError>`, `poll(&self, id: &AiJobId) -> Result<AiJobState, AiError>`, `result(&self, id: &AiJobId) -> Result<AiResult, AiError>`, `cancel(&self, id: &AiJobId) -> Result<(), AiError>`. Blocking calls are fine: they will run on a job worker thread.
   - A helper `run_to_completion(provider, req, ctx: &crate::jobs::JobCtx, poll_every: Duration) -> Result<AiResult, AiError>` that submits, polls, reports progress through `ctx.progress`, calls `cancel` and returns a cancelled error when `ctx.cancelled()` is true, and validates that the result's size and format match the request (mismatch → error, never a panic). Make the sleep injectable or short so tests stay fast; on wasm the helper must still compile.
3. On `Session` (`crates/engine/src/lib.rs`): a private field holding `Option<Arc<dyn AiProvider>>`, `pub fn set_ai_provider(&mut self, p: Option<Arc<dyn AiProvider>>)` and `pub fn ai_provider(&self) -> Option<Arc<dyn AiProvider>>`. Initialise it to `None` everywhere a `Session` is built. If `Session` derives traits a trait object can't satisfy (e.g. `Debug`), implement them by hand for that field rather than removing the derive.
4. A `FakeProvider` for tests (in `ai.rs` under `#[cfg(test)]`, or `pub` behind a `testing` module if later integration tests need it): fills the masked area with a solid colour, and can be configured to fail, to report progress over N polls, to never finish (for cancel tests), or to return a wrong-size result.
5. Tests (in `crates/engine/src/ai/tests.rs` or `ai_tests.rs`, following the repo's pattern): submit/poll/result happy path; progress reaches 1.0; cancel mid-job returns cancelled and `cancel` was called; failure message passes through; wrong-size result → error; no provider → the exact "Craft AI Manager isn't running" text; `Session` starts with no provider and `set_ai_provider` round-trips.

Rules (from AGENTS.md, enforced by clippy): no `unwrap`/`expect`/`panic!`/indexing panics outside tests; no `unsafe`; the new module starts with the crate's deny list if the crate doesn't already apply it; nothing below L6 may use UI crates; must build for wasm.

**Done when all of these pass** (paste the summary lines into the PR):
```
cargo test -p photocraft-engine --lib ai
cargo clippy -p photocraft-engine --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
```
Then: run every line in `agents/guards.txt` (all must pass); fill in `agents/receipts/PC-01.md` from `agents/receipts/TEMPLATE.md`; set PC-01 to `review` in `agents/TASKS.md`; add a short entry at the top of `agents/HANDOFF.md`; commit on the branch and push it. Open a PR titled `[PC-01] AI provider interface in the engine` if you can; otherwise say so and stop. Do not merge, and do not touch `main`.
