# TASKS: PhotoCraft AI, Milestone 1

Read `agents/SPEC.md` (including the v1.1 amendments) and `agents/FEATURES.md` (the master checklist and its rules) first. Tasks here are work units; FEATURES.md says what counts as done. One row per task. Update the status when you start (`doing`) and finish (`done`, `blocked: why`). Branch per task: `ai/<id>-<short-name>`, e.g. `ai/pc-01-provider-trait`.

Status values: `todo`, `doing`, `review`, `done`, `blocked`. Owners: **Erik**, **Codex** (cloud), **local** (Codex + Strata on CraftML), **Claude** (plan/review).

Every PhotoCraft task also passes the AGENTS.md §5 gate for the crates it touches: `cargo test -p <crates>`, `cargo clippy -p <crates> --all-targets -- -D warnings`, `cargo xtask layers`. Use `CARGO_TARGET_DIR=target/agent-<owner>`.

## Decisions for Erik

| ID | Status | Question | Recommendation |
|---|---|---|---|
| D1 | done | Command id and layer name. **Decided (Erik, Oct 13): Photoshop's names** — Edit › Generative Fill…, `edit.generativeFill`, layer "Generative Fill". | Unblocks PC-02. |
| D2 | done | big-lama weights. **Decided (Erik, Oct 13): accept for now** for M1; the catalog entry records the Places365 training-data note; revisit before any commercial release. | Unblocks MG-02. |
| D3 | done | Manager's name: **Craft AI Manager (CAIM)**. Repo name: `erikerikerik/craft-ai-manager` unless Erik picks another. | Unblocks MG-00. |
| D4 | todo | Comment on upstream #41 describing the provider-trait hook? | Yes, short, once PC-02 works on the fork, so it shows working code. |
| D5 | todo | Start the manager on demand or in the tray? | M1: started by hand; PhotoCraft only shows "not running". Decide in step 2. |

## Machine setup (Erik, admin account unless noted)

| ID | Status | Task | Check |
|---|---|---|---|
| S1 | done | Visual Studio Build Tools (C++ workload) | `cargo build -p photocraft` links in CraftML |
| S2 | ? | `ConsentPromptBehaviorUser = 0` (auto-deny elevation) | `reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System" /v ConsentPromptBehaviorUser` shows `0x0` |
| S3 | ? | D: drive: deny CraftML if it holds anything personal | `dir D:\Users` as CraftML → Access denied |
| S4 | partial | Fork `erikerikerik/photocraft` exists and is cloned in `E:\CraftML\work\photocraft` (done). Still: fine-grained token limited to the fork, branch protection on `main`. | `git push origin HEAD:main` from CraftML is refused |
| S5 | doing | Strata `--vram-reserve-mib`: 1200 → 1728 helped (idle free 662 → 1311 MiB; lowest during requests 0 → 875 MiB) but still dipped under 1024 twice, so Claude set **1920** (backup `strata-iq3_s.json.bak-1200`). Erik: restart Strata once more. | No `WARNING: only … MiB of VRAM free` in `strata-iq3_s.log` after 5 agent turns |
| S6 | done | Rust ≥ 1.95 in CraftML (workspace `rust-version`): 1.99.0 (PC-00) | `rustc --version` |
| S7 | todo | Turn on GitHub Actions for the fork (Settings › Actions), so PRs get CI runs that receipts can link | A PR shows checks |

## PhotoCraft fork (`erikerikerik/photocraft`)

| ID | Status | Owner | Task | Depends | Test that must pass |
|---|---|---|---|---|---|
| PC-00 | done | local | Baseline: build and test the fork unchanged. Record build time, test time and any failures in `agents/HANDOFF.md`. No code changes. | S1, S6 | `cargo build -p photocraft -p photocraft-cli`; `cargo xtask layers`; `cargo test -p photocraft-engine --lib --tests` |
| PC-01 | done | local | `crates/engine/src/ai.rs`: `AiProvider` trait (`tasks`, `submit`, `poll`, `result`, `cancel`), request/result types (task, window RGBA f32 + `PixelFormat`, coverage mask, region, prompt), `AiError`. `Session::set_ai_provider(Option<Arc<dyn AiProvider + Send + Sync>>)`. A `FakeProvider` for tests (fills with a solid colour, can be told to fail, stall or report progress). | PC-00 | `cargo test -p photocraft-engine --lib ai`; `cargo xtask wasm` |
| PC-02 | todo | Codex | Command `edit.generativeFill`: refactor `content_aware_fill_as` / `apply_content_aware_fill` so the fill step is pluggable, then add the AI variant that calls the provider inside `jobs::run` (poll loop checks `ctx.cancelled()` and calls `cancel`). Always writes a new layer. Catalog entry under Edit. Tests: new layer named per D1; original pixels unchanged; one undo removes it; no selection → `Err`; no provider → `Err` with "Craft AI Manager isn't running"; cancel → document unchanged; non-RGB → `Err`; provider returns wrong size → `Err`. | PC-01, D1 | `cargo test -p photocraft-engine --lib generative`; `cargo xtask parity` (commit `docs/parity-checklist.md`); `cargo test -p photocraft-engine --test panic_hunt -- --ignored` |
| PC-03 | todo | Codex (tests: local) | New crate `crates/ai-client` (`photocraft-ai-client`, L6, native only): read `%LOCALAPPDATA%\CraftAI\endpoint.json`; HTTP/1.1 over `std::net::TcpStream` to 127.0.0.1 only; bearer token; multipart upload of `job.json` + `image.png` + `mask.png`; poll, result, DELETE on cancel; timeouts; response size caps. Register in `xtask/src/layers.rs`. Tests against an in-process fake server (`std::net::TcpListener`). | PC-01 | `cargo test -p photocraft-ai-client`; `cargo xtask layers`. Tests cover: token sent; 401 → clear error; manager absent → "not running"; oversize/malformed reply → `Err`; non-loopback address in endpoint.json → refused |
| PC-04 | todo | Codex | Inject the client in `apps/photocraft` and `apps/photocraft-cli` at startup (re-read `endpoint.json` per job, since the token changes on each manager start). | PC-02, PC-03 | `cargo build -p photocraft -p photocraft-cli`; manual: manager closed → Edit › Generative Fill shows the message, nothing crashes |
| PC-05 | todo | local | Automation test: `command_run edit.generativeFill` and `jobs_cancel` over MCP with `FakeProvider`. | PC-02 | `cargo test -p photocraft-automation` |
| PC-06 | todo | Claude + Erik | End-to-end on the 4090 PC against the real manager (CPU). Screenshot via `cargo run -p photocraft-ui-egui --example snapshot` or `--control` + `ui.screenshot`. Walk the M1 "done when" list. | PC-04, MG-05 | M1 checklist in SPEC, all 9 items |
| PC-07 | todo | local | `agents/run-guards.ps1`: read `agents/guards.txt`, run each command for this repo, print a pass/fail table, exit non-zero on any failure. | PC-00 | Script runs the 3 BASE guards and reports them |

## Craft AI Manager (CAIM, new repo `craft-ai-manager`)

| ID | Status | Owner | Task | Depends | Test that must pass |
|---|---|---|---|---|---|
| MG-00 | todo | Erik + Codex | Create the repo with branch protection. Rust workspace, CI (`fmt --check`, `clippy -D warnings`, `test`), the same no-panic clippy lints as PhotoCraft, `agents/` with SPEC, TASKS, HANDOFF. | D3 | CI green on an empty `main` |
| MG-01 | todo | Codex | Server: bind `127.0.0.1:0`; random token per start; write `%LOCALAPPDATA%\CraftAI\endpoint.json` readable by the current user only; `/v1/health`, `/v1/tasks`, the jobs API from SPEC; multipart with caps (body size, pixel count). | MG-00 | Protocol tests: no token → 401; wrong token → 401; bad multipart → 400; unknown job → 404; cancel → `cancelled`; oversize → 413 |
| MG-02 | todo | Codex | Model store: catalog embedded as TOML (id, url, revision, sha256, size, license, licence note); download to `%LOCALAPPDATA%\CraftAI\models\<id>\`; streaming SHA-256; refuse a mismatch and anything that isn't `.onnx` / `.safetensors`; no network once present. | MG-00, MG-03, D2 | Tests with a local fixture file: good hash → ready; bad hash → deleted and `Err`; second run makes no request |
| MG-03 | done | local | Verify LaMa: download `lama_fp32.onnx` at revision `a3ee2fca…` (SPEC v1.1 §8), compute SHA-256, dump input/output names, shapes, dtypes and value ranges, run it once on a test image + mask, time it on the CPU (7950X). Write `E:\CraftML\work\lama-check\lama.md` with the results and the licence note. **Result:** `agents/models/lama.md` (SHA-256 matches; image 0–1, mask 1 = fill, output 0–255; 1.2 s per 512×512 on the CPU). Moves to the CAIM repo as `docs/models/lama.md` once MG-00 exists. Script, not app code; no pickle loading. | none | Script exits 0; SHA-256 printed matches the page or the mismatch is reported |
| MG-04 | todo | Codex | ONNX backend with the `ort` crate, CPU execution provider. Tensor facts in `agents/models/lama.md`. LaMa runner: crop the mask bounding box plus padding (max of 32 px or 25% of the box), resize to 512×512, normalise per MG-03, run, resize back, feather the mask a few pixels, blend only inside it. | MG-01, MG-02, MG-03 | Golden test: a public-domain photo with a real object masked, result within tolerance; pixels outside the mask identical to the input |
| MG-05 | todo | Codex | Job runner: one worker, queue, progress stages (download, prepare, run, finish), cancel between stages (and `ort` run termination if available). | MG-04 | Tests: cancel while queued and while running → `cancelled`; two jobs run in order |
| MG-06 | todo | Claude | Review MG and PC branches against SPEC before each merge. | each PR | Review notes in the PR |

## Conductor (`agents/CONDUCTOR.md`)

| ID | Status | Owner | Task | Depends | Test that must pass |
|---|---|---|---|---|---|
| CN-00 | todo | Erik | GitHub access: (1) give the Claude GitHub connection write access to the fork (Contents + Pull requests); (2) create a fine-grained token for the CraftML account limited to the fork, `craft-ai-manager` and `craft-conductor` (Contents + Pull requests read/write); (3) install GitHub CLI for CraftML and run `gh auth login --with-token`. Covers S4's token. | none | `gh pr list -R erikerikerik/photocraft` works in CraftML; Claude can open a PR on the fork |
| CN-01 | todo | Erik | Install Claude Code in the CraftML account and sign in. | none | `claude -p "reply with ok"` prints ok |
| CN-02 | todo | Erik + Claude | Create the private repo `erikerikerik/craft-conductor` and copy `CONDUCTOR.md` into it. | CN-00 | Repo exists |
| CN-03 | todo | Codex | Conductor v0, part 1: loop, kill switch, read `TASKS.md`, pick ready tasks, worktree per task, run `codex exec` (local or cloud profile), run tests + guards, push, open PR, labels, retries, logs, `status.md`. **No Claude step yet.** | CN-00, CN-02, PC-07 | Dry run (`-DryRun`) prints the plan for PC-00 without running it; a real run on PC-00 opens a PR with a receipt |
| CN-04 | todo | Codex | Part 2: review step with `claude -p` (diff + receipt + rules), comment, `ai:approved` / `ai:changes`, send changes back to the author. | CN-01, CN-03 | On a test PR: Claude comments, the label changes, the author agent gets the comments |
| CN-05 | todo | local | Part 3: budgets in `conductor.json`, usage-limit detection and "cooling", usage log. | CN-03 | A budget of 0 stops dispatching; a fake limit message marks the agent cooling |
| CN-06 | todo | Claude | Review the conductor code before it runs unattended. | CN-04, CN-05 | Review notes in the PR |

## Order

1. Now, in parallel: **PC-00** and **MG-03** (local agent), D1–D3 are decided, **S1–S6** (Erik).
2. Then **PC-01** and **MG-00 → MG-01** (Codex).
3. Then **PC-02, PC-03** and **MG-02, MG-04, MG-05**.
4. Then **PC-04, PC-05**, and **PC-06** to close Milestone 1.

**Conductor track:** CN-00 and CN-01 first (Erik). CN-03 can start as soon as PC-07 (the guard script) exists. Until the conductor runs, tasks are started by hand with the prompts in `agents/prompts/`.
