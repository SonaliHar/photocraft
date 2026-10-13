# HANDOFF

Update this at the end of every session: newest entry on top, keep it short.

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
