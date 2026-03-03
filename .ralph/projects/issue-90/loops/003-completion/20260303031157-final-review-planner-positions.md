---
artifact: final-review-planner-positions
loop: 3
project: issue-90
backend: claude(opus)
role: planner
created_at: 2026-03-03T03:11:57Z
---

I now have all the information I need to evaluate every amendment. Let me verify one last detail — whether the test at line 338 actually checks file content or just deserialized values.

The test at lines 338-357 does `toml::from_str(&merged_str)` and then `assert_eq!(parsed, GlobalConfig::default())` — this asserts the deserialized struct, not the raw file content. Since `toml::from_str` fills in serde defaults for missing keys, this assertion can pass even if keys were never physically written to the TOML file. The amendment FR-002's claim about this is correct.

# Planner Positions

## Amendment: AM1

### Position
ACCEPT

### Rationale
Verified: the file `20260303T023119Z-impl-notes.md` exists at the repo root and was added in commit `62dfa01` (loop 2 implementing -> reviewing). It is a scratch/notes artifact with no runtime or test purpose. It shows up in `git diff origin/master...HEAD` as a new tracked file. It does not belong in the source tree.

## Amendment: AM2

### Position
ACCEPT

### Rationale
Verified in `src/cli/init.rs:272-275`: `plan_actions()` is a single-line wrapper that calls `plan_full_actions(root)` and has an explicit `#[allow(dead_code)]` attribute. A project-wide grep for `plan_actions` shows the only definition is at line 273, and no call sites exist — all callers use `plan_full_actions` directly (lines 364, 398, 415, 491-492). This is dead code that should be removed.

## Amendment: FR-001

### Position
ACCEPT

### Rationale
Verified by reading `src/config/global.rs:1259-1273`. The `ensure_tables()` function at line 1270 calls `.as_table_mut().expect("just inserted a table")`. When a key already exists in the document as an inline table (e.g., `workspace = { default_backend = "codex" }`), the insert is skipped (key exists), but `as_table_mut()` returns `None` for inline tables in `toml_edit` (it only matches `Item::Table`, not `Item::Value(Value::InlineTable)`) — confirmed via toml_edit docs. This causes a panic (`exit 101`).

Similarly, `navigate_tables_mut()` at line 1254 uses `as_table_mut()?` which silently returns `None` for inline tables, causing the removal branch at line 1189-1191 to silently no-op instead of removing the key.

This is a real correctness bug: any user who hand-edits their `ralph.toml` using inline table syntax (valid TOML) will hit a panic on `config set` or get silent false-success on key removal.

## Amendment: FR-002

### Position
ACCEPT

### Rationale
Verified in `src/cli/init.rs:298-307`. The `merge_tables()` function at line 305 does `existing_item.as_table_mut()`, which returns `None` for inline tables (same `toml_edit` behavior as FR-001). When an existing config section uses inline table syntax, the recursion into nested defaults is silently skipped — missing default keys will not be backfilled.

Also verified the test claim: `copy_files_overlay_fills_missing_keys` at `src/validate/tests_init.rs:338-357` deserializes via `toml::from_str` and asserts `parsed == GlobalConfig::default()`. Since serde's `#[serde(default)]` fills missing fields with defaults during deserialization, this assertion can pass even when keys were never physically written to the TOML file. The test does not actually verify that the merge function inserted the missing keys — it only verifies that deserialization produces the expected result (which it always would, due to defaults). This is a genuine test gap.

## Amendment: FR-003

### Position
ACCEPT

### Rationale
This is a duplicate of AM1 (both identify the same file `20260303T023119Z-impl-notes.md` at repo root). The problem is real — verified the file exists and is tracked in the diff. Accepting for the same reasons as AM1.
