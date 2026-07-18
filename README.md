# Atlas

Atlas is an AI-first workspace platform — markdown **notes** (with wikilinks and backlinks) and kanban **tasks**, multi-workspace, with resource-sharing permissions and human-vs-agent attribution. One REST API serves all three consumers alike: the web UI, the MCP server (agents), and the CLI. This repository is the full monorepo: Rust backend services, a Vue 3 web frontend, and shared tooling.

Start with [docs/README.md](docs/README.md) for the documentation portal: product/features, web app, REST API, CLI, MCP, operations, limitations, and contributor entry points.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the crate map, request lifecycle, data model, permission model, and the web frontend overview.

## Quick start

```sh
direnv allow          # activate the dev shell (Rust 1.96, pnpm, devenv, podman)
tests                  # run the workspace test suite; Postgres starts automatically
```

Atlas is not run locally — it is deployed as containers with its runtime configuration injected at deploy time. The dev shell is for building, linting, and testing; see [Useful commands](#useful-commands) below.

## Repo map

```
atlas/
├── crates/
│   ├── atlas_domain/   # pure domain types + ports, no I/O deps
│   ├── atlas_api/      # shared DTOs + OpenAPI schemas (the wire contract)
│   ├── atlas_server/   # axum HTTP server + SeaORM adapters
│   ├── atlas_client/   # typed HTTP client over atlas_api/atlas_domain
│   ├── atlas_cli/      # clap CLI using atlas_client
│   ├── atlas_mcp/      # Model Context Protocol server (rmcp)
│   └── migration/      # sea-orm-migration tool (run: cargo run -p migration -- up)
├── apps/
│   ├── web/            # Vue 3 + Vite + Pinia + Tailwind v4 frontend
│   └── desktop/        # Tauri desktop app wrapping the web UI
├── packages/           # reserved for shared TS packages
├── nix/                # desktop package + nightly install (see nix/README-nightly.md)
├── deploy/             # container images and VPN deploy script
├── .github/workflows/  # style, tests, web, desktop, and nightly CI
├── flake.nix           # devenv dev shell, command surface, and Nix packages
└── biome.json          # TypeScript/JSON formatter and linter
```

## Useful commands

| Command | What it does |
|---------|-------------|
| `check` | `cargo check --workspace` |
| `tests` | starts an ephemeral Postgres container, then nextest + doctests |
| `clippy` | clippy -D warnings |
| `build-web` | vue-tsc + vite build |
| `lint-web` | biome ci |
| `verify` | full local gate (fmt-check + clippy + tests + build + lint-web + build-web) |

## Desktop app

Atlas ships a Tauri desktop client (`apps/desktop`) that wraps the web UI. CI publishes a prebuilt AppImage to a rolling `nightly` release on every push to `main`, so you can install it without compiling:

```sh
nix run github:0xErwin1/atlas/nightly#atlas-desktop-nightly
```

See [nix/README-nightly.md](nix/README-nightly.md) for home-manager installation. To build from source instead, run `nix build .#atlas-desktop`.

## Documentation

- [Documentation portal](docs/README.md) — product, web, REST API, CLI, MCP, operations, limitations, and contributor map.
- [ARCHITECTURE.md](ARCHITECTURE.md) — crate map, request lifecycle, data model, permissions, and frontend architecture.
- [CODE_STYLE.md](CODE_STYLE.md) — coding and documentation conventions.
- [CONTRIBUTING.md](CONTRIBUTING.md) — contribution workflow.

See [AGENTS.md](AGENTS.md) for agent guidance and commit conventions, and [CODE_STYLE.md](CODE_STYLE.md) for coding conventions and lint policy.

## License

Licensed under the [Apache License 2.0](LICENSE).
