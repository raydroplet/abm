{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    wayland
    libxkbcommon

    # Use Nix-native Rust instead of rustup to avoid FHS wrapper issues
    # cargo
    rustc
    # clippy
    # rustfmt

    # Provide the exact linkers the project's config demands
    clang
    lld
    pkg-config

    # Audio backends
    alsa-lib
    libpulseaudio
    pipewire
  ];

shellHook = ''
  export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
    pkgs.wayland
    pkgs.libxkbcommon
    pkgs.libglvnd
  ]}:$LD_LIBRARY_PATH
'';
}
