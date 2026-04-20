{
  description = "Compio dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
      in
      with pkgs;
      {
        devShells.default = mkShell {
          buildInputs = [
            nodejs
            gnuplot_qt
            go
            cmake
            glib
            gdb
            lldb
            openssl
            python315
            pkg-config
            cargo-nextest
            cargo-flamegraph
            (rust-bin.selectLatestNightlyWith (
              toolchain:
              toolchain.default.override {
                extensions = [
                  "rust-src"
                  "miri"
                ];
                targets = [
                  "x86_64-unknown-linux-gnu"
                  "x86_64-unknown-freebsd"
                  "x86_64-pc-windows-gnu"
                  "aarch64-apple-darwin"
                ];
              }
            ))
          ];
        };
      }
    );
}
