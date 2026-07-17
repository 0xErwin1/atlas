{
  description = "Atlas — AI-first workspace platform (dev environment)";

  inputs = {
    devenv-root = {
      url = "file+file:///dev/null";
      flake = false;
    };
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    devenv.url = "github:cachix/devenv";
  };

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };

  outputs =
    inputs@{ flake-parts, nixpkgs, rust-overlay, flake-utils, devenv, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.devenv.flakeModule ];

      systems = flake-utils.lib.defaultSystems;

      perSystem = { system, ... }:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          rustToolchain = pkgs.rust-bin.stable."1.96.0".default.override {
            extensions = [ "rustfmt" "clippy" "rust-analyzer" "rust-src" ];
          };
          tauriDriver = pkgs.rustPlatform.buildRustPackage {
            pname = "tauri-driver";
            version = "2.0.6";

            src = pkgs.fetchCrate {
              pname = "tauri-driver";
              version = "2.0.6";
              hash = "sha256-fTCkEs4NLBW0khaHL4jpVNkrbQg22YPsRMjfJNqnCWA=";
            };

            cargoHash = "sha256-MThAcU+U8PyBGauh3dy7ZRvRX9INmOEeghIlQEGLAPs=";
          };
          atlasDbusRunSession = pkgs.writeShellScriptBin "atlas-dbus-run-session" ''
            umask 077
            exec ${pkgs.dbus}/bin/dbus-run-session \
              --config-file=${pkgs.dbus}/share/dbus-1/session.conf "$@" 2>/dev/null
          '';

          desktopGateSubcommands =
            "red|full|asset-audit|tooling|release-audit|launch|controller-test|webdriver-test|host-test";

          rustPlatformPinned = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };

          webDist = pkgs.callPackage ./nix/frontend.nix { src = ./.; };
        in
        {
          packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            atlas-desktop = pkgs.callPackage ./nix/atlas-desktop.nix {
              inherit webDist;
              rustPlatform = rustPlatformPinned;
              src = ./.;
            };
          };

          devenv.shells.default = {
            packages = [
              rustToolchain
              pkgs.mold
              pkgs.cargo-nextest
              pkgs.cargo-watch
              pkgs.nodejs_22
              pkgs.pnpm
              pkgs.podman
              pkgs.curl
              pkgs.openssl
              pkgs.actionlint
              pkgs.cargo-tauri
              pkgs.glib-networking
              pkgs.libsecret
              pkgs.pkg-config
            ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              tauriDriver
              atlasDbusRunSession
              pkgs.dbus
              pkgs.gnome-keyring
              pkgs.webkitgtk_4_1
              pkgs.xorg-server
            ];

            # .env is deliberately NOT loaded here, and `dotenv.enable` must not
            # be turned on. The shell's only consumers are tests, and tests must
            # not inherit ambient server configuration: fixtures CREATE and
            # force-DROP databases against whatever connection string they are
            # handed, and several assert on a setting being absent (see
            # crates/atlas_server/tests/api_health.rs). Under edition 2024 with
            # unsafe_code = forbid, a test cannot unset a variable to defend
            # itself, so the only safe point of control is to never set it. The
            # deployed server takes its environment from the deployment.
            enterShell = ''
              # testcontainers only auto-detects rootless *Docker* socket paths,
              # none of which match podman's, so it needs DOCKER_HOST spelled out.
              # An existing value wins: it is the escape hatch for a remote or
              # non-podman runtime.
              if [ -z "''${DOCKER_HOST:-}" ] && [ -S "/run/user/$(id -u)/podman/podman.sock" ]; then
                export DOCKER_HOST="unix:///run/user/$(id -u)/podman/podman.sock"
              fi

              echo "Atlas dev shell (Rust 1.96, pnpm, devenv, podman)"
            '';

            scripts = {
              check.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo check --workspace
              '';

              clippy.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo clippy --workspace --all-targets -- -D warnings
              '';

              # Named `format`, not `fmt`: nixpkgs' stdenv sits ahead of devenv's
              # scripts on PATH, so a script called `fmt` resolves to coreutils'
              # and is unreachable under its own name.
              format.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo fmt --all
                pnpm exec biome format --write .
              '';

              fmt-check.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo fmt --all -- --check
              '';

              # cargo nextest is process-per-test, so the container that
              # atlas_test_harness starts must live one level above the test
              # runner it wraps; see crates/atlas_test_harness/src/main.rs.
              tests.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo run --quiet -p atlas_test_harness -- \
                  bash -c 'cargo nextest run --workspace && cargo test --doc --workspace'
              '';

              build.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo build --workspace
              '';

              lint-web.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                pnpm exec biome ci .
              '';

              build-web.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                gen-types
                pnpm --filter @atlas/web build
              '';

              verify.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                fmt-check
                clippy
                tests
                build
                lint-web
                build-web
              '';

              gen-types.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                cargo run -p atlas_server --bin dump_openapi > apps/web/openapi.json
                pnpm --filter @atlas/web exec openapi-typescript openapi.json -o src/api/types.d.ts
              '';

              desktop-dev.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT/apps/desktop/src-tauri"
                cargo tauri dev
              '';

              # Collapses the 7 desktop-gate-* just recipes, the bare
              # desktop-gate recipe (subcommand "full"), and desktop-host-test
              # (subcommand "host-test") into one dispatcher. Only the
              # subcommands that reach TlsGateServer::spawn() — full, launch,
              # controller-test, webdriver-test, host-test — route through
              # atlas_test_harness; the rest are audit-only and start no
              # container.
              desktop-gate.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"

                subcommand="''${1:-}"
                if [ "$#" -gt 0 ]; then
                  shift
                fi

                run_through_harness() {
                  cargo run --quiet -p atlas_test_harness -- "$@"
                }

                case "$subcommand" in
                  red)
                    bash apps/desktop/tests/test_linux_gate_harness.sh
                    ;;
                  full)
                    run_through_harness bash apps/desktop/tests/linux_gate.sh --evidence \
                      "''${ATLAS_DESKTOP_GATE_EVIDENCE_PATH:-/tmp/atlas-desktop-gate-evidence.json}"
                    ;;
                  asset-audit)
                    bash apps/desktop/tests/linux_gate.sh --asset-audit
                    ;;
                  tooling)
                    bash apps/desktop/gate/test_tauri_driver_tooling.sh
                    ;;
                  release-audit)
                    bash apps/desktop/gate/audit_release_exclusion.sh
                    ;;
                  launch)
                    run_through_harness bash apps/desktop/gate/run_webdriver_launch.sh
                    ;;
                  controller-test)
                    VITE_ATLAS_DESKTOP_GATE=1 pnpm --filter @atlas/web build
                    cargo build -p atlas_desktop --features desktop-gate --bin atlas-desktop-gate
                    run_through_harness cargo nextest run -p atlas_desktop --features desktop-gate --test gate_controller
                    ;;
                  webdriver-test)
                    VITE_ATLAS_DESKTOP_GATE=1 pnpm --filter @atlas/web build
                    cargo build -p atlas_desktop --features desktop-gate --bin atlas-desktop-gate
                    run_through_harness cargo nextest run -p atlas_desktop --features desktop-gate --test gate_controller controller_drives_the_packaged_vue_webdriver_login_and_restart_flow
                    ;;
                  host-test)
                    run_through_harness bash apps/desktop/tests/test_desktop_host.sh
                    ;;
                  *)
                    echo "usage: desktop-gate {${desktopGateSubcommands}}" >&2
                    exit 2
                    ;;
                esac
              '';

              deploy.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                bash deploy/deploy.sh
              '';

              build-images.exec = ''
                set -euo pipefail
                cd "$DEVENV_ROOT"
                TAG="''${1:-local}" bash deploy/build-images.sh
              '';
            };
          };

          formatter = pkgs.nixpkgs-fmt;
        };

      flake.homeManagerModules.atlas-desktop =
        { pkgs, lib, ... }:
        {
          imports = [ ./nix/home-manager.nix ];
          config.programs.atlas-desktop.package =
            lib.mkDefault inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.atlas-desktop;
        };
    };
}
