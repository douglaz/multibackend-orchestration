Here is the specification:

---

## Summary

Switch the default `nix build` output to produce a fully static, portable `ralph` binary linked against musl libc. The current `flake.nix` uses `pkgs.rustPlatform.buildRustPackage` with glibc and an unused OpenSSL dependency. Replace this with a musl-based cross-compilation using `pkgsStatic` (or `pkgsCross.musl64`) so the resulting binary has zero dynamic library dependencies. The crate dependency tree is already pure Rust (no native C libraries), making this straightforward.

## Acceptance Criteria

1. `nix build` produces a statically-linked binary at `result/bin/ralph`.
2. `file result/bin/ralph` reports `statically linked`.
3. `ldd result/bin/ralph` reports `not a dynamic executable` (or equivalent).
4. `cargo check --target x86_64-unknown-linux-musl` succeeds.
5. `cargo test --target x86_64-unknown-linux-musl` succeeds (all existing tests pass).
6. `nix build` succeeds including the existing `postCheck` conformance tests (`ralph validate`).
7. The `devShells.default` continues to work for local development (may remain glibc-based).
8. No OpenSSL dependency remains in the build (it is unused).

## Technical Approach

### Current State

- `flake.nix` uses `pkgs.rustPlatform.buildRustPackage` with default glibc toolchain.
- `buildInputs` includes `pkgs.openssl` — unused (no TLS crates in dependency tree).
- All Rust dependencies are pure Rust: `unsafe-libyaml` (pure Rust libyaml port), `chrono`, `tokio`, `clap`, `serde_json`, `regex`, etc. No `-sys` crates that link C libraries.

### Approach: `pkgsStatic.rustPlatform.buildRustPackage`

Use Nix's `pkgsStatic` overlay, which cross-compiles everything against musl libc with static linking. This is the idiomatic Nix approach:

```nix
let
  pkgs = import nixpkgs { inherit system; };
  staticPkgs = pkgs.pkgsStatic;
in
{
  packages.default = staticPkgs.rustPlatform.buildRustPackage {
    pname = "ralph";
    version = "0.1.0";
    src = ./.;
    cargoLock.lockFile = ./Cargo.lock;

    nativeBuildInputs = [ pkgs.git ];
    # No buildInputs — no native C libraries needed

    nativeCheckInputs = [ pkgs.bash ];

    postPatch = ''
      for f in tests/orchestrator.rs tests/backend.rs tests/tail_tmux.rs src/validate/mock_scripts.rs; do
        substituteInPlace "$f" \
          --replace-fail '#!/usr/bin/env bash' '#!${pkgs.bash}/bin/bash'
      done
    '';

    postCheck = ''
      echo "running ralph validate conformance tests..."
      target/*/release/ralph validate --bin target/*/release/ralph
    '';
  };
}
```

Key changes:

1. **Replace `pkgs` with `staticPkgs` (`pkgs.pkgsStatic`)** for the package derivation. This automatically sets the Rust target to `x86_64-unknown-linux-musl` and uses musl-based static toolchain.
2. **Remove `pkgs.openssl` from `buildInputs`** — no crate in the dependency tree uses it.
3. **Remove `pkgs.pkg-config` from `nativeBuildInputs`** — only needed for discovering native libs (OpenSSL), which are gone.
4. **Keep `nativeBuildInputs` items from the *host* `pkgs`** (not `staticPkgs`) since `git` and `bash` are build-host tools, not target libraries.
5. **`devShells.default` stays unchanged** — developers use glibc for fast iteration; the static build is for distribution.

### `.cargo/config.toml` (optional, for local `cargo` commands)

To support `cargo check --target x86_64-unknown-linux-musl` and `cargo test --target x86_64-unknown-linux-musl` outside Nix, add a `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-musl]
linker = "x86_64-unknown-linux-musl-gcc"
```

This file is optional — Nix builds set the linker via environment variables. It enables developers who have `musl-tools` installed to run `cargo test --target x86_64-unknown-linux-musl` directly.

### `postCheck` target path adjustment

With the musl target, `cargo` places binaries under `target/x86_64-unknown-linux-musl/release/` instead of `target/release/`. The existing glob `target/*/release/ralph` already handles this correctly.

## Files & Modules

| File | Action | Description |
|---|---|---|
| `flake.nix` | **Modify** | Switch `packages.default` to use `pkgsStatic.rustPlatform.buildRustPackage`; remove `openssl` and `pkg-config` from inputs; keep `devShells.default` on glibc `pkgs` |
| `.cargo/config.toml` | **Create** (optional) | Set musl linker for local `cargo` commands targeting musl |
| `Cargo.toml` | **No change** | No crate-level changes needed; dependency tree is already pure Rust |

## Testing Strategy

1. **`nix build` smoke test**: Run `nix build` and verify it completes successfully.
2. **Static linking verification**: Run `file result/bin/ralph` — must contain `statically linked`. Run `ldd result/bin/ralph` — must report `not a dynamic executable`.
3. **Conformance tests via Nix**: The existing `postCheck` in the derivation runs `ralph validate --bin ralph` (22 MCP conformance tests + integration tests). These must all pass under the musl build.
4. **Binary functional test**: Run `./result/bin/ralph --help` and `./result/bin/ralph validate --bin ./result/bin/ralph` outside the Nix sandbox to confirm the static binary works on a standard Linux host.
5. **Dev shell regression**: Run `nix develop` and verify `cargo check`, `cargo test`, and `rust-analyzer` still work.
6. **Local musl cargo test** (if `.cargo/config.toml` added): With `musl-tools` installed, run `cargo test --target x86_64-unknown-linux-musl` and verify all tests pass.

## Out of Scope

- **macOS / aarch64-linux static builds**: This spec targets `x86_64-unknown-linux-musl` only. `pkgsStatic` will naturally extend to `aarch64-unknown-linux-musl` on aarch64 systems via `eachDefaultSystem`, but macOS uses different static linking approaches (not musl) and is not addressed.
- **CI pipeline changes**: Updating GitHub Actions or other CI to use the static build target.
- **Container image or release packaging**: Building Docker images or tarballs from the static binary.
- **Removing the glibc dev shell**: `devShells.default` stays glibc-based for IDE tooling compatibility.
- **Vendoring or bundling the musl toolchain outside Nix**: Developers without Nix continue using their system toolchain.
- **Cross-compilation to non-native architectures** (e.g., building aarch64 on x86_64).