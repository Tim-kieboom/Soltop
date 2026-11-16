{
  description = "A Task Manager style system monitor for Linux";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # eframe loads these with dlopen at runtime, so they are not found by the linker
      # and have to be put on the library path of the installed program.
      runtimeLibs =
        pkgs: with pkgs; [
          wayland
          libxkbcommon
          vulkan-loader
          libGL
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          fontconfig
        ];
    in
    {
      packages = forAll (pkgs: rec {
        soltop = pkgs.rustPlatform.buildRustPackage {
          pname = "soltop";
          version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.makeWrapper
            pkgs.copyDesktopItems
          ];
          buildInputs = runtimeLibs pkgs;

          # The entry that makes Soltop show up in application launchers.
          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "soltop";
              desktopName = "Soltop";
              genericName = "System Monitor";
              comment = "A Task Manager style system monitor";
              exec = "Soltop";
              icon = "utilities-system-monitor"; # a theme icon, Soltop has none of its own yet
              terminal = false;
              categories = [
                "System"
                "Monitor"
              ];
              # the app id the window announces, so launchers match the window to this entry
              startupWMClass = "Soltop";
            })
          ];

          # Two tests scan the machine they run on (processes, disks, the scan thread),
          # which the build sandbox does not suit.
          doCheck = false;

          # /run/opengl-driver/lib is where NixOS puts the graphics driver, and with it
          # libnvidia-ml, which the GPU monitoring loads when an NVIDIA GPU is present.
          postInstall = ''
            wrapProgram $out/bin/Soltop \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)} \
              --suffix LD_LIBRARY_PATH : /run/opengl-driver/lib
          '';

          meta = {
            description = "A Task Manager style system monitor for Linux";
            homepage = "https://github.com/Tim-kieboom/Soltop";
            mainProgram = "Soltop";
            platforms = pkgs.lib.platforms.linux;
          };
        };
        default = soltop;
      });

      apps = forAll (pkgs: rec {
        soltop = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.soltop}/bin/Soltop";
        };
        default = soltop;
      });

      devShells = nixpkgs.lib.genAttrs systems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true; # required for the NVIDIA packages
          };
          # The NVIDIA driver package only exists for x86_64 here.
          nvidia = pkgs.lib.optionals (system == "x86_64-linux") [
            pkgs.linuxPackages.nvidiaPackages.stable
          ];
          libs =
            runtimeLibs pkgs
            ++ [
              pkgs.rustc
              pkgs.cargo
              pkgs.protobuf
              pkgs.libdrm
            ]
            ++ nvidia;
        in
        {
          default = pkgs.mkShell {
            name = "soltop-shell";
            buildInputs = libs;

            LIBRARY_PATH = pkgs.lib.concatStringsSep ":" (
              map (p: "${p}/lib") (nvidia ++ [ pkgs.libdrm ]) ++ [ "/run/opengl-driver/lib" ]
            );
            LD_LIBRARY_PATH = "/run/opengl-driver/lib:${pkgs.lib.makeLibraryPath libs}";
            VK_DRIVER_FILES = "/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json";
          };
        }
      );
    };
}
