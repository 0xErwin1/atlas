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
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.mold
            pkgs.cargo-nextest
            pkgs.sea-orm-cli
            pkgs.just
            pkgs.nodejs_22
            pkgs.pnpm
            pkgs.podman
            pkgs.podman-compose
            pkgs.actionlint
          ];
          shellHook = ''
            echo "Atlas dev shell (Rust 1.96, pnpm, just, podman, sea-orm-cli)"
          '';
        };
        formatter = pkgs.nixpkgs-fmt;
      });
}
