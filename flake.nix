{
  description = "agent-task: AI エージェント向け SQLite タスク管理 CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        agent-task = pkgs.rustPlatform.buildRustPackage {
          pname = "agent-task";
          version = "0.1.0";

          src = pkgs.lib.cleanSource ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ];

          # rusqlite's "bundled" feature compiles its own sqlite3 from source,
          # so the crate build itself needs no external sqlite — but a C
          # toolchain is still required, which buildRustPackage provides.
          doCheck = true;

          meta = with pkgs.lib; {
            description = "AI エージェント向け SQLite タスク管理 CLI";
            homepage = "https://github.com/yuarth/agent-task";
            license = licenses.mit;
            mainProgram = "agent-task";
          };
        };
      in
      {
        packages.default = agent-task;
        packages.agent-task = agent-task;

        apps.default = flake-utils.lib.mkApp { drv = agent-task; };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            pkgs.cargo
            pkgs.rustc
            pkgs.pkg-config
            pkgs.rust-analyzer
            pkgs.rustfmt
            pkgs.clippy
          ];
        };

        checks.default = agent-task;
      });
}
