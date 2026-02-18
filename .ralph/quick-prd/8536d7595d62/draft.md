## Summary

Remove three compiler warnings flagged by `cargo check`: two unused imports and one dead-code warning. Deleting the dead-code function cascades into a fourth unused import (`std::io::ErrorKind`), which must also be removed. All changes are deletions in existing files with zero behavioral impact.

## Acceptance Criteria

- [ ] `assert_stderr_contains` import removed from `src/validate/tests_auto_init.rs`
- [ ] `std::os::unix::process::CommandExt` import removed from `src/backend/mod.rs`
- [ ] `kill_and_reap_child` function deleted from `src/backend/mod.rs`
- [ ] `std::io::ErrorKind` import removed from `src/backend/mod.rs` (cascading cleanup)
- [ ] `cargo check` completes with zero warnings
- [ ] All existing tests pass (`cargo test`)

## Technical Approach

1. **`src/validate/tests_auto_init.rs:6`** — Delete the `assert_stderr_contains` token from the grouped `use` import line. The line currently reads:
   ```rust
   use crate::validate::assertions::{
       assert_dir_exists, assert_exit_code, assert_file_exists, assert_stderr_contains,
   };
   ```
   Remove `, assert_stderr_contains` so the remaining imports are preserved.

2. **`src/backend/mod.rs:16–17`** — Delete the two lines `#[cfg(unix)]` and `use std::os::unix::process::CommandExt;` entirely. The warning proves no call-site relies on this trait, so straight deletion is safe.

3. **`src/backend/mod.rs:453–470` — `kill_and_reap_child`** — Delete the entire function. The decision rule is deterministic: a codebase-wide search shows **zero call-sites** outside the function's own definition, so deletion is the correct action. (If call-sites had existed, `#[allow(dead_code)]` would apply instead, but that is not the case here.)

4. **`src/backend/mod.rs:11`** — Delete `use std::io::ErrorKind;`. This import's sole consumer is the `kill_and_reap_child` function deleted in step 3. Leaving it would introduce a new unused-import warning, violating the zero-warning goal.

5. **Validate** — Run `cargo check` and confirm zero warnings. Run `cargo test` and confirm no regressions.

## Files & Modules

| File | Change |
|---|---|
| `src/validate/tests_auto_init.rs` | Remove `assert_stderr_contains` from import list (~line 6) |
| `src/backend/mod.rs` | Remove `#[cfg(unix)]` + `use std::os::unix::process::CommandExt;` (~lines 16–17) |
| `src/backend/mod.rs` | Delete `kill_and_reap_child` function body (~lines 453–470) |
| `src/backend/mod.rs` | Remove `use std::io::ErrorKind;` (~line 11) — cascading from function deletion |

No new files. No changes to `Cargo.toml` or any other module.

## Testing Strategy

1. **`cargo check`** — Must emit zero warnings. This is the primary gate. Specifically verify that no new warnings are introduced by the cascading `ErrorKind` removal.
2. **`cargo test`** — Full test suite must pass. Since we are only removing unused code, no test should break. If any test references `kill_and_reap_child` directly, that test must also be removed or updated (unlikely given it is flagged as dead code).
3. **Manual review** — Confirm the diff touches only the four targeted locations and nothing else.

## Out of Scope

- Refactoring code adjacent to the removed lines
- Addressing any warnings not currently emitted by `cargo check`
- Adding new lints, clippy passes, or CI configuration
- Changing public API surface or module visibility
- Upgrading dependencies or toolchain
