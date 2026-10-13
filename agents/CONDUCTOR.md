# Conductor: design v0

Status: agreed direction, Oct 13, 2026 (Erik + Claude). Built by Codex, reviewed by Claude.

The conductor is a small program on the CraftML PC that hands tasks to Codex, Strata (local) and Claude, runs the tests, and moves work along through GitHub. **It contains no AI itself**: polling GitHub and reading `TASKS.md` costs no tokens. The models only run when there's work for them.

## How the agents talk

They don't chat directly. **GitHub is the message board:**

- **Work to do:** rows in `agents/TASKS.md` on `main` (status `todo`, owner `local` or `Codex`, every dependency `done`).
- **Work in progress:** one branch and one PR per task (`ai/<id>-<name>`, PR title `[<ID>] …`).
- **Conversation:** PR comments. Each starts with who wrote it: `**[claude]**`, `**[codex]**`, `**[local]**`, `**[conductor]**`, `**[erik]**`. The conductor passes the whole thread to whichever agent acts next.
- **Where a PR stands:** labels, set only by the conductor and Erik:

| Label | Meaning | Next |
|---|---|---|
| `ai:running` | An agent is working on the branch | Conductor waits for it |
| `ai:red` | Tests or guards failed | Conductor sends the failure back to the same agent (2 retries max, then `ai:blocked`) |
| `ai:needs-review` | Green: tests, guards and the AGENTS.md gate pass | Claude reviews |
| `ai:changes` | Claude asked for changes | Conductor sends the review comments back to the author |
| `ai:approved` | Claude approved | **Erik merges.** v0 never merges anything itself |
| `ai:blocked` | Needs a human | Erik |

## The loop (every 5 minutes)

1. `git fetch`; read `agents/TASKS.md` from `origin/main`.
2. Pick at most one ready task per agent (only **one local job at a time**: Strata uses the whole GPU).
3. Make a git worktree for it: `E:\CraftML\work\wt\<id>` on branch `ai/<id>-<name>` from `origin/main`. Agents never share a folder.
4. Build the prompt from a template: the task row, "read AGENTS.md, agents/SPEC.md, agents/FEATURES.md first", the rules, and "write the receipt, update HANDOFF".
5. Run the agent with a time limit and save the output to `E:\CraftML\conductor\logs\<id>-<time>.log`:
   - owner `local` → `codex exec --profile strata …`
   - owner `Codex` → `codex exec …` (the cloud model)
6. Run the task's test command and `agents/run-guards.ps1` itself, and write the results into the receipt. The agent's own claims don't count.
7. Push the branch, open or update the PR, set the label.
8. For `ai:needs-review` PRs, run Claude Code headless (`claude -p`) with the **review prompt**: the diff, the receipt, the matching SPEC and AGENTS.md rules, and the task's acceptance tests. Post its answer as a `**[claude]**` comment and set `ai:approved` or `ai:changes`.
9. Write a status table to `E:\CraftML\conductor\status.md` (what's running, waiting, blocked, today's usage).

## Saving tokens

- **No AI while idle.** The conductor polls; models run only for a task or a review.
- **Local first (Erik, Oct 13).** Every task goes to the local agent (Codex + Strata) unless it's on the short list below. Cloud tokens are limited, so cloud Codex is the fallback, not the default:
  - **Cloud Codex** only when the local agent has failed the same task twice (`ai:red` after its retries, or Claude's review finds it can't converge), or for a change Erik marks "cloud".
  - **Claude** for planning, reviews, and **UI design** (layout, interaction, look; FEATURES §3). Claude writes the UI spec and checks the snapshots; the local agent writes the code.
- **Claude reviews green PRs only**, and sees only the diff plus the rules that apply, not the whole repo.
- **Daily budgets** in `conductor.json`: `codex_cloud_runs_per_day`, `claude_reviews_per_day`, `max_retries`. When a budget runs out, work waits (or moves to Strata if it's size S) instead of spending more.
- **Limit detection:** when a CLI reports a usage limit, that agent is marked "cooling" until the reset time it gives, and the conductor routes around it.
- **Cloud Claude** (chat sessions like this one) is for planning. It runs when Erik starts it, with an optional daily summary as a scheduled task. Off by default.
- **Usage log:** every run records agent, task, duration and, where the CLI reports it, tokens. A week of numbers tells us what to tune.

## Safety

- Runs as the CraftML user: no admin rights, worktrees only under `E:\CraftML\work\wt\`.
- GitHub through a **fine-grained token** limited to the fork, `craft-ai-manager` and `craft-conductor`: contents and pull requests read/write, nothing else.
- Never pushes to `main`, never merges, never deletes branches with open PRs, never force-pushes.
- One worktree per task; a failed run leaves the branch for inspection.
- Kill switch: a file `E:\CraftML\conductor\STOP` pauses everything at the next loop.

## Where it lives

A separate private repo, `erikerikerik/craft-conductor`, because it serves both the fork and CAIM. Until that exists, this design lives here in `agents/`.

**v0 language:** PowerShell 5.1 (already on Windows, nothing to install, easy for Erik to read). Rewrite in Rust only if it grows.

## Later (v1+)

- Auto-merge for `ai:approved` PRs whose guards pass, once Erik trusts it.
- Claude Code pointed at Strata's Anthropic-style endpoint (`/v1/messages`) as a fallback reviewer when Claude limits are reached.
- A read-only status page.
