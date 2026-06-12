# Atlas

Atlas is an AI-first workspace platform. This repository contains the full monorepo: Rust backend services, a Vue 3 web frontend, and shared tooling.

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
│   ├── atlas_domain/   # pure domain types, no I/O deps
│   ├── atlas_server/   # axum HTTP server
│   ├── atlas_client/   # HTTP client wrapping atlas_domain
│   ├── atlas_cli/      # clap CLI using atlas_client
│   ├── atlas_mcp/      # Model Context Protocol server (rmcp)
│   └── migration/      # sea-orm-migration stub (empty for now)
├── apps/
│   └── web/            # Vue 3 + Vite + Tailwind v4 frontend
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

See [CONTRIBUTING.md](CONTRIBUTING.md) for commit conventions and [CODE_STYLE.md](CODE_STYLE.md) for lint policy.
