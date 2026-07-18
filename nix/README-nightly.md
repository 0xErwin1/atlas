# Installing the nightly desktop build

CI compiles the desktop app on every push to `main` and publishes an AppImage to
a rolling `nightly` prerelease. The `atlas-desktop-nightly` flake package installs
that prebuilt binary — it never compiles locally. The AppImage's hash is pinned on
the `nightly` git ref, so always consume through that ref, not `main`.

## Run it once

```sh
nix run github:0xErwin1/atlas/nightly#atlas-desktop-nightly
```

## Install via home-manager

```nix
{
  inputs.atlas.url = "github:0xErwin1/atlas/nightly";

  # in your home-manager configuration:
  imports = [ inputs.atlas.homeManagerModules.atlas-desktop ];
  programs.atlas-desktop = {
    enable = true;
    package = inputs.atlas.packages.${pkgs.system}.atlas-desktop-nightly;
  };
}
```

This adds "Atlas Desktop" to the application menu with its icon and never triggers
a local build — Nix downloads the AppImage the CI produced. The repository is
public, so both the flake source and the AppImage download need no authentication.
