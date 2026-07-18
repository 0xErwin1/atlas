# Installing the nightly desktop build

CI compiles the desktop app on every push to `main` and publishes an AppImage to
a rolling `nightly` prerelease. The `atlas-desktop-nightly` flake package installs
that prebuilt binary — it never compiles locally. The AppImage's hash is pinned on
the `nightly` git ref, so always consume through that ref, not `main`.

The repository is private, which affects the two fetches Nix makes:

- **The flake source** — fetch it over SSH with your existing key (no token):
  `git+ssh://git@github.com/0xErwin1/atlas?ref=nightly`.
- **The AppImage asset** — served over HTTPS, so the SSH key does not apply.
  A private release asset needs a GitHub token via `netrc-file` (below), or the
  asset must be hosted somewhere unauthenticated (see "Zero-token alternative").

## Run it once

```sh
nix run 'git+ssh://git@github.com/0xErwin1/atlas?ref=nightly#atlas-desktop-nightly'
```

## Install via home-manager

```nix
{
  inputs.atlas.url = "git+ssh://git@github.com/0xErwin1/atlas?ref=nightly";

  # in your home-manager configuration:
  imports = [ inputs.atlas.homeManagerModules.atlas-desktop ];
  programs.atlas-desktop = {
    enable = true;
    package = inputs.atlas.packages.${pkgs.system}.atlas-desktop-nightly;
  };
}
```

This adds "Atlas Desktop" to the application menu with its icon and never triggers
a local build — Nix downloads the AppImage the CI produced.

## Token for the asset download

The `git+ssh://` source needs no token, but the AppImage `fetchurl` still has to
authenticate to `github.com`. Configure a GitHub token with `repo` scope once:

`~/.config/nix/nix.conf`

```
netrc-file = /home/YOU/.config/nix/netrc
```

`~/.config/nix/netrc`

```
machine github.com login YOU password ghp_yourtoken
```

The token authenticates the request to `github.com`; the redirect to the signed
asset URL carries its own signature and needs no token. Keep the netrc at mode `600`.

## Zero-token alternative

To avoid the token entirely, host the AppImage where it can be fetched without
auth — for example your own deploy server, which CI already reaches over the VPN.
CI would upload the AppImage there and `nix/nightly-info.nix` would point `url` at
that host instead of the GitHub release. Ask if you want this wired.
