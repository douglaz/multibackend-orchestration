## Summary

Replace `default_final_reviewer_template()` in `src/prompts/templates.rs` and `FINAL_REVIEWER_GUARDRAILS` in `src/workflow/orchestrator.rs` with a structured review framework that adds bug-qualification rules, priority tagging (`[P0]`–`[P3]`), and concrete `git diff` instructions using new `{{review_diff_command}}`, `{{base_branch}}`, and `{{merge_base_sha}}` template variables. The same variables are also inserted in `src/workflow/quick_dev_orchestrator.rs::build_final_review_prompt()` to prevent dangling template literals. The merge-base computation reuses the existing `git::commit::merge_base` and `git::branch::remote_ref_exists` helpers, with a multi-tier fallback that guarantees the rendered diff command is always valid shell syntax. The Markdown output format, parser (`parse_final_reviewer_output`), and `Amendment` struct remain unchanged.

## Acceptance Criteria

1. `cargo check` — no compilation errors.
2. `cargo test --lib parse_final_reviewer` — all existing parser tests pass unchanged.
3. `ralph validate --filter final_review` — conformance tests pass (covers `final_review::completion_no_amendments`, `final_review::restart_round_then_complete`, `final_review::planner_completion_after_amendments_fails`).
4. `ralph validate --filter final_review_cap_skip` — conformance tests pass.
5. `ralph validate` (full suite) — no regressions in `tests_resume_backend_resolution`, `tests_prompt_review_panel`, `tests_completion_panel`, `tests_stray_cleanup`, or `tests/orchestrator.rs`.
6. `default_final_reviewer_template()` contains all seven "WHEN TO FLAG AN ISSUE" rules.
7. `default_final_reviewer_template()` contains priority level definitions (`[P0]`–`[P3]`).
8. `default_final_reviewer_template()` contains `{{review_diff_command}}` in the workflow section (not a raw `{{merge_base_sha}}` embedded inside a `git diff` command).
9. `build_final_reviewer_prompt()` in `orchestrator.rs` inserts `base_branch`, `merge_base_sha`, and `review_diff_command` into `vars`.
10. `build_final_review_prompt()` in `quick_dev_orchestrator.rs` inserts `base_branch`, `merge_base_sha`, and `review_diff_command` into `vars`, preventing dangling `{{…}}` literals.
11. `compute_merge_base_sha()` uses `git::commit::merge_base` and `git::branch::remote_ref_exists` — no direct `std::process::Command` usage.
12. `compute_merge_base_sha()` tries `origin/<branch>` first (via `remote_ref_exists`), falls back to local branch name, returns `None` on failure.
13. When merge-base resolution fails entirely, `review_diff_command` falls back to `git diff HEAD -- . ':(exclude).ralph'` (valid syntax, reviews only the working tree against HEAD).
14. Amendment ID format in `## Amendment: <ID>` heading is unchanged (no priority prefix). Priority tags appear only inside `### Problem`.
15. No changes to `src/workflow/parser.rs` or any parser test files.
16. The first line of the template remains `"You are a final reviewer auditing a completed project for correctness, safety, and robustness."` — this string is matched by ~15 test scripts across `src/validate/` and `tests/orchestrator.rs` to route prompts.

## Technical Approach

### 1. Replace `default_final_reviewer_template()` (`src/prompts/templates.rs:396–448`)

Replace the function body with a new template that retains the opening sentence verbatim (`"You are a final reviewer auditing a completed project for correctness, safety, and robustness."`) since it is used as a prompt-routing sentinel in test mock scripts (see `src/validate/tests_final_review.rs:202`, `tests_final_review_cap_skip.rs:230`, `mock_scripts.rs:321/464/650/794`, `tests_resume_backend_resolution.rs:652/804/956/1148`, `tests_prompt_review_panel.rs:159`, `tests_completion_panel.rs:120/253`, `tests/orchestrator.rs:2936`).

The new template body adds the following sections, in order:

| Section | Content |
|---------|---------|
| Role preamble | Retain verbatim first sentence. Expand with "You have full access…" paragraph. |
| **WHEN TO FLAG AN ISSUE** | 7 numbered rules: (1) meaningfully impacts correctness/perf/security/maintainability; (2) discrete and actionable; (3) rigor matches codebase norms; (4) introduced by project changes, not pre-existing; (5) no unstated assumptions about runtime/environment; (6) affected code paths must be provably identified; (7) not an intentional, documented design choice. |
| **HOW TO WRITE AMENDMENT BODIES** | 7 rules: cite files/lines/functions; accurate severity via `[P0]`–`[P3]` tag; ≤1–2 paragraphs for Problem; no code blocks >3 lines; state required reproduction scenarios; matter-of-fact tone; immediately graspable. |
| **HOW MANY AMENDMENTS** | Output all qualifying findings. Prefer NO AMENDMENTS if nothing would definitely be fixed. |
| **PRIORITY LEVELS** | `[P0]` blocking correctness/safety, `[P1]` urgent, `[P2]` normal, `[P3]` low. |
| **ADDITIONAL GUIDELINES** | Style issues only if they obscure meaning or violate documented standards; one amendment per discrete issue; concurrency isolation checks; test assertion quality; stray files; open scope beyond spec. |
| **YOUR WORKFLOW** | Uses `{{review_diff_command}}` (a complete, pre-computed shell command) — **not** a raw `{{merge_base_sha}}` embedded in a `git diff` template. Also references `{{base_branch}}` for context. |
| **CRITICAL FORMAT REQUIREMENTS** | Identical format constraints as today. |
| Output examples | Identical `# Final Review: NO AMENDMENTS` / `# Final Review: AMENDMENTS` blocks with `## Summary`, `## Amendment: <ID>` → `### Problem` / `### Proposed Change` / `### Affected Files`. |
| `{{system_guardrails}}` | Variable at end under `## System Guardrails`. |

The template uses `{{review_diff_command}}`, `{{merge_base_sha}}`, and `{{base_branch}}` (double-brace syntax matching `render_template()` in `templates.rs:7–31`). The `render_template_with_fallback` function performs literal string replacement via `str::replace` — unmatched `{{key}}` tokens are left as-is in the rendered output. Therefore all three variables **must** be inserted into `vars` at every call site (even as empty strings) to avoid dangling literals. The `{{review_diff_command}}` variable is always a syntactically valid shell command (see fallback logic in step 4).

### 2. Replace `FINAL_REVIEWER_GUARDRAILS` constant (`src/workflow/orchestrator.rs:84–90`)

Replace with a new constant covering:

- Full codebase access via tools.
- Broad, open-ended review — not limited to specific categories.
- Evaluate project-wide outcomes against master prompt.
- Concrete, high-signal amendments with affected files and priority tags (`[P0]`–`[P3]`).
- Globally unique amendment IDs.
- When following an amendment round: verify prior amendments via code tracing, not self-reported changes.
- Ignore trivial style unless it obscures meaning or violates documented standards.
- One amendment per discrete, actionable issue.

### 3. Add `base_branch`, `merge_base_sha`, and `review_diff_command` to `build_final_reviewer_prompt()` (`src/workflow/orchestrator.rs:4484–4487`)

Add a `repo_root: Option<&Path>` parameter to `build_final_reviewer_prompt()`. After the existing `vars.insert("system_guardrails", ...)` block (line 4484–4487), insert:

```rust
let base_branch = effective.global.git.base_branch.clone();
vars.insert("base_branch".to_owned(), base_branch.clone());
let merge_base_sha = repo_root
    .and_then(|root| compute_merge_base_sha(root, &base_branch))
    .unwrap_or_default();
let review_diff_command = build_review_diff_command(&merge_base_sha);
vars.insert("merge_base_sha".to_owned(), merge_base_sha);
vars.insert("review_diff_command".to_owned(), review_diff_command);
```

Update the call site at `orchestrator.rs:3530` to pass `repo_root_ref`:

```rust
let prompt = build_final_reviewer_prompt(
    effective,
    state,
    prompt_content,
    reviewer_backend_impl.name(),
    &planner_backend_name,
    repo_root_ref,  // new parameter
)?;
```

This mirrors the existing pattern where `repo_root_ref` is already threaded through the orchestrator's `run()` method (declared at line 249, used at lines 374, 439, 622, etc.).

### 4. Add `compute_merge_base_sha()` and `build_review_diff_command()` helpers (`src/workflow/orchestrator.rs`)

Place near `build_final_reviewer_prompt`. Both use existing `git` module helpers instead of raw `std::process::Command`:

```rust
fn compute_merge_base_sha(repo_root: &Path, base_branch: &str) -> Option<String> {
    use crate::git::branch::remote_ref_exists;
    use crate::git::commit::merge_base;

    let remote_ref = format!("origin/{base_branch}");
    if remote_ref_exists(repo_root, &remote_ref).unwrap_or(false) {
        if let Ok(sha) = merge_base(repo_root, &remote_ref, "HEAD") {
            return Some(sha);
        }
    }
    // Fallback: try local branch name (works in local-only repos without a remote).
    merge_base(repo_root, base_branch, "HEAD").ok()
}

fn build_review_diff_command(merge_base_sha: &str) -> String {
    if merge_base_sha.is_empty() {
        "git diff HEAD -- . ':(exclude).ralph'".to_owned()
    } else {
        format!("git diff {merge_base_sha}...HEAD -- . ':(exclude).ralph'")
    }
}
```

**Rationale for reusing existing helpers:**
- `git::commit::merge_base(workdir, left, right)` (`src/git/commit.rs:400`) already wraps `git merge-base` with proper `current_dir`, error handling via `RalphError::Orchestration`, and output trimming through the shared `run_git` helper.
- `git::branch::remote_ref_exists(workdir, remote_ref)` (`src/git/branch.rs:70`) wraps `git rev-parse --verify` and returns `Result<bool>`, giving a clean check before attempting merge-base against a remote ref that may not exist.
- Using these avoids duplicating `std::process::Command` invocations with inconsistent error handling patterns.

**Fallback chain** (addresses edge case where configured `git.base_branch` is missing):
1. Try `merge_base(repo_root, "origin/<base_branch>", "HEAD")` — works in CI/detached-HEAD scenarios.
2. Try `merge_base(repo_root, "<base_branch>", "HEAD")` — works in local-only repos.
3. If both fail, `compute_merge_base_sha` returns `None` → `unwrap_or_default()` → empty string → `build_review_diff_command` produces `git diff HEAD -- . ':(exclude).ralph'`, which is always valid shell syntax (shows staged+unstaged changes, enough for an LLM reviewer to begin its review with tool-based file reading).

### 5. Add `base_branch`, `merge_base_sha`, and `review_diff_command` to `quick_dev_orchestrator.rs::build_final_review_prompt()` (`src/workflow/quick_dev_orchestrator.rs:1218`)

Add a `repo_root: Option<&Path>` parameter. Insert the same three variables after existing `vars.insert("master_prompt", ...)`:

```rust
fn build_final_review_prompt(
    effective: &EffectiveConfig,
    prompt_content: &str,
    repo_root: Option<&Path>,  // new parameter
) -> Result<String> {
    let mut vars = BTreeMap::new();
    vars.insert(
        "system_guardrails".to_owned(),
        QUICK_DEV_REVIEWER_GUARDRAILS.to_owned(),
    );
    vars.insert("master_prompt".to_owned(), prompt_content.to_owned());

    // Insert review diff variables so {{…}} tokens are always substituted.
    let base_branch = effective.global.git.base_branch.clone();
    vars.insert("base_branch".to_owned(), base_branch.clone());
    let merge_base_sha = repo_root
        .and_then(|root| compute_merge_base_sha(root, &base_branch))
        .unwrap_or_default();
    let review_diff_command = build_review_diff_command(&merge_base_sha);
    vars.insert("merge_base_sha".to_owned(), merge_base_sha);
    vars.insert("review_diff_command".to_owned(), review_diff_command);

    // ... rest of function unchanged ...
}
```

`compute_merge_base_sha` and `build_review_diff_command` are defined in `orchestrator.rs`. To avoid cross-module duplication, extract them to a shared location (e.g., a `pub(crate)` function in `src/workflow/mod.rs` or `src/git/commit.rs`). Alternatively, since `quick_dev_orchestrator.rs` is in the same `workflow` module, they can be made `pub(super)` in `orchestrator.rs` and imported directly.

Update both call sites in `quick_dev_orchestrator.rs` (lines 686 and 729) to pass `repo_root`:

```rust
let impl_final_prompt = build_final_review_prompt(effective, &prompt_content, repo_root.as_deref())?;
// ...
let rev_final_prompt = build_final_review_prompt(effective, &prompt_content, repo_root.as_deref())?;
```

The `repo_root` variable is already available at both call sites — it is declared at `quick_dev_orchestrator.rs:128` as `let repo_root: Option<PathBuf> = self.workspace.root.parent().map(|p| p.to_owned());`.

### 6. No test file modifications expected

All test scripts match on the sentinel string `"You are a final reviewer auditing a completed project for correctness, safety, and robustness."` which is preserved verbatim as the first line of the new template. No test assertions pattern-match on the guardrails constant text or other template body content. Therefore no changes to `src/validate/tests_final_review.rs`, `src/validate/tests_final_review_cap_skip.rs`, or any other test file should be necessary. If `ralph validate` reveals a match on removed text, the fix is a targeted substring update in the affected mock script.

## Files & Modules

| File | Change | Lines affected |
|------|--------|---------------|
| `src/prompts/templates.rs` | Replace body of `default_final_reviewer_template()` | Lines 396–448 |
| `src/workflow/orchestrator.rs` | Replace `FINAL_REVIEWER_GUARDRAILS` constant | Lines 84–90 |
| `src/workflow/orchestrator.rs` | Add `repo_root: Option<&Path>` parameter and `base_branch` + `merge_base_sha` + `review_diff_command` vars in `build_final_reviewer_prompt()` | Lines 4461–4517 |
| `src/workflow/orchestrator.rs` | Update call site to pass `repo_root_ref` | Line 3530 |
| `src/workflow/orchestrator.rs` | Add `compute_merge_base_sha()` helper (uses `git::commit::merge_base` + `git::branch::remote_ref_exists`) | New function near line 4517 |
| `src/workflow/orchestrator.rs` | Add `build_review_diff_command()` helper | New function near `compute_merge_base_sha` |
| `src/workflow/quick_dev_orchestrator.rs` | Add `repo_root: Option<&Path>` parameter and `base_branch` + `merge_base_sha` + `review_diff_command` vars in `build_final_review_prompt()` | Lines 1218–1238 |
| `src/workflow/quick_dev_orchestrator.rs` | Update two call sites (lines 686, 729) to pass `repo_root.as_deref()` | Lines 686, 729 |

**Files NOT modified:**
- `src/workflow/parser.rs` — output format unchanged
- `src/git/commit.rs` — existing `merge_base` helper used as-is
- `src/git/branch.rs` — existing `remote_ref_exists` helper used as-is
- `src/validate/tests_final_review.rs` — sentinel string preserved
- `src/validate/tests_final_review_cap_skip.rs` — sentinel string preserved
- `src/validate/mock_scripts.rs` — sentinel string preserved
- Any planner, vote, or arbiter templates
- `Amendment` struct or any parser types

## Testing Strategy

1. **Existing parser tests** (`cargo test --lib parse_final_reviewer`): Must pass unchanged. These are standard `#[test]` functions in `src/workflow/parser.rs` that validate `# Final Review: NO AMENDMENTS` / `# Final Review: AMENDMENTS` heading parsing, amendment block extraction, required subsections (`Problem`, `Proposed Change`, `Affected Files`), and duplicate-ID detection — none of which change.

2. **Conformance tests via `ralph validate`**: The `tests_final_review` and `tests_final_review_cap_skip` modules define `ConformanceTest` structs (not `#[test]` functions) exercised through the `ralph validate` binary subcommand:
   - `ralph validate --filter final_review` — exercises `final_review::completion_no_amendments`, `final_review::restart_round_then_complete`, `final_review::planner_completion_after_amendments_fails`.
   - `ralph validate --filter final_review_cap_skip` — exercises cap-skip scenarios.
   - These tests spin up a `RalphHarness` with mock backend scripts that route prompts by matching on the sentinel string. Since the sentinel is preserved, no mock script changes are needed.

3. **Full conformance suite** (`ralph validate`): Run without `--filter` to catch regressions in any other test module (`tests_resume_backend_resolution`, `tests_stray_cleanup`, `tests_prompt_review_panel`, `tests_completion_panel`, `tests_validate_flags`) that routes on the sentinel string.

4. **Compile-time verification** (`cargo check`): Ensures the new `repo_root: Option<&Path>` parameter is threaded through all call sites in both `orchestrator.rs` and `quick_dev_orchestrator.rs`, and that `git::commit::merge_base` / `git::branch::remote_ref_exists` imports resolve correctly.

5. **Merge-base fallback verification**: Both `ralph validate` conformance tests and the `tests/orchestrator.rs` integration tests run in real git repositories. The `compute_merge_base_sha` function will execute against these repos, exercising the `remote_ref_exists` → `merge_base` → local fallback chain. To explicitly verify the fallback-to-empty-string path (no valid base ref), add a temporary `eprintln!` or inspect the rendered prompt in log output from a conformance test run in a repo where the configured `base_branch` does not exist.

6. **No new test files needed**: The change is a prompt content replacement with two small helpers. `compute_merge_base_sha` delegates to tested git helpers (`merge_base`, `remote_ref_exists`) and has trivial fallback logic (`Option` chaining). `build_review_diff_command` is a pure string formatter with a single branch. Both are exercised end-to-end by the existing conformance tests running in real git repos.

## Out of Scope

- JSON output format replacing Markdown.
- Per-amendment confidence scores.
- Structured `code_location` parsing (file/line extracted from amendment body).
- Priority-based filtering before deliberation (e.g., auto-reject `[P3]` amendments).
- Changes to planner, vote, or arbiter templates.
- Changes to `parser.rs` or the `Amendment` struct.
- Adding `merge_base_sha` / `base_branch` / `review_diff_command` to any template other than final reviewer.
- Unit tests for `compute_merge_base_sha` (requires a real git repo; covered by existing integration/conformance tests).