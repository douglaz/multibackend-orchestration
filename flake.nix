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
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "ralph";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.git
          ];

          nativeCheckInputs = [
            pkgs.bash
          ];

          buildInputs = [
            pkgs.openssl
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
            pkg-config
            openssl
            git
          ];

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
}
