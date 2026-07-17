{ lib
, rustPlatform
, pkg-config
, mold
, wrapGAppsHook3
, copyDesktopItems
, makeDesktopItem
, webkitgtk_4_1
, glib-networking
, dbus
, openssl
, gnome-keyring
, webDist
, src
,
}:

let
  desktopItem = makeDesktopItem {
    name = "atlas-desktop";
    desktopName = "Atlas Desktop";
    exec = "atlas_desktop";
    icon = "atlas-desktop";
    categories = [
      "Office"
      "Utility"
    ];
    startupWMClass = "me.feuer.atlas.desktop";
  };
in
rustPlatform.buildRustPackage {
  pname = "atlas-desktop";
  version = "0.0.0";
  inherit src;

  cargoLock.lockFile = ../Cargo.lock;

  # Build only the shipping binary. The desktop-gate binaries carry
  # `required-features`, so they are excluded here, and gate.rs's
  # `compile_error!` guarantees the feature can never reach a release build.
  cargoBuildFlags = [
    "-p"
    "atlas_desktop"
    "--bin"
    "atlas_desktop"
  ];

  # The crate tests need a database and a webdriver; neither exists in the
  # build sandbox.
  doCheck = false;

  # The workspace's .cargo/config.toml links with mold (-fuse-ld=mold).
  nativeBuildInputs = [
    pkg-config
    mold
    wrapGAppsHook3
    copyDesktopItems
  ];

  buildInputs = [
    webkitgtk_4_1
    glib-networking
    dbus
    openssl
    gnome-keyring
  ];

  desktopItems = [ desktopItem ];

  # `generate_context!` embeds the frontend at compile time, so the built SPA
  # must sit where tauri.conf.json's `frontendDist` (../../web/dist) points
  # before cargo runs.
  postPatch = ''
    mkdir -p apps/web/dist
    cp -r ${webDist}/. apps/web/dist/
  '';

  postInstall = ''
    install -Dm644 apps/web/public/favicon.svg \
      "$out/share/icons/hicolor/scalable/apps/atlas-desktop.svg"
  '';
}
