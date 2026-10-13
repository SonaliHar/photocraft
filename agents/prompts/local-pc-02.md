# Prompt for the local agent: PC-02 (Edit › Generative Fill command)

Run in a fresh worktree with `codex --profile strata` (see the commands in `agents/HANDOFF.md`). Paste everything below the line.

---

You are working on a fork of PhotoCraft, a Rust image editor. Before writing code, read: `AGENTS.md` (all of it, especially §3 rules 1, 2, 9 and §5), `agents/SPEC.md` › "v1.1 amendments" §2–§4, the PC-02 row in `agents/TASKS.md`, the "Review" section of `agents/receipts/PC-01.md`, and `crates/engine/src/ai.rs` (the provider interface you will call).

**Task PC-02:** add the command `edit.generativeFill`. It fills the selection through the AI provider and puts the result on a **new layer named "Generative Fill"**, as one undo step, in a background job. No network code, no UI code beyond the menu catalog line, no new dependencies.

Work on branch `ai/pc-02-generative-fill` with `CARGO_TARGET_DIR=target/agent-local`.

**How to build it (reuse, don't copy):** `crates/engine/src/edit_menu_cmds.rs` already has Content-Aware Fill:
- `content_aware_fill_as(s, p, cmd, label)` reads the selection, picks the window (`crate::fill_cmds::sampling_window(hb, doc.bounds())`), reads the window pixels and the selection coverage on a worker via `crate::jobs::run`, and fills with PatchMatch;
- `apply_content_aware_fill(...)` writes a `FilledWindow` to the layer, a new layer (`output: "new"`) or a duplicate, as one undo step. The new layer's name comes from `doc.next_layer_name("Content-Aware Fill")`.

Refactor the smallest amount needed so the AI command can reuse the window reading and the apply step: for example, a shared helper that reads `(window, img, coverage, original, cover, bounds)` and a `layer_name` parameter for the apply step. Content-Aware Fill's behaviour and all its existing tests must not change.

**The AI command:**
1. New module `crates/engine/src/generative_cmds.rs` with `specs()` (AGENTS.md §6: new commands go in a new module), registered in `commands.rs`. Label "Generative Fill…", menu Edit, params doc `{"prompt":string?,"sampleAllLayers":bool=false}`.
2. Add `(&["Edit"], "Generative Fill…", None, "edit.generativeFill"),` to `crates/ui-egui/src/menu_catalog.rs` right after the Content-Aware Fill line.
3. `enabled`: a document, an unlocked pixel layer and a non-empty selection. It stays enabled when no provider is set, so the user gets the message instead of a greyed-out item.
4. `run`, in this order, each failure an `Err` with a plain sentence:
   - no provider (`s.ai_provider()` is `None`) → `AiError::NoProvider` ("Craft AI Manager isn't running");
   - a non-empty `prompt` → "Prompts aren't supported yet; leave the prompt empty to remove the selection" (M1 has only `fill_masked_area`);
   - document colour model not RGB → "Generative Fill works on RGB documents for now";
   - no selection, or selection outside the canvas → same messages as Content-Aware Fill.
5. In the `jobs::run` worker: build an `AiRequest` (task `FillMaskedArea`, the window, the window pixels and `PixelFormat`, the coverage, `prompt: None`, `client_version` from `crate::build_info` or the crate version), call `crate::ai::run_to_completion(provider.as_ref(), &req, ctx, Duration::from_millis(100))`, then **reject any non-finite pixel in the result** (review follow-up 1) and hand the pixels to the shared apply step with output "new" and layer name "Generative Fill".
6. Cancel must leave the document untouched (it will: `jobs::run` discards the result), and the label of the job is "Generative Fill".

**Tests** (unit tests in `crates/engine/src/generative_cmds/tests.rs` or similar; use or expose PC-01's fake provider, for example by moving it to a `#[cfg(test)] pub(crate) mod testing` in `ai.rs`):
- new layer named "Generative Fill" above the active layer; the original layer's pixels identical before and after (B01);
- one `undo` removes the new layer (B02);
- pixels outside the selection are transparent on the new layer; a feathered selection blends by coverage (B11);
- no selection → `Err`; no provider → the exact "Craft AI Manager isn't running" text (B04); non-empty prompt → `Err`;
- a Gray, a CMYK and a Lab document → `Err`, at 8 and 16 bit (B10); RGB works at 8, 16 and 32 bit;
- provider failure, wrong-size result and a NaN pixel in the result → `Err`, document unchanged;
- cancel during the job → document unchanged (B03);
- Content-Aware Fill's existing tests still pass unchanged.

**Done when all of these pass** (paste the summary lines into the PR):
```
cargo test -p photocraft-engine --lib generative
cargo test -p photocraft-engine --lib edit_menu
cargo clippy -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo xtask parity        # commit the regenerated docs/parity-checklist.md
cargo test -p photocraft-engine --test panic_hunt -- --ignored
```
Then run every line of `agents/guards.txt`, add `PC-02 | photocraft | cargo test -p photocraft-engine --lib generative` to it, fill in `agents/receipts/PC-02.md` from the template, set PC-02 to `review` in `agents/TASKS.md`, add a short entry at the top of `agents/HANDOFF.md`, **commit everything**, push the branch and open a PR titled `[PC-02] Edit › Generative Fill command`. Check with `git status` that nothing is left uncommitted before you say you're done. Don't merge, don't touch `main`.
