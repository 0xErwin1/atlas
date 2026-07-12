# Atlas

Atlas is an AI-first workspace platform — markdown **notes** (with wikilinks and backlinks) and kanban **tasks**, multi-workspace, with resource-sharing permissions and human-vs-agent attribution. One REST API serves all three consumers alike: the web UI, the MCP server (agents), and the CLI. This repository is the full monorepo: Rust backend services, a Vue 3 web frontend, and shared tooling.

Start with [docs/README.md](docs/README.md) for the documentation portal: product/features, web app, REST API, CLI, MCP, operations, limitations, and contributor entry points.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the crate map, request lifecycle, data model, permission model, and the web frontend overview.

## Quick start

```sh
direnv allow          # activate the Nix devshell (Rust 1.96, pnpm, just, podman)
just db-up            # start Postgres 17 via podman compose
just dev              # run atlas_server on :8080
```

In a separate terminal:

```sh
just dev-web          # run Vite dev server on :5173
```

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
│   └── web/            # Vue 3 + Vite + Pinia + Tailwind v4 frontend
├── packages/           # reserved for shared TS packages
├── .github/workflows/  # style, tests, web CI
├── justfile            # all local task recipes
├── compose.yaml        # Postgres 17 via podman
├── flake.nix           # Nix devshell
└── biome.json          # TypeScript/JSON formatter and linter
```

## Useful recipes

| Recipe | What it does |
|--------|-------------|
| `just check` | `cargo check --workspace` |
| `just test` | nextest + doctests |
| `just clippy` | clippy -D warnings |
| `just build-web` | vue-tsc + vite build |
| `just lint-web` | biome ci |
| `just verify` | full local gate (fmt-check + clippy + test + build + lint-web) |

## Documentation

- [Documentation portal](docs/README.md) — product, web, REST API, CLI, MCP, operations, limitations, and contributor map.
- [ARCHITECTURE.md](ARCHITECTURE.md) — crate map, request lifecycle, data model, permissions, and frontend architecture.
- [CODE_STYLE.md](CODE_STYLE.md) — coding and documentation conventions.
- [CONTRIBUTING.md](CONTRIBUTING.md) — contribution workflow.

See [AGENTS.md](AGENTS.md) for agent guidance and commit conventions, and [CODE_STYLE.md](CODE_STYLE.md) for coding conventions and lint policy.
