{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    pkg-config
    wayland
    libxkbcommon
  ];

shellHook = ''
  export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
    pkgs.wayland
    pkgs.libxkbcommon
    pkgs.libglvnd
  ]}:$LD_LIBRARY_PATH
'';
}
