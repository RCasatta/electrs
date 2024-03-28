{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };
          rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

          src = craneLib.cleanCargoSource ./.; 

          nativeBuildInputs = with pkgs; [ rustToolchain clang ]; # required only at build time
          buildInputs = with pkgs; [ ]; # also required at runtime

          commonArgs = {
            inherit src buildInputs nativeBuildInputs;
            cargoExtraArgs = "--no-default-features --features liquid";  # avoid autodownload feature, which cannot happen in nix for deterministism
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          bin = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            # doCheck = false;
          });

        in
        with pkgs;
        {
          packages =
            {
              # that way we can build `bin` specifically,
              # but it's also the default.
              inherit bin;
              default = bin;
            };

          devShells.default = mkShell {
            inputsFrom = [ bin ];
          };
        }
      );
}
