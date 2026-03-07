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
        isLinux = pkgs.stdenv.hostPlatform.isLinux;

        commonArgs = {
          pname = "ralph";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = [
            pkgs.git
          ];

          nativeCheckInputs = [
            pkgs.bash
            pkgs.rustfmt
            pkgs.clippy
          ];

          # Tests generate mock backend scripts at runtime with
          # #!/usr/bin/env bash shebangs. The sandbox doesn't provide
          # /usr/bin/env, so patch the shebang string in the test source
          # to use the Nix store bash path directly.
          postPatch = ''
            for f in tests/orchestrator.rs tests/backend.rs tests/tail_tmux.rs src/validate/mock_scripts.rs; do
              substituteInPlace "$f" \
                --replace-fail '#!/usr/bin/env bash' '#!${pkgs.bash}/bin/bash'
            done
          '';

          # Run conformance tests against the built binary
          postCheck = ''
            echo "running ralph validate conformance tests..."
            target/*/release/ralph validate --bin target/*/release/ralph
          '';
        };

        staticPackage = pkgs.pkgsStatic.rustPlatform.buildRustPackage (commonArgs // {
          postInstall = ''
            echo "verifying static linkage..."
            file_output="$(${pkgs.file}/bin/file "$out/bin/ralph")"
            echo "$file_output"
            if ! echo "$file_output" | grep -Eq "statically linked|static-pie linked"; then
              echo "FAIL: binary is NOT statically linked"
              exit 1
            fi
          '';
        });

        dynamicPackage = pkgs.rustPlatform.buildRustPackage commonArgs;
      in
      {
        packages =
          {
            default = if isLinux then staticPackage else dynamicPackage;
            dynamic = dynamicPackage;
          }
          // pkgs.lib.optionalAttrs isLinux {
            static = staticPackage;
          };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells =
          {
            default = pkgs.mkShell {
              packages = with pkgs; [
                cargo
                rustc
                rustfmt
                clippy
                rust-analyzer
                git
                gh
                jq
              ];

              RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

              shellHook = ''
                if [ -d .git ] && [ -d .githooks ]; then
                  current_hooks_path=$(git config core.hooksPath || echo "")
                  if [ "$current_hooks_path" != ".githooks" ]; then
                    git config core.hooksPath .githooks
                    echo "Git hooks configured (.githooks)"
                    echo "  pre-commit: cargo fmt --check"
                    echo "  pre-push:   cargo fmt + clippy + nix build"
                    echo "  Disable: git config --unset core.hooksPath"
                  fi
                fi
              '';
            };
          }
          // pkgs.lib.optionalAttrs isLinux {
            musl = pkgs.mkShell {
              packages = [
                pkgs.pkgsStatic.buildPackages.cargo
                pkgs.pkgsStatic.buildPackages.rustc
                pkgs.pkgsStatic.stdenv.cc
                pkgs.git
                pkgs.bash
              ];

              CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER =
                "${pkgs.pkgsStatic.stdenv.cc}/bin/${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
              RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            };
          };
      });
}
