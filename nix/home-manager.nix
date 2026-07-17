{ config, lib, ... }:

let
  cfg = config.programs.atlas-desktop;
in
{
  options.programs.atlas-desktop = {
    enable = lib.mkEnableOption "Atlas desktop app";
    package = lib.mkOption {
      type = lib.types.package;
      description = "The atlas-desktop package to install.";
    };
  };

  # The package ships its own `.desktop` entry and icon under `share/`, which
  # home-manager merges into XDG_DATA_DIRS — no separate xdg.desktopEntries.
  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];
  };
}
