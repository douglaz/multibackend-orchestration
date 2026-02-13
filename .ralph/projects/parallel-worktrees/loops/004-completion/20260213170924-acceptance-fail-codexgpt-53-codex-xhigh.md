---
artifact: acceptance-fail
loop: 4
project: parallel-worktrees
backend: codex(gpt-5.3-codex-xhigh)
role: qa
created_at: 2026-02-13T17:09:24Z
---

# QA: FAIL
## Failures
- Project-wide checks passed (`nix develop -c cargo check`, `nix develop -c cargo test`, `nix build -L`, and `./result/bin/ralph validate --bin ./result/bin/ralph` with `98/98`), but the full diff audit against `origin/master` still has acceptance gaps.
- Legacy `created_at` fallback is incomplete. `src/workspace/summary.rs:19` through `src/workspace/summary.rs:31` falls back to `state.created_at` (sentinel `MIN_UTC`) when no loops exist, instead of using a real fallback timestamp. This violates the migration behavior in the spec.
  - Repro: remove `"created_at"` from a project `state.json`, then run `ralph project show p1 --json`; output contains `"created_at": "-262143-01-01T00:00:00Z"`.
- Empty/whitespace active-project files do not emit warnings. `src/workspace/active.rs:43` through `src/workspace/active.rs:45` returns `None` silently for empty/whitespace content, but the acceptance criteria require warning on stderr for empty, whitespace-only, and invalid active-project files.
  - Repro: write whitespace to `.git/ralph-active-project`, run `ralph project list`; stderr is empty.

## Suggested Fixes
- Implement full legacy `created_at` fallback:
  - When `created_at == MIN_UTC`, use earliest loop/completion `started_at`; if absent, fallback to `state.json` filesystem `mtime` (or `Utc::now()` if unavailable).
  - Update summary derivation path(s) so file metadata is available.
  - Add tests for legacy state missing `created_at` with and without loops.
- Emit warnings for empty/whitespace active-project files:
  - In `src/workspace/active.rs`, log a warning before returning `None` when trimmed content is empty.
  - Add validate/unit coverage for whitespace-only active-project files asserting warning behavior.
