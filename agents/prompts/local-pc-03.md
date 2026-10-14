# Prompt for the local agent: PC-03 (Craft AI Manager client crate)

Run in a fresh worktree with `codex --profile strata`. Paste everything below the line.

---

You are working on a fork of PhotoCraft, a Rust image editor. Before writing code, read: `AGENTS.md` (§2 layering, §3 never-crash rules, §5, §6), `agents/SPEC.md` › "Job protocol v1" and **amendment 11 (wire details), which is the contract you implement exactly**, the PC-03 row in `agents/TASKS.md`, and `crates/engine/src/ai.rs` (the `AiProvider` trait you implement).

**Task PC-03:** a new crate that implements `photocraft_engine::ai::AiProvider` by talking to Craft AI Manager over loopback HTTP. No UI, no app wiring (that's PC-04), no changes to the engine.

Work on branch `ai/pc-03-ai-client` with `CARGO_TARGET_DIR=target/agent-local`.

1. **Crate:** `crates/ai-client`, package `photocraft-ai-client`. Create `Cargo.toml` atomically (write it outside `crates/`, then move it in; AGENTS.md §6). Dependencies only from what the workspace already uses: `photocraft-engine`, `photocraft-color`, `photocraft-geom`, `serde`, `serde_json`, `thiserror` (workspace versions) and `png = "0.18"`. **No HTTP library**: use `std::net::TcpStream`. Register `("ai-client", Class::Layer(6))` in `xtask/src/layers.rs`. Start `lib.rs` with `#![forbid(unsafe_code)]` and the deny list from AGENTS.md §3.
2. **`Endpoint`:** read and parse `endpoint.json` from the path in SPEC amendment 11 (or `CRAFT_AI_ENDPOINT`). Missing file, unreadable file, bad JSON, `version` ≠ 1, port 0 → `AiError::NoProvider`. The client connects to `127.0.0.1` only.
3. **`CaimProvider`:** implements `AiProvider`. **Re-read `endpoint.json` on every `submit`** (the token changes each time the manager starts) and keep the endpoint used for that job with the job id, so `poll`, `result` and `cancel` go to the same manager. A connection refused → `AiError::NoProvider`.
4. **HTTP:** a small private module: write the request line, headers and body from SPEC amendment 11; read the status line, headers and a `Content-Length` body with the size caps; set connect/read timeouts. Reject chunked or missing `Content-Length`, oversize bodies, non-UTF-8 headers, and anything malformed, with an error, never a panic. Map 401 → "Craft AI Manager refused the request (token mismatch); restart Craft AI Manager", other errors → the server's `error` sentence.
5. **Encoding:** `AiRequest` → `job` JSON + `image` PNG (RGB or RGBA from the request's `PixelFormat`, 8- or 16-bit per the format's sample type, f32 clamped 0–1 and rounded) + `mask` PNG (8-bit gray from coverage). Only RGB formats; anything else → error (the engine already refuses non-RGB, but don't trust it). Decoding: `result.png` → f32 in the request's format; size, depth and channels must match the request, otherwise error. Fill `AiResult` provenance from `/result`.
6. **Tests** against an in-process fake server (`std::net::TcpListener` on `127.0.0.1:0`, a thread, scripted replies), with `CRAFT_AI_ENDPOINT` pointing at a temp file (use a per-test file and pass the path in, rather than setting the variable process-wide, so tests can run in parallel):
   - full happy path: health not required; submit → poll (running, then done) → result → pixels round-trip within 1/255 (8-bit) and 1/65535 (16-bit);
   - the `Authorization` header carries the token from the file; a request without it isn't sent;
   - no endpoint file, bad JSON, wrong version → `NoProvider`; connection refused → `NoProvider`;
   - 401 → the token-mismatch sentence; 404/409/500 → the server's sentence;
   - malformed status line, missing `Content-Length`, chunked reply, body over the cap, result PNG of the wrong size or depth → error, no panic;
   - `cancel` sends `DELETE /v1/jobs/{id}` to the endpoint the job was submitted to, even after the file changes;
   - `endpoint.json` naming a host other than 127.0.0.1 is refused (if you add a host field), and the client never connects elsewhere.

**Done when all of these pass** (paste the summary lines into the PR):
```
cargo test -p photocraft-ai-client
cargo clippy -p photocraft-ai-client --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
```
Then run every line of `agents/guards.txt`, add `PC-03 | photocraft | cargo test -p photocraft-ai-client`, fill in `agents/receipts/PC-03.md` from the template, set PC-03 to `review` in `agents/TASKS.md`, add a short entry at the top of `agents/HANDOFF.md`, **commit everything**, check `git status` is clean, and push the branch.

**Opening the PR:** use `gh pr create` if `gh` is installed and signed in. **Do not read stored credentials** (`git credential fill` or similar; SPEC › Security rules). If `gh` isn't available, stop after pushing and say so; Erik opens the PR from the link Git prints.
