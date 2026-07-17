{ stdenv
, lib
, nodejs_22
, pnpm_11
, fetchPnpmDeps
, pnpmConfigHook
, src
,
}:

# Stage 1 of the desktop package: the built Vue SPA. The Rust build embeds this
# via `generate_context!` at compile time, so it must exist before stage 2.
# The `@atlas/web` build script reads the committed `apps/web/openapi.json`
# (kept current by openapi_drift.rs), so no server-side schema generation runs.
stdenv.mkDerivation (finalAttrs: {
  pname = "atlas-web-dist";
  version = "0.0.0";
  inherit src;

  nativeBuildInputs = [
    nodejs_22
    pnpm_11
    (pnpmConfigHook.override { pnpm = pnpm_11; })
  ];

  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs) pname version src;
    pnpmWorkspaces = [ "@atlas/web" ];
    fetcherVersion = 4;
    hash = "sha256-zsN4neeaRVVWSBICeS/6q0r+PkOltWXozGElWoEsrbo=";
  };

  buildPhase = ''
    runHook preBuild
    pnpm --filter=@atlas/web build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    cp -r apps/web/dist "$out"
    runHook postInstall
  '';
})
