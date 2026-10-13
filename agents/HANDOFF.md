# HANDOFF

Update this at the end of every session: newest entry on top, keep it short.

## 2026-10-13: local agent (PC-00)

Baseline build and test of the fork **unchanged** (PC-00). No source file touched; `CARGO_TARGET_DIR=target/agent-local`, cold build (no `target/` before, empty `~/.cargo/registry`).

| Step | Command | Result | Time |
|---|---|---|---|
| 1 | `rustc --version`, `cargo --version` | 1.99.0 / 1.99.0, newer than the 1.95 the workspace wants (TASKS S6 can be `done`) | - |
| 2 | `cargo xtask layers` | pass | 12.3 s (includes the crates.io index and the `craft-libs` git fetch) |
| 3 | `cargo build -p photocraft -p photocraft-cli` | pass, 0 warnings, 0 errors | 2 m 31 s |
| 4 | `cargo test -p photocraft-engine --lib --tests` | pass | 3 m 02 s |

**Step 4 numbers:** 1771 passed, 0 failed, 19 ignored, over 54 test binaries (1 lib unittest binary + 53 integration test files; doc-tests excluded by `--lib --tests`). The lib binary alone: 1599 passed, 0 failed, 13 ignored in 15.2 s.

**Artifacts:** `target/agent-local/debug/photocraft.exe` (85 MiB), `target/agent-local/debug/photocraft-cli.exe` (54 MiB). The MSVC link worked, so TASKS S1 looks satisfied.

**Disk:** E: free 851.9 GB after the run (869.3 GB before); `target/agent-local` is 18.0 GB.

**Notes**
- Ran on `99e11388`; the `ai/agents-setup` PR merged into `main` mid-session (`0d48ca1e`), which adds only `agents/` files, so these numbers hold for the source at both revs.
- No failures, so no error excerpts. Everything downloaded on first use: crates.io plus the private `storytold/craft-libs` git dep at both pins, so network and GitHub access from CraftML are fine.
- The first attempt at step 2 ran inside the agent's network sandbox and stopped at "can't checkout from 'https://github.com/storytold/craft-libs': you are in the offline mode (--offline)". Environment artifact of the sandbox, not the machine; re-run with network access it passed. Future local agents need network for their first cargo command.
- `corpus/` is absent, so corpus-gated tests were skipped as designed; the 19 ignored are the usual opt-in ones, not failures.

**Next:** PC-01 (provider trait) is unblocked. A warm `target/agent-local` (18 GB) is on disk for reuse.
## 2026-10-13: Claude (planning)

**State**
- Fork cloned at `E:\CraftML\work\photocraft`, `main` = `origin/main` @ `99e11388` (upstream `storytold/photocraft` added as remote `upstream`, not fetched yet). Clean under Windows Git (`core.autocrlf=true`).
- `agents/` created: SPEC (v1 + v1.1 amendments), FEATURES (master checklist), TASKS, guards.txt, receipts/TEMPLATE.md, prompts/ and this file. Pushed by Claude through the GitHub connector to branch `ai/agents-setup` with a PR into `main`. After the PR merges, run `git pull` in the local clone (the local copy is untracked and identical; delete it or let Git overwrite it).
- Decided: D1 Photoshop names (`edit.generativeFill`, layer "Generative Fill"); D2 accept big-lama for M1 with the training-data note; D3 Craft AI Manager (CAIM), repo `craft-ai-manager` (not created yet).
- No code changes in PhotoCraft yet.

**What I learned from the source (details in SPEC v1.1)**
- Upstream issue #41 asks for an AI backend "behind a provider interface", so the seam is an `AiProvider` trait in the engine plus a separate L6 client crate.
- Background jobs (`crates/engine/src/jobs.rs`: `jobs::run`, `JobCtx`, progress, cancel) and Content-Aware Fill's window + "output to new layer" path (`edit_menu_cmds.rs`) already exist; Generative Fill reuses them.
- Preferences › Integrations exists (`prefs.rs` › `Integrations`) but is hidden until implemented; leave it alone in M1.

**Strata** (`F:\CraftML\strata\...\strata-iq3_s.log`): no requests have run since the restart at 1200 MiB reserve (Oct 12, 21:27). Idle free VRAM is 662 MiB, already under the 1024 MiB warning line, and every earlier run dropped to 0 MiB on its second request. Claude set it to 1728 (backup `strata-iq3_s.json.bak-1200`). Restart Strata for it to take effect, then check the log after a few agent turns (TASKS S5).

**Conductor** designed in `agents/CONDUCTOR.md` (our own dispatcher on the CraftML PC, GitHub as the message board, Claude both in cloud sessions and as Claude Code on the PC, token budgets). Tasks CN-00 to CN-06 in TASKS.

**Next**
1. Erik: CN-00 (GitHub access) and CN-01 (Claude Code in CraftML); merge the `ai/agents-setup` PR; restart Strata; S1–S4, S6, S7 in `agents/TASKS.md`.
2. Local agent: PC-00 (prompt in `agents/prompts/local-pc-00.md`), then MG-03 (`agents/prompts/local-mg-03.md`).
3. Codex: PC-01 once PC-00 is green.
