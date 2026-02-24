I now have the full `cargo clippy` output (20 warnings) and the project structure. Let me write the specification.

## Summary

Run `cargo clippy` and resolve all 20 warnings across the ralph codebase. Warnings fall into six categories: unused imports (1), dead code (3), style issues (5: `let...else`→`?`, unneeded `return`, collapsible `else if`, `single_match`, `unused_unit`), unnecessary derefs (3), simplifiable `map_or` calls (4), and `too_many_arguments` (4). Dead code is deleted entirely. `too_many_arguments` warnings are suppressed with `#[allow(clippy::too_many_arguments)]`. All other warnings are fixed mechanically. No public API or behavioral changes.

## Acceptance Criteria

- [ ] `cargo clippy` produces zero warnings
- [ ] All unused imports are removed
- [ ] All dead code (`verdict_label`, `HistoryEntry`, `loop_number`) is deleted — not suppressed
- [ ] All style issues are fixed inline (`?` operator, remove unneeded `return`, collapse `else { if }`, `match`→`if`, remove `-> ()`)
- [ ] All `needless_option_as_deref` derefs are removed
- [ ] All `map_or(false, …)` / `map_or(true, …)` calls are replaced with `is_some_and` / `is_none_or`
- [ ] All `too_many_arguments` warnings are suppressed with `#[allow(clippy::too_many_arguments)]`
- [ ] No public API changes
- [ ] No behavioral changes
- [ ] `cargo build` succeeds
- [ ] `cargo test` passes (existing tests)

## Technical Approach

Each warning is a mechanical, localized fix. Apply them file-by-file:

### 1. Unused imports (1 warning)
| File | Line | Fix |
|------|------|-----|
| `src/backend/mod.rs:12` | `use std::os::unix::process::CommandExt` | Delete the import line |

### 2. Dead code (3 warnings) — delete entirely
| File | Lines | Item | Fix |
|------|-------|------|-----|
| `src/cli/history.rs:138` | fn `verdict_label` (~18 lines) | Delete the entire function |
| `src/cli/history.rs:157` | enum `HistoryEntry` | Delete the enum definition |
| `src/cli/history.rs:162-170` | impl block with `loop_number` | Delete the entire impl block |

### 3. Style issues (5 warnings)
| File | Line | Lint | Fix |
|------|------|------|-----|
| `src/backend/output_normalizer.rs:283` | `question_mark` | Replace `let Some(items) = content.as_array() else { return None; };` with `let items = content.as_array()?;` |
| `src/daemon/process.rs:272` | `needless_return` | Remove the bare `return;` statement (it's the last statement in the `if` block) |
| `src/daemon/rebase_agent.rs:282` | `single_match` | Replace `match backend { RebaseAgentBackend::None => { … } _ => {} }` with `if backend == RebaseAgentBackend::None { … }` |
| `src/validate/tests_prd.rs:119` | `unused_unit` | Remove `-> ()` from `fn setup_prd_mock` signature |
| `src/workflow/orchestrator.rs:1957` | `collapsible_else_if` | Collapse `} else { if … }` into `} else if … ` |

### 4. Unnecessary derefs (3 warnings)
| File | Line | Fix |
|------|------|-----|
| `src/backend/tmux_backend.rs:421` | Replace `log_writer.as_deref_mut()` with `log_writer` in `if let Some(writer) =` |
| `src/backend/mod.rs:43` | Replace `log_writer.as_deref_mut()` with `log_writer` in `let _ =` |
| `src/backend/mod.rs:623` | Replace `log_writer.as_deref_mut()` with `log_writer` in `if let Some(writer) =` |

**Note**: Since the deref type is the same as the origin, `.as_deref_mut()` is a no-op. For `let _ = log_writer.as_deref_mut()`, replace with `let _ = log_writer;` (or simply `let _ = &mut log_writer;` if the intent is to suppress unused warnings — verify context). Actually, per clippy's suggestion, replace `log_writer.as_deref_mut()` with just `log_writer` in each case.

### 5. Simplifiable `map_or` (4 warnings)
| File | Line | Fix |
|------|------|-----|
| `src/validate/tests_commands.rs:501` | `.map_or(false, \|c\| c.is_ascii_digit())` → `.is_some_and(\|c\| c.is_ascii_digit())` |
| `src/validate/tests_run.rs:260` | `.map_or(false, \|n\| n.contains(…))` → `.is_some_and(\|n\| n.contains(…))` |
| `src/validate/tests_run.rs:853` | Same pattern → `.is_some_and(…)` |
| `src/validate/tests_streaming.rs:669` | `.map_or(true, \|a\| a.is_empty())` → `.is_none_or(\|a\| a.is_empty())` |

### 6. Too many arguments (4 warnings) — suppress
| File | Line | Function | Fix |
|------|------|----------|-----|
| `src/daemon/runtime.rs:181` | `post_artifact_comments_with_client` (9 args) | Add `#[allow(clippy::too_many_arguments)]` above fn |
| `src/daemon/runtime.rs:236` | `sweep_artifact_comments` (8 args) | Add `#[allow(clippy::too_many_arguments)]` above fn |
| `src/daemon/runtime.rs:279` | `try_post_artifact_comment` (8 args) | Add `#[allow(clippy::too_many_arguments)]` above fn |
| `src/workflow/orchestrator.rs:4694` | `execute_with_parse_retries` (13 args) | Add `#[allow(clippy::too_many_arguments)]` above fn |

## Files & Modules

| File | Edits | Warning Categories |
|------|-------|--------------------|
| `src/backend/mod.rs` | 3 | unused import, needless deref ×2 |
| `src/backend/output_normalizer.rs` | 1 | question_mark style |
| `src/backend/tmux_backend.rs` | 1 | needless deref |
| `src/cli/history.rs` | 1 (delete block) | dead code ×3 |
| `src/daemon/process.rs` | 1 | needless return |
| `src/daemon/rebase_agent.rs` | 1 | single_match |
| `src/daemon/runtime.rs` | 3 | too_many_arguments ×3 |
| `src/validate/tests_commands.rs` | 1 | unnecessary map_or |
| `src/validate/tests_prd.rs` | 1 | unused unit |
| `src/validate/tests_run.rs` | 2 | unnecessary map_or ×2 |
| `src/validate/tests_streaming.rs` | 1 | unnecessary map_or |
| `src/workflow/orchestrator.rs` | 2 | collapsible else_if, too_many_arguments |

**Total: 12 files, 18 edit sites, 20 warnings resolved.**

## Testing Strategy

1. **Primary validation**: Run `cargo clippy` after all fixes — must produce zero warnings.
2. **Build check**: Run `cargo build` — must compile without errors.
3. **Test suite**: Run `cargo test` — all existing tests must pass. The dead code deletions in `history.rs` remove never-called functions, so no test can reference them. The `map_or`→`is_some_and`/`is_none_or` changes in test files are semantically identical.
4. **Manual review**: Verify the `collapsible_else_if` change in `orchestrator.rs` preserves the exact same branch logic (indentation-only change, no logic difference).

## Out of Scope

- Refactoring functions to reduce argument count (suppressed with `#[allow]` instead)
- Adding new clippy lints or enabling `#![warn(clippy::pedantic)]`
- Refactoring dead code into used code paths
- Changing any public struct/enum/function signatures
- Updating documentation or adding comments beyond the `#[allow(…)]` attributes
- Addressing any warnings from `cargo clippy -- -W clippy::pedantic` or other non-default lint groups