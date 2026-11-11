{
  description = "Compio dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    devshell.url = "github:numtide/devshell";
    devshell.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      devshell,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [
          (import rust-overlay)
          devshell.overlays.default
        ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
      in
      with pkgs;
      {
        devShells.default = pkgs.devshell.mkShell {
          imports = [ "${devshell}/extra/language/c.nix" ];
          name = "compio";
          language.c = {
            compiler = stdenv.cc;
            includes = [
              glib
              openssl
            ];
          };
          packages = [
            nodejs
            gnuplot_qt
            go
            cmake
            gdb
            lldb
            python315
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
          commands = [
            {
              name = "fmt";
              category = "Development";
              help = "Format the Rust workspace";
              command = ''cargo fmt --all "$@"'';
            }
            {
              name = "fmt-check";
              category = "Checks";
              help = "Check the Rust format without changes";
              command = ''cargo fmt --all --check "$@"'';
            }
            {
              name = "check";
              category = "Checks";
              help = "Check all workspace targets and features";
              command = ''cargo check --workspace --all-features --all-targets "$@"'';
            }
            {
              name = "lint";
              category = "Checks";
              help = "Run Clippy and treat warnings as errors";
              command = ''cargo clippy --workspace --all-features --all-targets "$@" -- -Dwarnings'';
            }
            {
              name = "test-nextest";
              category = "Tests";
              help = "Run workspace tests with nextest";
              command = ''cargo nextest run --workspace --features all,nightly "$@"'';
            }
            {
              name = "test-doc";
              category = "Tests";
              help = "Run workspace documentation tests";
              command = ''cargo test --workspace --doc --features all,nightly "$@"'';
            }
            {
              name = "docs";
              category = "Documentation";
              help = "Build workspace documentation with all features";
              command = ''cargo doc --workspace --all-features --no-deps "$@"'';
            }
          ];
        };
      }
    );
}
