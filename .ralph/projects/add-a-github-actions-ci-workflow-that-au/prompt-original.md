I now have everything I need. Let me produce the updated specification addressing all three review issues.

---

## Summary

Add a GitHub Actions CI workflow (`.github/workflows/ci.yml`) that validates every pull request targeting `master` and every push to `master`. The workflow uses the project's existing Nix Flakes setup exclusively — `nix build -L` for the full build + 192 conformance tests, `nix develop -c cargo clippy --all-targets -- -D warnings` for lint checks, and `nix develop -c cargo fmt -- --check` for formatting verification. A Nix store cache layer eliminates redundant builds across runs. All third-party Actions are pinned to immutable version tags (`actions/checkout@v4`, `DeterminateSystems/nix-installer-action@v21`, `DeterminateSystems/magic-nix-cache-action@v13`).

## Acceptance Criteria

- [ ] `.github/workflows/ci.yml` exists and is valid GitHub Actions YAML
- [ ] Workflow triggers on `pull_request` events targeting `master` and `push` events to `master`
- [ ] Job runs `nix build -L` which compiles the release binary, executes `cargo test`, and runs `ralph validate` conformance tests (192 tests) via the flake's `postCheck` hook
- [ ] Job runs `nix develop -c cargo clippy --all-targets -- -D warnings` to lint all targets (lib, bin, and integration tests) with zero warnings enforced
- [ ] Job runs `nix develop -c cargo fmt -- --check` to enforce formatting
- [ ] Nix store (`/nix/store`) is cached between workflow runs using `DeterminateSystems/nix-installer-action@v21` + `DeterminateSystems/magic-nix-cache-action@v13` (pinned version tags, not floating branch refs)
- [ ] Cache is considered operational when the `magic-nix-cache-action` post-job step logs `Uploading to cache` (cold run) or `Fetching from cache` (warm run). On fork PRs, cache restores are expected to succeed but writes may be skipped due to GitHub Actions' cross-fork cache isolation — this is acceptable behavior, not a failure.
- [ ] No bare `cargo` invocations — all commands run through `nix build` or `nix develop -c`

## Technical Approach

### Workflow Structure

A single job (`ci`) on `ubuntu-latest` with the following steps:

1. **Checkout** — `actions/checkout@v4`
2. **Install Nix** — `DeterminateSystems/nix-installer-action@v21`. This installs Nix with flake support enabled by default and is the GitHub-recommended Nix installer for Actions. Pinned to tag `v21` (November 2025) for reproducibility.
3. **Enable Nix cache** — `DeterminateSystems/magic-nix-cache-action@v13`. This transparently caches the Nix store using GitHub Actions cache, requiring zero configuration. It hooks into the Nix daemon to capture store paths built during the run and restores them on subsequent runs. Pinned to tag `v13` (July 2025) for reproducibility.
4. **Build & Test** — `nix build -L`. This single command triggers the full `buildRustPackage` derivation from `flake.nix`, which: compiles the release binary (static on Linux via `pkgsStatic`), runs `cargo test` (the `checkPhase`), patches shebangs for sandbox compatibility (`postPatch`), and runs `ralph validate --bin` conformance tests (`postCheck`).
5. **Clippy** — `nix develop -c cargo clippy --all-targets -- -D warnings`. Runs inside the dev shell which provides `clippy`, `cargo`, and `rustc` at matching versions. The `--all-targets` flag ensures linting covers the library, binary, and all 14 integration test files under `tests/`. The `--all-features` flag is omitted because the project defines no Cargo features. The `-D warnings` flag fails the step on any lint warning.
6. **Format check** — `nix develop -c cargo fmt -- --check`. Runs inside the dev shell which provides `rustfmt`. The `--check` flag exits non-zero if any file needs formatting.

### Why This Approach

- **Nix-first**: The project already defines its entire build, test, and dev environment in `flake.nix`. Reusing it in CI means no drift between local and CI environments.
- **`magic-nix-cache-action`**: This is the simplest Nix caching solution for GitHub Actions — it requires no configuration, no external services, and uses the built-in GitHub Actions cache. It caches all Nix store paths including the Rust toolchain, dependencies, and build artifacts.
- **Pinned action versions**: All third-party Actions use immutable version tags (`actions/checkout@v4`, `nix-installer-action@v21`, `magic-nix-cache-action@v13`) rather than floating branch refs like `@main`. This prevents unexpected breakage from upstream changes and ensures CI behavior is reproducible. Tags are preferred over commit SHAs for readability while still being immutable (GitHub Actions tags are fixed release points for these projects). Version bumps should be performed deliberately via PR.
- **Single job, sequential steps**: The build step compiles everything; clippy and fmt reuse the already-cached Nix store paths (Rust toolchain, deps). Parallelizing into separate jobs would duplicate Nix setup overhead for minimal wall-clock gain given the shared cache.
- **`--all-targets` on clippy**: The project contains a library (`src/lib.rs`), a binary (`src/main.rs`), and 14 integration test files (`tests/*.rs`). Default clippy only checks `--lib --bins`. Adding `--all-targets` also lints integration tests, which can contain lint issues (e.g., unused imports, dead code behind `#[cfg(test)]`) that would otherwise go undetected. `--all-features` is not included because `Cargo.toml` defines no `[features]` section — adding it would be a no-op today but could cause unexpected failures if features are added later without updating CI.
- **`-D warnings` on clippy**: Ensures the lint gate is strict, matching the project's recent commit history (PR #73 fixed clippy warnings).

### Caching Details

`magic-nix-cache-action` works by:
1. Starting a local HTTP binary cache server before the workflow steps
2. Intercepting Nix store writes during the build
3. Uploading new store paths to GitHub Actions cache at the end of the job
4. On subsequent runs, restoring cached paths before any Nix commands execute

This caches the entire toolchain (`rustc`, `cargo`, `clippy`, `rustfmt`, `gcc`, `musl`, etc.) and all Cargo crate derivations, so subsequent runs skip downloading and compiling dependencies entirely.

**Fork PR behavior**: GitHub Actions restricts cache writes from workflows triggered by fork PRs (for security — a fork could poison the cache for the base repo). In practice this means: fork PR runs will benefit from cache *restores* seeded by prior `master`/same-repo runs, but new store paths built during the fork PR run will not be persisted. This results in slightly slower fork PR builds (cold-cache penalty for new dependencies) but is correct, expected behavior — not a CI failure. The workflow requires no special configuration to handle this; `magic-nix-cache-action` degrades gracefully.

### Action Version Pinning Policy

| Action | Pinned ref | Release date | Rationale |
|--------|-----------|--------------|-----------|
| `actions/checkout` | `@v4` | Stable major | GitHub-maintained; major version tag is immutable within v4.x |
| `DeterminateSystems/nix-installer-action` | `@v21` | Nov 2025 | Latest stable release; installs Determinate Nix by default |
| `DeterminateSystems/magic-nix-cache-action` | `@v13` | Jul 2025 | Latest stable release; zero-config GitHub Actions cache |

To update pinned versions in the future: check the respective GitHub Releases pages, test the new version on a branch, and update the workflow via PR.

## Files & Modules

| File | Action | Description |
|------|--------|-------------|
| `.github/workflows/ci.yml` | **Create** | New CI workflow file — the only file change in this PR |

No modifications to existing source code, `flake.nix`, `Cargo.toml`, or any other files.

## Testing Strategy

1. **Workflow syntax validation**: Push the branch and open a PR against `master` — GitHub Actions will parse the YAML and report syntax errors immediately.
2. **Build & test step**: The `nix build -L` step exercises the exact same build + test pipeline used locally. The `-L` flag streams build logs so failures are diagnosable. This covers: compilation, `cargo test` (unit + integration tests), and 192 `ralph validate` conformance tests.
3. **Clippy step**: Verify the step passes on the current `master` codebase (which should be clean after PR #73). The `--all-targets` flag can be validated by confirming clippy's output includes `Checking ralph (tests)` entries for integration test files. Deliberately introduce a warning in a test file on a test branch to confirm the step catches test-target lint issues.
4. **Fmt step**: Verify the step passes on the current codebase. Deliberately introduce a formatting violation on a test branch to confirm the step fails.
5. **Cache effectiveness**: On the first (cold cache) run, verify that the `magic-nix-cache-action` post-job step logs `Uploading to cache` with a non-zero count of store paths. On the second (warm cache) run, verify that the step logs `Fetching from cache` and that `nix build` completes significantly faster (toolchain + dependency download/build should be skipped). Specific log indicators to check:
   - Cold run post-step: `Uploading N paths to cache` (N > 0)
   - Warm run setup: `Fetching from cache` / `Restored N paths from cache`
   - Warm run build time: substantially reduced vs. cold run (expect >50% reduction)
6. **Fork PR cache behavior**: Optionally verify on a fork PR that the run completes successfully even though cache writes are skipped. The `magic-nix-cache-action` post-step may log a warning or silently skip the upload — neither should cause step failure.
7. **Version pin verification**: Confirm the workflow YAML contains `@v21`, `@v13`, and `@v4` refs — not `@main`, `@master`, or bare branch names.

## Out of Scope

- **macOS / multi-platform CI**: The current flake supports `eachDefaultSystem`, but CI runs on `ubuntu-latest` only. macOS runners can be added later if needed.
- **Release automation**: No artifact publishing, binary uploads, or release tagging.
- **Dependency update automation** (e.g., Dependabot for `flake.lock` or `Cargo.lock`).
- **Code coverage reporting**.
- **Branch protection rules**: Configuring GitHub branch protection to require CI to pass is a repository settings change, not a workflow change.
- **Concurrency controls or workflow cancellation** for superseded runs — can be added as a follow-up if needed.
- **Separate clippy/fmt jobs**: All steps run in a single job to maximize cache reuse and minimize complexity. Splitting into parallel jobs is a future optimization.
- **Commit SHA pinning**: Actions are pinned to version tags (e.g., `@v21`) rather than full commit SHAs. Version tags provide sufficient immutability for these well-maintained actions while remaining human-readable. SHA pinning can be adopted later if supply-chain security requirements increase.
- **`--all-features` in clippy**: The project currently defines no Cargo features, so this flag would be a no-op. It is intentionally omitted to avoid false failures if features are added later without CI awareness.