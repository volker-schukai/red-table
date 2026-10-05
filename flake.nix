{
  description = "red-table - a performance-first, keyboard-driven terminal image browser";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    supportedSystems = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    projectVersion = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
    forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    pkgsFor = system: import nixpkgs {inherit system;};
  in {
    packages = forAllSystems (
      system: let
        pkgs = pkgsFor system;
      in {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "red-table";
          version = projectVersion;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          meta = {
            description = "Fast, keyboard-driven terminal image browser";
            mainProgram = "red-table";
            platforms = supportedSystems;
          };
        };

        yazi-plugin = pkgs.stdenvNoCC.mkDerivation {
          pname = "red-table-yazi";
          version = projectVersion;
          src = ./red-table.yazi;
          dontBuild = true;

          installPhase = ''
            runHook preInstall
            mkdir -p "$out"
            cp main.lua README.md LICENSE "$out/"
            runHook postInstall
          '';

          meta = {
            description = "Yazi selection bridge for red-table";
            license = pkgs.lib.licenses.mit;
            platforms = supportedSystems;
          };
        };
      }
    );

    apps = forAllSystems (system: {
      default = {
        type = "app";
        program = "${self.packages.${system}.default}/bin/red-table";
        meta.description = "Run the red-table terminal image browser";
      };
    });

    checks = forAllSystems (system: {
      inherit (self.packages.${system}) default yazi-plugin;
    });

    devShells = forAllSystems (
      system: let
        pkgs = pkgsFor system;
        docs = pkgs.python3.withPackages (pythonPackages: [
          pythonPackages.mkdocs
          pythonPackages.mkdocs-material
        ]);
      in {
        default = pkgs.mkShell {
          inputsFrom = [self.packages.${system}.default];

          packages = with pkgs; [
            alejandra
            bashInteractive
            cargo
            cargo-audit
            cargo-deny
            cargo-nextest
            clippy
            coreutils
            direnv
            docs
            expect
            ffmpeg-full
            figlet
            findutils
            git
            gawk
            gnutar
            gnugrep
            gnused
            go-task
            nix
            nix-direnv
            pkg-config
            lua5_4
            imagemagick
            kitty
            rust-analyzer
            rustc
            rustfmt
            stylua
            xdotool
            xorg-server
            yazi
          ];

          RUST_BACKTRACE = "1";

          shellHook = ''
            # The invoking shell may carry a dynamic-linker search path from
            # another environment. It would shadow this closure's libraries,
            # so the shell owns the search path and drops the inherited value.
            unset LD_LIBRARY_PATH
            ${pkgs.figlet}/bin/figlet -w 120 "red-table"
            printf '\n'
            printf '  Rust image browser | performance first | keyboard driven\n'
            printf '  Run "task" to list the available commands.\n\n'
          '';
        };
      }
    );

    formatter = forAllSystems (system: (pkgsFor system).alejandra);
  };
}
