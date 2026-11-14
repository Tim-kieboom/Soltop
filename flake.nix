{
  description = "eframe devShell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:

    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true; # ← required for NVIDIA packages
        };
        nvidia = pkgs.linuxPackages.nvidiaPackages.stable; # ← correct attr path
        libs = with pkgs; [
          wayland
          libxkbcommon
          vulkan-loader
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          fontconfig
          rustc
          cargo
          protobuf
          nvidia
          libdrm
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          name = "eframe-shell";
          buildInputs = libs;

          LIBRARY_PATH = "${nvidia}/lib:${pkgs.libdrm}/lib:/run/opengl-driver/lib";
          LD_LIBRARY_PATH = "/run/opengl-driver/lib:${pkgs.lib.makeLibraryPath libs}";
          VK_DRIVER_FILES = "/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json";
        };
      }
    );
}
