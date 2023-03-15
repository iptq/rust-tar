{
  inputs = { fenix.url = "github:nix-community/fenix"; };

  outputs = { self, nixpkgs, flake-utils, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default ];
        };

        toolchain = pkgs.fenix.stable;
      in rec {
        devShell = pkgs.mkShell {
          packages = (with pkgs; [
            cargo-deny
            cargo-edit
            cargo-watch
          ]) ++ (with toolchain; [
            cargo
            rustc
            clippy

            # Get the nightly version of rustfmt so we can wrap comments
            pkgs.fenix.default.rustfmt
          ]);
        };
      });
}
