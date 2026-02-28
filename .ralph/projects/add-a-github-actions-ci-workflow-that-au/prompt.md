Implement a GitHub Actions CI workflow for this repository.

### Goal
Create a CI workflow that runs on:
- Pull requests targeting `master`
- Pushes to `master`

The workflow must use the project’s Nix Flakes setup for all build/test/lint/format checks.

### Deliverable
- Create exactly one file: `.github/workflows/ci.yml`
- Do not modify any other file.

### Required Workflow Behavior
Use a single job named `ci` on `ubuntu-latest` with these steps in this order:

1. `actions/checkout@v4`
2. `DeterminateSystems/nix-installer-action@v21`
3. `DeterminateSystems/magic-nix-cache-action@v13`
4. Run `nix build -L`
5. Run `nix develop -c cargo clippy --all-targets -- -D warnings`
6. Run `nix develop -c cargo fmt -- --check`

### Constraints
- Use only pinned version tags above (no `@main`, `@master`, or unpinned refs).
- No bare `cargo` commands outside `nix develop -c ...`.
- No extra jobs or matrix strategy.
- Keep branch filters strictly on `master`.
- Add minimal permissions for the job: `contents: read`.

### Acceptance Criteria
- `.github/workflows/ci.yml` exists and is valid GitHub Actions YAML.
- Workflow triggers exactly on:
  - `pull_request` with `branches: [master]`
  - `push` with `branches: [master]`
- `ci` job runs the three required command steps exactly as specified.
- Nix cache action is present and runs before Nix build/dev commands.
- Cache behavior is considered correct if logs show cache restore and/or cache upload activity; fork PR cache-write limitations are acceptable and must not fail CI.
- Workflow uses only the required file change (`.github/workflows/ci.yml`).

### Notes for Implementer
- `nix build -L` is expected to execute the project’s configured build/test/validate pipeline as defined by the flake.
- Do not encode brittle assertions in workflow comments (for example, exact conformance test counts), because these can change over time.