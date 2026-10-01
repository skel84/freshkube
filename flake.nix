{
  description = "Freshkube: native desktop app for Talos Linux and Kubernetes clusters.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
  };

  outputs =
    {
      self,
      nixpkgs,
      systems,
    }:
    let
      inherit (nixpkgs) lib;
      forEachPkgs = f: lib.genAttrs (import systems) (system: f nixpkgs.legacyPackages.${system});

      package =
        {
          lib,
          rustPlatform,
          protobuf,
          pkg-config,
          stdenv,
          libxkbcommon,
          wayland,
        }:
        let
          manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        in
        rustPlatform.buildRustPackage rec {
          inherit (manifest.workspace.package) version;

          pname = "freshkube";
          src = ./.;

          cargoDeps = rustPlatform.importCargoLock {
            lockFile = src + "/Cargo.lock";
          };

          nativeBuildInputs = [
            protobuf
          ] ++ lib.optionals stdenv.isLinux [
            pkg-config
          ];

          buildInputs = lib.optionals stdenv.isLinux [
            libxkbcommon
            wayland
          ];

          meta = {
            description = "Native desktop app for Talos Linux and Kubernetes clusters: node monitoring, log streaming, etcd health, diagnostics and resource browsing";
            homepage = "https://github.com/skel84/talos-pilot";
            license = with lib.licenses; [ mit ];
            mainProgram = "freshkube";
          };
        };
    in
    {
      packages = forEachPkgs (pkgs: rec {
        freshkube = pkgs.callPackage package { inherit (pkgs) protobuf; };
        default = freshkube;
      });
      devShells = forEachPkgs (pkgs: {
        default = pkgs.mkShell {
          # automatically pulls nativeBuildInputs + buildInputs
          inputsFrom = [ (pkgs.callPackage package { inherit (pkgs) protobuf; }) ];
        };
      });
      overlays.default = final: _: {
        freshkube = final.callPackage package { };
      };
    };
}
