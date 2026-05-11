{
  description = "Dumbgram TUI - terminal Telegram client";

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
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        packageName = cargoToml.package.name;
        packageVersion = cargoToml.package.version;

        rustToolchain = pkgs.rust-bin.stable.latest.default;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        dumbgram = rustPlatform.buildRustPackage {
          pname = packageName;
          version = packageVersion;
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];

          doCheck = false;

          meta = with pkgs.lib; {
            description = "Terminal Telegram client built with Ratatui and Grammers";
            homepage = "https://github.com/DeevsDeevs/dumbgram-tui";
            mainProgram = packageName;
            platforms = platforms.linux ++ platforms.darwin;
          };
        };
      in
      {
        packages = {
          default = dumbgram;
          dumbgram_tui = dumbgram;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = dumbgram;
        };

        apps.dumbgram_tui = flake-utils.lib.mkApp {
          drv = dumbgram;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustToolchain
            pkg-config
            rust-analyzer
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];

          RUST_BACKTRACE = "1";
          CARGO_TERM_COLOR = "always";
        };
      }
    );
}
