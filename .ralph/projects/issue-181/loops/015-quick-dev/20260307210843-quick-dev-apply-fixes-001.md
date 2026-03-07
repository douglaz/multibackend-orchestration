---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-181
backend: claude
role: implementer
created_at: 2026-03-07T21:08:43Z
---

# Apply reviewer fix: harden branch-format validation

## Changes made

**1. `src/daemon/runtime.rs:1400` — Hardened `validate_daemon_branch_format`**

Replaced single-ID check (`issue-1 → ralph/issue-1`) with a two-ID check that validates both `issue-1 → ralph/issue-1` AND `issue-2 → ralph/issue-2`. A constant format like `"ralph/issue-1"` now fails on the second check because it produces `"ralph/issue-1"` instead of `"ralph/issue-2"`.

**2. `src/daemon/runtime.rs` — New unit test `daemon_branch_format_validation_rejects_constant_format`**

Asserts that `validate_daemon_branch_format("ralph/issue-1")` returns an error mentioning `git.branch_format` and `ralph/issue-2`.

**3. `src/validate/tests_daemon.rs` — New conformance test `daemon_branch_format_constant_blocks_dispatch`**

End-to-end test that configures `git.branch_format = "ralph/issue-1"`, runs the daemon with `--single-iteration`, and asserts it exits with code 2, prints the validation error, and never invokes the child command.

**Verification**: `cargo fmt`, `cargo clippy`, and all 3 branch-format unit tests pass.
