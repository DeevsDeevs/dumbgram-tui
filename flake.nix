{
  description = "Dumbgram TUI - terminal Telegram client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          packageName = cargoToml.package.name;
          packageVersion = cargoToml.package.version;
          dumbgram = pkgs.rustPlatform.buildRustPackage {
            pname = packageName;
            version = packageVersion;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.pkg-config ];

            meta = with pkgs.lib; {
              description = "Terminal Telegram client built with Ratatui and Grammers";
              homepage = "https://github.com/deevus/dumbgram-tui";
              mainProgram = packageName;
              platforms = platforms.linux ++ platforms.darwin;
            };
          };
        in
        {
          default = dumbgram;
          dumbgram_tui = dumbgram;
        });

      apps = forAllSystems (system:
        let
          package = self.packages.${system}.default;
        in
        {
          default = {
            type = "app";
            program = "${package}/bin/dumbgram_tui";
          };
          dumbgram_tui = self.apps.${system}.default;
        });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              pkg-config
              rust-analyzer
              rustc
              rustfmt
            ];

            RUST_BACKTRACE = "1";
            CARGO_TERM_COLOR = "always";
          };
        });
    };
}
