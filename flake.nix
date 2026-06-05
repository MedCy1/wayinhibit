{
  description = "A small Wayland idle inhibitor written in Rust";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          version = manifest.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          postInstall = ''
            installManPage man/wayinhibit.1
            installShellCompletion --bash completions/wayinhibit.bash
            installShellCompletion --zsh completions/_wayinhibit
            installShellCompletion --fish completions/wayinhibit.fish
          '';

          meta = with pkgs.lib; {
            description = manifest.description;
            homepage = manifest.homepage;
            license = licenses.mit;
            maintainers = [];
            platforms = platforms.linux;
            mainProgram = "wayinhibit";
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [ cargo rustc rustfmt clippy ];
        };
      });
}
