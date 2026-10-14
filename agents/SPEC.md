# PhotoCraft AI: spec v1

Status: agreed direction, Oct 13, 2026. Owner: Erik. Planning and review: Claude. Coding: Codex, plus the local agent (Strata on CraftML) for narrow tested tasks.
This file goes into the repo as `agents/SPEC.md`. Every agent reads it, plus `agents/TASKS.md` and `agents/HANDOFF.md`, before starting.

> **v1.1 (Oct 13, 2026):** see "v1.1 amendments" at the end. Where they disagree with the text above, the amendments win.

## Goal

Firefly-style AI tools for PhotoCraft. First: removing lines and objects / filling a masked area. Later: select subject, remove background, generative fill, expand canvas, upscale.
Local first, with the structure ready for web services. Lean and safe: PhotoCraft itself stays free of models, GPU runtimes and API keys.

## Decisions

| Topic | Decision |
|---|---|
| Architecture | **Thin PhotoCraft fork + a separate AI manager app** (option 2). |
| Platforms | Windows first. Mac (CoreML) and web come later. |
| Model licenses | Commercial-safe only. Each model's license is checked before it enters the catalog. |
| Weights | Never stored in a repo. Only ONNX and safetensors files; never pickle (`.ckpt`, `.pt`, `.bin` pickles). |
| ComfyUI | Optional backend (step 3), never a dependency. |
| GPU | CUDA in step 2.b. "Auto" by default, always with a CPU fallback. |
| Upstream | Work on a fork (maintainers unresponsive). Keep the fork change small and match PhotoCraft's `AGENTS.md` and layering rules, so a PR stays possible. |
| Branding | Don't use the ArtCraft name or logo for the manager. "Craft AI Manager" is a working name only. |

## Architecture

```
PhotoCraft (fork, thin)                     AI manager (separate app)
  AI panel + contextual bar        --->       job API (127.0.0.1, token)
  hook: masked area out, layer in  <---       task router
                                               backends: built-in ONNX (ort) | ComfyUI | web
                                               model catalog, downloads, SHA-256, devices, keychain
```

### PhotoCraft fork: what changes

- **New crate (working name `photocraft-ai-client`):** speaks the job protocol below.
  - It must respect PhotoCraft's layer rules (`cargo xtask layers` must pass); nothing below the UI layer may depend on UI code.
- **New tool "Fill (AI)":**
  1. Takes the current selection or quick mask and the pixels under it.
  2. Runs as a background job on a document snapshot, with a `CancelToken` and progress.
  3. Commits the result as a **new layer** in one undoable transaction. The original is never changed.
- **AI panel / contextual bar:** task buttons. Settings has one section: manager address status (connected or not) and a link to open the manager.
- **Behavior with no manager:** PhotoCraft works exactly as today. The AI tools show "AI manager not running" with a way to start it. Nothing crashes.
- **What stays out of PhotoCraft:** no model files, no ONNX Runtime, no CUDA, no keys, no network calls except to the local manager.

### AI manager: what it is

- Its own repository and installer (Rust; egui fits your existing Mac manager work).
- Runs on demand or in the tray, and listens only on `127.0.0.1`.
- **Owns:**
  - the model catalog
  - downloads with SHA-256 checks
  - update checks
  - disk usage
  - device choice
  - API keys (in the Windows Credential Manager / Keychain)
  - the backends
- It can later serve other Craft apps (LightCraft, FilmCraft) with the same protocol.

### PhotoCraft's own rules (from its AGENTS.md) that the fork change must follow

- **No panics in non-test code:** no `unwrap`, `expect` or `panic!`. New commands need tests that show they fail gracefully.
- **No `unsafe`** outside `photocraft-tablet`. (ONNX Runtime and CUDA live in the manager anyway.)
- **Everything is a command:** "Fill (AI)" and every other AI action must be an engine command, with its id in `menu_catalog.rs`.
  - After adding commands, run `cargo xtask parity`, commit the regenerated `docs/parity-checklist.md`, and run the panic fuzzing (`cargo test -p photocraft-engine --test panic_hunt -- --ignored`).
- **Native only:** never add Tauri or any webview/JS UI.
- **Checks before every PR:**
  - `cargo xtask layers`
  - `cargo clippy -p <crates> --all-targets -- -D warnings`
  - `cargo test -p <crates touched>`
- **Parallel agents:**
  - Each uses its own build folder (`CARGO_TARGET_DIR=target/agent-<name>`) and edits only files it owns.
  - Keep every `Cargo.toml` valid at all times (write to a temp file, then move it into place).
- **Never commit:** fonts, corpus files, `plan/` or `log/`.
- **Clean room:** copy no proprietary code, shaders, profiles or assets.

## Job protocol v1 (local HTTP)

- **Discovery:** the manager writes `%LOCALAPPDATA%\CraftAI\endpoint.json` with `{ "version": 1, "port": N, "token": "<random>" }`.
  - Only the current user can read the file.
  - A new token is made on each start.
- **Auth:** every request carries `Authorization: Bearer <token>`. Requests without it are refused.
- **Data:** JSON carries only the job details. Image and mask go as separate binary parts (multipart), never base64 inside JSON.

| Call | Purpose |
|---|---|
| `GET /v1/health` | Version, status, devices available |
| `GET /v1/tasks` | Which tasks are available now, and their limits (max size, needs prompt, etc.) |
| `POST /v1/jobs` | Multipart: `job.json` + `image.png` + `mask.png`. Returns `{ "id" }` |
| `GET /v1/jobs/{id}` | `{ state: queued/running/done/failed/cancelled, progress 0–1, message }` |
| `GET /v1/jobs/{id}/result` | PNG + JSON (bounds, model used, device used, timing) |
| `DELETE /v1/jobs/{id}` | Cancel |

Example `job.json`:

```json
{ "task": "fill_masked_area", "image_size": [W, H], "region": [x, y, w, h],
  "prompt": null, "settings": {}, "provider": "auto", "client": "photocraft/<version>" }
```

- **Versioning:** the path carries the version (`/v1/`). New fields are optional; breaking changes go to `/v2/`.
- **Web providers (step 4):** the manager needs explicit consent before a job may upload. Every result reports where it ran.

## Milestones and build order

### Milestone 1: mask → CPU fill → new layer (end to end)

**Done when:**

1. The user masks an area with PhotoCraft's existing selection or quick-mask tools and runs **Fill (AI)**.
2. PhotoCraft sends the area and mask to the manager. The manager runs **LaMa (ONNX) on the CPU**.
3. On first use the model downloads, is checked against its SHA-256, and then works offline.
4. The job shows progress and can be cancelled.
5. The result appears as a new layer named "Fill (AI)". The original is untouched, and the fill undoes in one step.
6. **Fixed-input handling:** LaMa ONNX conversions usually take 512×512. The manager:
   1. crops around the mask with padding
   2. scales
   3. runs the model
   4. scales back
   5. blends only inside the (feathered) mask
7. With the manager closed, PhotoCraft behaves as before and shows a clear message.
8. The model's exact weights, ONNX file and license have been verified and recorded.
9. **Tests:**
   - protocol tests (auth refused without token; cancel; bad input)
   - a fixed test image with a mask whose result is compared within a tolerance
   - PhotoCraft's existing test suite still passes

**Not in Milestone 1:** catalog UI, GPU, other tasks, ComfyUI, web.

### Step 2: the manager grows up

- Model catalog with versions, licenses, sizes, SHA-256, update checks, disk usage, and delete/redownload.
- New tasks:
  - **Select subject** (SAM2 or MobileSAM; returns a mask/selection)
  - **Remove background** (BiRefNet; returns a mask or a cut-out layer)

### Step 2.b: CUDA

- **ONNX Runtime CUDA execution provider for the built-in models.** Device order for "Auto": CUDA (NVIDIA card with a supported driver), then DirectML (other Windows GPUs), then CPU.
- **Any GPU start failure → the job runs on the CPU,** and the result reports which device ran it.
- **Free-VRAM check before every GPU job.** Falls back to the CPU if there isn't enough. This is needed on the 4090 PC, where Strata can hold almost all of the card's memory.
- **CUDA libraries are an optional download** (about 1–2 GB), fetched only when CUDA is turned on and checksum-verified, like the models. They aren't bundled in the installer.
- **Per-task device setting:** Auto / CUDA / CPU.
- **Tests compare GPU and CPU results within a tolerance.**
- **Acceptance:** the same three tasks run on the 4090, Auto chooses CUDA, and forcing the CPU still works. Select subject is clearly faster on the GPU (measure and record it).

### Step 3: ComfyUI backend (optional)

- Tasks: generative fill (prompt + mask) and expand canvas.
- It points at an existing ComfyUI, local or on another machine. The manager ships one workflow template per task.
- It never installs ComfyUI custom nodes automatically.
- It can reuse an existing model folder (for example Stability Matrix's) instead of downloading again.
- Models used through it follow the same commercial-safe rule.

### Step 4: web services and the model-mapping screen

- The user brings their own API keys, stored in the system keychain.
- An "uploads to X" notice is shown before every web job.
- Settings for which model and provider each tool uses.

### Later

- Upscale (Real-ESRGAN)
- Mac (CoreML execution provider)
- Other Craft apps
- Offering the thin hook upstream as a PR

## Candidate models (verify before use)

| Task | Candidate | License to confirm |
|---|---|---|
| Fill masked area | LaMa (big-lama), ONNX conversion | Code Apache-2.0; **confirm the weights' terms** |
| Select subject | SAM2 / MobileSAM | Apache-2.0 / check |
| Remove background | BiRefNet | MIT; check the specific checkpoint |
| Upscale | Real-ESRGAN | BSD-3; check the specific weights |

Excluded by the commercial-safe rule: RMBG (non-commercial), FLUX.1-dev (non-commercial).

## Security rules

- The manager listens on loopback only, needs a token, and sends no telemetry.
- Downloads come only from catalog URLs and are SHA-256 checked. Unknown files are refused.
- No pickle formats. ONNX and safetensors only.
- PhotoCraft never talks to the internet for AI. Only the manager does, and only for catalog downloads or explicitly allowed web jobs.
- Agents run in the CraftML account. They don't get admin rights or secrets, and never push to `main` (branch protection, PRs only).
- Agents never read stored credentials (`git credential fill`, Windows Credential Manager, browser stores, token files) and never pass them to other tools. GitHub work goes through `gh` signed in with the fine-grained token (TASKS CN-00). If a step needs a credential the agent doesn't have, it stops and says so.

## How the agents work

- **Repos:** `erikerikerik/photocraft` (fork) and a new repo for the manager.
- **Shared files:** each repo has `agents/` with SPEC.md (this file), TASKS.md and HANDOFF.md.
- **TASKS.md:** one line per task, with status, owner and the test command that must pass.
- **HANDOFF.md:** where the last agent stopped and what's next. It's updated at the end of every session.
- **Branches:** one per task (`ai/<task>`). Nothing merges without passing tests and a review by Claude or Codex.
- **Local agent (Strata):** good for tests, ONNX conversion and benchmark scripts, docs, and small well-specified changes. Not for architecture or cross-crate changes.
- **Before touching PhotoCraft:** read its `AGENTS.md`, `docs/architecture.md`, `docs/roadmap.md`, `docs/plugins.md` and `docs/control-protocol.md`.

## Open questions

1. The manager's real name, and its repo name.
2. Whether to also open an issue on storytold/photocraft describing the hook, even if they're quiet.
3. Starting the manager: on demand from PhotoCraft, or always in the tray?

## v1.1 amendments (Oct 13, 2026, Claude, after reading the PhotoCraft source)

These refine "PhotoCraft fork: what changes". Where this section and the text above disagree, this section wins.

1. **The seam is a provider trait in the engine.** Upstream issue [#41](https://github.com/storytold/photocraft/issues/41) (Generative AI editing, open, no comments) asks for "a backend behind a provider interface" that stays off the wasm/offline path. So:
   - `crates/engine/src/ai.rs` (L5) defines an `AiProvider` trait (`tasks`, `submit`, `poll`, `result`, `cancel`) and plain data types. No network, no new dependencies, wasm-safe. `Session` holds an optional `Arc<dyn AiProvider + Send + Sync>` (`Session::set_ai_provider`). With none, AI commands return "Craft AI Manager isn't running".
   - `crates/ai-client` = `photocraft-ai-client` (L6, native only) implements `AiProvider` against the job protocol. Register it in `xtask/src/layers.rs` as `Class::Layer(6)`. The wasm set is L0–L5 plus ui-egui only, so it isn't checked for wasm. `members = ["crates/*", ...]` picks it up automatically.
   - `apps/photocraft` and `apps/photocraft-cli` inject it at startup.
   - The same trait is where upstream's planned in-process `photocraft-ml` (L4, `docs/architecture.md` §10, gap G15) could plug in later, so a PR doesn't fight their design.
2. **Reuse Content-Aware Fill.** `crates/engine/src/edit_menu_cmds.rs` › `content_aware_fill_as` already reads the selection window, runs as a background job (`jobs::run`: progress, cancel, document locked) and `apply_content_aware_fill` writes the result to a new layer (`output: "new"`) as one undo step. The AI command reuses both; only the fill step changes (provider instead of PatchMatch). Refactor minimally into a shared helper; don't copy the code.
3. **Command id (decided Oct 13: Photoshop's names).** Photoshop has Edit › Generative Fill… (an empty prompt removes the object). Proposed: `edit.generativeFill`, added to `crates/ui-egui/src/menu_catalog.rs` under Edit. Params `{ "prompt"?: string, "sampleAllLayers"?: bool }`; empty or missing prompt → task `fill_masked_area`. Result layer named "Generative Fill" (Photoshop's name) instead of "Fill (AI)". Erik confirmed this (D1).
4. **Colour scope for M1.** RGB documents at 8/16/32-bit. Gray, CMYK, Lab and indexed return a clear "not supported yet" error, never a silent sRGB assumption. The client sends 8-bit PNG for 8-bit documents and 16-bit PNG otherwise (32-bit is clamped, with a note in the result).
5. **No new HTTP dependency in PhotoCraft.** The client uses `std::net::TcpStream` to 127.0.0.1 only (HTTP/1.1, Content-Length bodies, no TLS on loopback), like `apps/photocraft/src/control_server.rs` does on the server side. PNG through the `png` crate already in `Cargo.lock`. Any new dependency needs the justification `book/src/development/dependencies.md` asks for.
6. **MCP comes free.** Because it's a command, `command_run edit.generativeFill` and `jobs_list` / `jobs_cancel` work over MCP and the control channel. Add one automation test.
7. **`agents/` stays off upstream PR branches.** It lives on the fork's `main` only. PR branches for upstream are cut from `upstream/main` with the hook commits cherry-picked.
8. **LaMa candidate found** (M1 item 8 still open): Hugging Face `Carve/LaMa-ONNX` › `lama_fp32.onnx`, revision `a3ee2fca54baebec351b8fa7786154ffa7555aa6`, about 208 MB, fixed 512×512, opset 17, tagged Apache-2.0, a port of big-lama. LFS SHA-256 as shown on the page: `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6`. Re-check it on download before it goes into the catalog. **Licence note:** big-lama's code and published weights are Apache-2.0, but it was trained on Places365, whose image terms are for non-commercial research. Erik's decision (D2, Oct 13): accept for M1, record the training-data note in the catalog entry, revisit before any commercial release.
9. **Repo line endings.** The clone uses Windows `core.autocrlf=true`. Agents commit from Windows Git only; Git in a Linux VM over the mounted folder reports every file as modified.
10. **Name decided (Oct 13):** the manager is **Craft AI Manager**, short **CAIM**; repo `erikerikerik/craft-ai-manager`. This answers open question 1. PhotoCraft's messages call it "Craft AI Manager" (e.g. "Craft AI Manager isn't running"). `photocraft-ml` stays upstream's name for their planned in-process crate; we don't use it.
11. **Job protocol v1, wire details (Oct 13, before PC-03 and CAIM-01).** Both sides build to exactly this; a change needs a SPEC edit first.
    - **Transport:** HTTP/1.1 to `127.0.0.1:<port>` only. Every request has `Authorization: Bearer <token>`, `Host: 127.0.0.1:<port>`, `Connection: close` and a `Content-Length` (no chunked bodies either way). Discovery as above: `%LOCALAPPDATA%\CraftAI\endpoint.json` (macOS `~/Library/Application Support/CraftAI/`, Linux `$XDG_DATA_HOME/CraftAI/` or `~/.local/share/CraftAI/`), overridable with the `CRAFT_AI_ENDPOINT` environment variable (a path to the file) for tests. The client refuses any host other than 127.0.0.1.
    - **Errors:** a JSON body `{"error": "<plain sentence>"}` with 400 bad input, 401 missing or wrong token, 404 unknown job, 409 result not ready, 413 too large, 500 internal.
    - `GET /v1/health` → 200 `{"protocol": 1, "app": "caim/<version>", "devices": ["cpu", …]}`
    - `GET /v1/tasks` → 200 `{"tasks": [{"id": "fill_masked_area", "needs_prompt": false, "max_pixels": <n>}]}`
    - `POST /v1/jobs` → 202 `{"id": "<opaque>"}`. Body `multipart/form-data` with three parts, in this order:
      - `job` (`application/json`): `{"task": "fill_masked_area", "image_size": [w, h], "region": [x, y, w, h], "bit_depth": 8 | 16, "channels": 3 | 4, "prompt": null, "settings": {}, "provider": "auto", "client": "photocraft/<version>"}`. `region` is the window in document coordinates; `image_size` equals its width and height.
      - `image` (`image/png`): the window, RGB or RGBA, 8-bit for 8-bit documents and 16-bit otherwise, in the document's colour space (no conversion). f32 → integer: clamp 0–1, multiply by 255 or 65535, round.
      - `mask` (`image/png`): 8-bit grayscale, same size; 255 = fully selected, 0 = keep, values between are soft coverage.
    - `GET /v1/jobs/{id}` → 200 `{"state": "queued" | "running" | "done" | "failed" | "cancelled", "progress": 0–1, "message": "<text>"}`
    - `GET /v1/jobs/{id}/result` → 200 `{"model": "…", "device": "cpu" | "cuda" | …, "provider": "local" | "<service>", "elapsed_ms": <n>, "image_size": [w, h]}`; 409 while not done.
    - `GET /v1/jobs/{id}/result.png` → 200 `image/png`: same size, bit depth and channels as the `image` part. Pixels outside the mask equal the input.
    - `DELETE /v1/jobs/{id}` → 204 (also for a job that already finished).
    - **Client limits:** connect timeout 2 s, read timeout 30 s per request, JSON bodies ≤ 64 KiB, PNG bodies ≤ 1 GiB and ≤ the request's pixel count; anything else is an error, never a panic.
