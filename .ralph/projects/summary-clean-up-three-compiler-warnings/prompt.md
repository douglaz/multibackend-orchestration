### Objective
Apply a targeted dead-code cleanup in `src/backend/mod.rs` by removing an unused helper and any cascading orphaned import, with deterministic behavior and explicit validation.

### Scope
- In scope:
  1. Evaluate call-sites for `kill_and_reap_child`.
  2. Apply the deterministic rule below.
  3. Remove now-unused imports caused by this change.
  4. Run required validation commands.
- Out of scope:
  1. Refactors unrelated to `kill_and_reap_child` or `ErrorKind`.
  2. Behavioral changes to backend process lifecycle beyond this dead-code cleanup.

### Deterministic Rule
1. Search for call-sites of `kill_and_reap_child` across `src/` and `tests/`.
2. If call-site count is **zero**: delete `kill_and_reap_child` from `src/backend/mod.rs`.
3. If call-site count is **non-zero**: keep the function and add `#[allow(dead_code)]` above it.
4. For `use std::io::ErrorKind;` in `src/backend/mod.rs`: remove it if no remaining references exist after step 2 or 3.

### Files & Modules
| File | Required change |
|---|---|
| `src/backend/mod.rs` | Apply deterministic rule for `kill_and_reap_child`; remove orphaned `ErrorKind` import if unused. |

### Required Validation Commands
Run and record results for:
1. `rg -n "kill_and_reap_child\(" src tests`
2. `rg -n "\bErrorKind\b" src/backend/mod.rs src`
3. `cargo check`

### Acceptance Criteria
1. Deterministic rule is applied exactly as specified.
2. If zero call-sites are found, `kill_and_reap_child` is deleted.
3. `use std::io::ErrorKind;` is removed when orphaned.
4. `cargo check` succeeds.
5. No new warnings are introduced by this cleanup.
6. Diff is limited to intended symbols/files.

### Deliverable
Provide:
1. A short summary of edits made.
2. Validation command outputs (or concise pass/fail summaries).
3. Final determination of which branch of the deterministic rule was taken and why.