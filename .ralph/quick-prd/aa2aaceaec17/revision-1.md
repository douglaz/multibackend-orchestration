So 80 test registrations (minus 1 for the struct definition in runner.rs = 80 total tests). The flake comment says "40" which is already stale. This confirms review issue #4 about hard-coded counts drifting.

Now I have everything I need. Here's the revised specification:

---

## Summary

Switch the default `nix build` output to produce a fully static, portable `ralph` binary linked against musl libc. The current `flake.nix` uses `pkgs.rustPlatform.buildRustPackage` with glibc and an unused OpenSSL dependency. Replace this with a musl-based build using `pkgsStatic` so the resulting binary has zero dynamic library dependencies. Restrict the static build to Linux systems only; Darwin outputs fall back to the existing glibc-based (dynamically linked) build since musl is Linux-only. Add an automated static-linking assertion inside the Nix derivation so regressions are caught without manual inspection. Provide a dedicated `devShells.musl` for running `cargo check` and `cargo test` against the musl target inside Nix, rather than relying on externally-installed `musl-tools`. The crate dependency tree is already pure Rust (no native C library linkage), making this straightforward.

## Acceptance Criteria

1. On Linux: `nix build` produces a statically-linked binary at `result/bin/ralph`.
2. On Linux: `file result/bin/ralph` reports `statically linked`.
3. On Linux: `ldd result/bin/ralph` reports `not a dynamic executable` (or exits non-zero).
4. On Linux: the Nix derivation's `postInstall` phase automatically verifies static linking; the build fails if the binary is not fully static.
5. `nix build` succeeds including the existing `postCheck` conformance tests (`ralph validate`).
6. On Linux: `nix develop .#musl -c cargo check --target x86_64-unknown-linux-musl` succeeds.
7. On Linux: `nix develop .#musl -c cargo test --target x86_64-unknown-linux-musl` succeeds (all existing tests pass).
8. The `devShells.default` continues to work for local development (remains glibc-based, unchanged).
9. No OpenSSL dependency remains in the build (it is unused).
10. On Darwin: `nix build` continues to produce a working (dynamically linked) binary using the existing glibc-based derivation. Darwin outputs are unaffected by the musl change.

## Technical Approach

### Current State

- `flake.nix` uses `pkgs.rustPlatform.buildRustPackage` with the default glibc toolchain.
- `buildInputs` includes `pkgs.openssl` — unused (no TLS crates in the dependency tree).
- All Rust dependencies are pure Rust: `serde_yaml` (via `unsafe-libyaml`, a pure Rust libyaml port), `chrono`, `tokio`, `clap`, `serde_json`, `regex`, etc. The `-sys` crates present (`linux-raw-sys`, `core-foundation-sys`, `windows-sys`) define constants/types only and do not link any C libraries (confirmed: no `links =` entries in `Cargo.lock`).
- `flake-utils.lib.eachDefaultSystem` iterates over four systems: `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin`.

### Approach: Platform-Gated `pkgsStatic` with Darwin Fallback

Musl is Linux-only. Making `packages.default = pkgsStatic.rustPlatform.buildRustPackage` unconditionally across `eachDefaultSystem` would break or mis-specify Darwin outputs. Gate the derivation by platform:

```nix
{
  description = "Ralph Loop orchestration tool";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        isLinux = pkgs.lib.hasSuffix "-linux" system;

        commonArgs = {
          pname = "ralph";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.git ];
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

        # Linux: static musl build
        staticPkg = pkgs.pkgsStatic.rustPlatform.buildRustPackage (commonArgs // {
          postInstall = ''
            echo "verifying static linkage..."
            if file $out/bin/ralph | grep -q "statically linked"; then
              echo "OK: binary is statically linked"
            else
              echo "FAIL: binary is NOT statically linked"
              file $out/bin/ralph
              exit 1
            fi
          '';
        });

        # Darwin: standard dynamic build (musl is Linux-only)
        dynamicPkg = pkgs.rustPlatform.buildRustPackage commonArgs;

      in
      {
        packages.default = if isLinux then staticPkg else dynamicPkg;

        # Expose both variants on Linux for explicit selection
        packages = pkgs.lib.optionalAttrs isLinux {
          static = staticPkg;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            git
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      } // pkgs.lib.optionalAttrs isLinux {
        # Dedicated musl dev shell for cargo check/test --target x86_64-unknown-linux-musl
        devShells.musl = pkgs.mkShell {
          packages = [
            pkgs.pkgsStatic.rustPlatform.rust.cargo
            pkgs.pkgsStatic.rustPlatform.rust.rustc
            pkgs.pkgsStatic.stdenv.cc        # musl-gcc linker
            pkgs.git
            pkgs.bash
          ];
          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
}
```

Key changes from the original spec:

1. **Platform gating** (`isLinux`): On Linux, `packages.default` is the static musl build. On Darwin, it falls back to the standard dynamic build. This prevents `eachDefaultSystem` from producing broken musl derivations on macOS.
2. **`postInstall` static-link assertion**: The Linux derivation automatically verifies the binary is statically linked using `file` and fails the build if not. This catches regressions without manual inspection.
3. **Remove `pkgs.openssl` from `buildInputs`** and **`pkgs.pkg-config` from `nativeBuildInputs`** — no crate in the dependency tree uses them.
4. **Remove `openssl` and `pkg-config` from `devShells.default`** — they are unused there as well.
5. **Keep `nativeBuildInputs` items from the host `pkgs`** (not `pkgsStatic`) since `git` and `bash` are build-host tools, not target libraries.
6. **`devShells.default` stays unchanged** (minus unused openssl/pkg-config) — developers use glibc for fast iteration and IDE tooling.
7. **Add `devShells.musl`** (Linux only): A dedicated dev shell that provisions the musl Rust toolchain and musl-gcc linker via Nix. This makes `cargo check --target x86_64-unknown-linux-musl` and `cargo test --target x86_64-unknown-linux-musl` work without any external `musl-tools` installation. The shell sets `CARGO_BUILD_TARGET` so plain `cargo check` / `cargo test` also target musl automatically.
8. **`commonArgs` factoring**: Shared derivation arguments (src, cargoLock, postPatch, postCheck) are factored into a shared attrset to avoid duplication between the static and dynamic derivations.
9. **Remove hard-coded test count from the `postCheck` comment**: The conformance suite grows over time (currently 80 tests); the comment should not specify a count. The `ralph validate` command reports its own summary.

### `.cargo/config.toml` — Not Created

The original spec proposed an optional `.cargo/config.toml` that sets the musl linker. This is no longer needed: the `devShells.musl` provisions the linker via Nix and sets the correct environment. Creating a `.cargo/config.toml` could interfere with the default glibc dev shell (cargo would try to use the musl linker even without the musl shell) unless the config is target-scoped. Since the musl shell handles this via environment variables, no config file is needed.

### `postCheck` target path

With the musl target, `cargo` places binaries under `target/x86_64-unknown-linux-musl/release/` instead of `target/release/`. The existing glob `target/*/release/ralph` already handles this correctly.

## Files & Modules

| File | Action | Description |
|---|---|---|
| `flake.nix` | **Modify** | Platform-gate `packages.default`: Linux uses `pkgsStatic.rustPlatform.buildRustPackage` (static musl), Darwin uses `pkgs.rustPlatform.buildRustPackage` (dynamic). Remove `openssl` and `pkg-config` from all build inputs and dev shell. Add `postInstall` static-link assertion to the Linux derivation. Add `devShells.musl` (Linux only) with musl Rust toolchain. Factor shared args into `commonArgs`. Remove stale test-count comment. |
| `Cargo.toml` | **No change** | Dependency tree is already pure Rust; no crate-level changes needed. |
| `.cargo/config.toml` | **Not created** | Not needed; `devShells.musl` provisions the linker via environment. |

## Testing Strategy

1. **`nix build` on Linux**: Run `nix build` and verify it completes successfully. The `postInstall` phase automatically asserts the binary is statically linked — if it isn't, the build fails.
2. **Static linking verification (manual, supplementary)**: Run `file result/bin/ralph` — must contain `statically linked`. Run `ldd result/bin/ralph` — must report `not a dynamic executable` (note: `ldd` exits non-zero for static binaries on some systems; use `ldd result/bin/ralph || true` in scripts).
3. **Conformance tests via Nix**: The `postCheck` phase runs `ralph validate --bin ralph`. All conformance tests must pass under the musl build. No hard-coded test count is asserted; the `ralph validate` runner reports pass/fail/skip counts dynamically.
4. **Binary functional test**: Outside the Nix sandbox, run `./result/bin/ralph --help` and `./result/bin/ralph validate --bin ./result/bin/ralph` to confirm the static binary works on a standard Linux host without any Nix runtime dependencies.
5. **Dev shell regression (`devShells.default`)**: Run `nix develop` and verify `cargo check`, `cargo test`, `cargo clippy`, and `rust-analyzer` still work. Confirm the removal of `openssl`/`pkg-config` causes no breakage.
6. **Musl dev shell (`devShells.musl`)**: On Linux, run `nix develop .#musl` and verify:
   - `cargo check --target x86_64-unknown-linux-musl` succeeds.
   - `cargo test --target x86_64-unknown-linux-musl` succeeds with all tests passing.
   - Plain `cargo check` and `cargo test` also target musl (via `CARGO_BUILD_TARGET`).
7. **Darwin `nix build` (if Darwin runner available)**: Run `nix build` on macOS and verify it produces a working dynamic binary. Confirm no musl-related errors.

## Out of Scope

- **macOS static builds**: Darwin uses a different static linking approach (not musl). The Darwin output remains dynamically linked. Producing fully static Darwin binaries is a separate effort.
- **CI pipeline changes**: Updating GitHub Actions or other CI to use the static build target or the `devShells.musl`.
- **Container image or release packaging**: Building Docker images or tarballs from the static binary.
- **Removing the glibc dev shell**: `devShells.default` stays glibc-based for IDE tooling compatibility and fast iteration.
- **Vendoring or bundling the musl toolchain outside Nix**: Developers without Nix continue using their system toolchain.
- **Cross-compilation to non-native architectures** (e.g., building aarch64 on x86_64): `pkgsStatic` targets the native system architecture. Cross-arch builds are not addressed.
- **aarch64-linux musl**: While `pkgsStatic` on `aarch64-linux` will naturally produce `aarch64-unknown-linux-musl` binaries, this spec only explicitly validates `x86_64-unknown-linux-musl`. aarch64 musl should work but is not tested as part of this change.