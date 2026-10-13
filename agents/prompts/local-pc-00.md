# Prompt for the local agent: PC-00 (baseline build, no changes)

Run from `E:\CraftML\work\photocraft` with `codex --profile strata`. Paste everything below the line.

---

You are working in the PhotoCraft repository (a Rust image editor). Read `AGENTS.md` sections 1, 2, 5 and 6, and `agents/SPEC.md` (only "How the agents work"), before doing anything.

Task PC-00 from `agents/TASKS.md`: build and test the repository **without changing any source file**.

Rules:
- Do not edit, create or delete any file outside `agents/HANDOFF.md` and `target/agent-local/`.
- Use `CARGO_TARGET_DIR=target/agent-local` for every cargo command (PowerShell: `$env:CARGO_TARGET_DIR = "target/agent-local"`).
- Do not run `git commit`, `git push`, `git checkout` or `git reset`.
- If a command fails, do not try to fix the code. Record the error and continue with the next step.

Steps, in order. Time each one (`Measure-Command { ... }` or note start/end times):
1. `rustc --version` and `cargo --version`. The workspace needs Rust 1.95 or newer; stop and report if it's older.
2. `cargo xtask layers`
3. `cargo build -p photocraft -p photocraft-cli`
4. `cargo test -p photocraft-engine --lib --tests`

When done, add an entry at the top of `agents/HANDOFF.md` under a heading `## <date>: local agent (PC-00)` with:
- each command, pass or fail, and how long it took
- the number of tests passed / failed / ignored from step 4
- for any failure: the first 30 lines of the error, and your one-line guess at the cause (missing tool, Windows-only issue, real test failure)
- free disk space left on E:

Then set PC-00's status in `agents/TASKS.md` to `done` or `blocked: <reason>`. Change nothing else in that file.
