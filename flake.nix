{
  description = "Atlas — AI-first workspace platform (dev environment)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
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
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.mold
            pkgs.cargo-nextest
            pkgs.cargo-watch
            pkgs.sea-orm-cli
            pkgs.just
            pkgs.nodejs_22
            pkgs.pnpm
            pkgs.podman
            pkgs.podman-compose
            pkgs.process-compose
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
          shellHook = ''
            echo "Atlas dev shell (Rust 1.96, pnpm, just, podman, sea-orm-cli)"
          '';
        };
        formatter = pkgs.nixpkgs-fmt;
      });
}
