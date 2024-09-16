{
  description = "sqlxmq is a message queue built by Diggsey on rust sqlx";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/24.05";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils }:
  flake-utils.lib.eachDefaultSystem (system:
  let
    pkgs = (import "${nixpkgs}" {
      inherit system;
    });

    runDeps = with pkgs; [
      openssl
    ];

    buildDeps = with pkgs; [
      pkg-config
    ] ++ runDeps;
  in rec {
    devShells.devShell.${system} = pkgs.mkShell {
      buildInputs = with pkgs; [
        cargo
        cargo-expand
        rustc
        rust-analyzer
        clippy

        postgresql
        sqlx-cli
      ] ++ buildDeps;
    };
    devShells.default = devShells.devShell.${system};
  });
}
