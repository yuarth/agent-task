{ pkgs ? import <nixpkgs> { } }:

pkgs.rustPlatform.buildRustPackage {
  pname = "agent-task";
  version = "0.1.0";

  src = pkgs.lib.cleanSource ./.;

  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ pkgs.sqlite ];

  doCheck = true;

  meta = with pkgs.lib; {
    description = "AI エージェント向け SQLite タスク管理 CLI";
    homepage = "https://github.com/yuarth/agent-task";
    license = licenses.mit;
    mainProgram = "agent-task";
  };
}
