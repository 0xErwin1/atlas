# Operations and setup

This page covers the practical setup and runtime knobs visible from `justfile`, `.env.example`, and server startup code.

## Local development flow

Recommended path from the root docs:

```sh
direnv allow
just db-up
just dev
```

In another terminal:

```sh
just dev-web
```

The repo assumes:

- the Nix dev shell for pinned Rust/pnpm tooling
- Podman for local Postgres
- PostgreSQL 17

## Useful `just` targets

| Command | Purpose |
|---|---|
| `just db-up` | start local Postgres |
| `just db-down` | stop local Postgres |
| `just db-wait` | wait for Postgres readiness |
| `just migrate` | apply migrations |
| `just db-reset` | run migration fresh |
| `just db-clean-tests` | drop leftover test DBs |
| `just seed-dev` | run the dev seeder binary |
| `just dev` | run `atlas_server` |
| `just gen-types` | dump OpenAPI and regenerate web types |
| `just dev-web` | regenerate types, then run the Vite app |
| `just build-web` | regenerate types, then build the web app |
| `just check` / `just test` / `just clippy` / `just build` | core Rust verification |
| `just lint-web` / `just fmt-web` | frontend formatting/linting |
| `just verify` | full local gate |
| `just up` | process-compose full stack: Postgres + seed + API + web |

`just up` exports a default `ATLAS_ROOT_PASSWORD=rootdev` if none is set and documents the default login as `root` / `$ATLAS_ROOT_PASSWORD`.

## Required and common environment variables

### Server boot and HTTP

| Variable | Required? | Default / behavior |
|---|---:|---|
| `DATABASE_URL` | Yes | no default at runtime; required for server startup |
| `ATLAS_ROOT_PASSWORD` | First boot only | used by bootstrap when no users exist yet |
| `ATLAS_PORT` | No | `8080`; server binds `0.0.0.0:<port>` |
| `RUST_LOG` | No | `info,atlas_server=debug,tower_http=info` |
| `ATLAS_SERVER_URL` | No | public base URL reported by `/v1/meta` and used in links |
| `ATLAS_BUILD` | No | build identifier surfaced by `/v1/meta` |

### Sessions, cookies, and document internals

| Variable | Default | Notes |
|---|---|---|
| `ATLAS_SESSION_TTL_HOURS` | `168` | session sliding TTL |
| `ATLAS_SESSION_MAX_TTL_HOURS` | `720` | max session age |
| `ATLAS_COOKIE_SECURE` | `true` | set `false` or `0` for local HTTP dev |
| `ATLAS_ANCHOR_INTERVAL` | `50` | must be `>= 2` |

### Attachments

| Variable | Default | Notes |
|---|---|---|
| `ATLAS_ATTACHMENT_BACKEND` | `disk` | `disk` or `s3` |
| `ATLAS_ATTACHMENT_ROOT` | `./data/attachments` | disk backend root |
| `ATLAS_UPLOAD_ALLOWED_EXTENSIONS` | — | comma-separated allow-list of upload extensions (e.g. `png,jpg,pdf,txt`); when set, an upload's declared extension must be in the list and pass the content check. Empty/unset keeps the default (all safe types allowed; executables always blocked). |
| `ATLAS_S3_BUCKET` | — | required when backend is `s3` |
| `ATLAS_S3_ENDPOINT` | — | required when backend is `s3` |
| `ATLAS_S3_ACCESS_KEY_ID` | — | required when backend is `s3` |
| `ATLAS_S3_SECRET_ACCESS_KEY` | — | required when backend is `s3` |
| `ATLAS_S3_REGION` | `auto` | suitable for R2-style endpoints |

The shared default attachment size cap in `AppState` is `20 MiB`.

### Semantic search embeddings

Semantic search is an optional API and MCP surface. Lexical `/search` stays enabled
and unchanged when embeddings are disabled.

| Variable | Default | Notes |
|---|---|---|
| `ATLAS_EMBEDDINGS_ENABLED` | `false` | Enables `/api/workspaces/{ws}/semantic-search` and the MCP `semantic_search` tool. Disabled returns `503` on semantic search only. |
| `ATLAS_EMBEDDINGS_PROVIDER` | `deterministic` | `deterministic`/`test` for offline development, or `openai_compatible` for an OpenAI-compatible embeddings API. |
| `ATLAS_EMBEDDINGS_MODEL` | `atlas-test-embedding` | Stored with each embedding row; changing it requires re-indexing content for the new model. |
| `ATLAS_EMBEDDINGS_DIMENSIONS` | `1536` | Must match the provider output and the pgvector column/index size. |
| `ATLAS_EMBEDDINGS_API_KEY` | — | Required only when `ATLAS_EMBEDDINGS_ENABLED=true` and provider is `openai_compatible`. |
| `ATLAS_EMBEDDINGS_BASE_URL` | `https://api.openai.com/v1` | Base URL for OpenAI-compatible providers. |
| `ATLAS_EMBEDDINGS_BATCH_SIZE` | `64` | Batch size used by embedding writes/backfills. |
| `ATLAS_EMBEDDINGS_TIMEOUT_MS` | `30000` | Provider request timeout. |
| `ATLAS_EMBEDDINGS_RETRY_ATTEMPTS` | `2` | Provider retry attempts. |

Backfill/indexing behavior:

- Missing or stale embeddings are skipped by semantic search; they do not break lexical search.
- Re-indexing hashes normalized chunk text and skips unchanged chunks for the active model/dimensions.
- Task indexing includes readable ID, title, description, labels, visible comments, attachment file names, checklist items, and direct visible subtask text.
- Document indexing includes title, content, visible comments, and attachment file names.
- Deferred scope: durable background queue automation and HNSW tuning are not part of this slice; run explicit backfill/re-index flows when changing model or dimensions.

### Rate limiting

The authenticated API surface is rate-limited per principal (the resolved user or
API key), not per IP, because the volume risk comes from programmatic clients (the
CLI and MCP server), which are always authenticated. IP-based limiting still
guards the unauthenticated login and activation routes.

| Variable | Default | Notes |
|---|---|---|
| `ATLAS_RATE_LIMIT_ENABLED` | `true` | set `false`/`0` to disable the per-principal limiter |
| `ATLAS_RATE_LIMIT_PER_SECOND` | `20` | steady-state requests per second per principal |
| `ATLAS_RATE_LIMIT_BURST` | `40` | maximum instantaneous burst per principal |

The limiter is in-memory (GCRA via `governor`); it is per-process and not shared
across replicas. A rejected request returns `429 Too Many Requests` with a
`Retry-After` header. The `atlas_client` used by the CLI and MCP honors that
header and retries automatically with bounded backoff, so bulk operations
self-throttle instead of failing on the first rejection.

### Webhooks and integrations

| Variable | Required? | Default / notes |
|---|---:|---|
| `ATLAS_WEBHOOK_ENC_KEY` | Yes | base64 value that must decode to exactly 32 bytes; `.env.example` suggests `openssl rand -base64 32` |
| `ATLAS_WEBHOOK_POLL_INTERVAL_MS` | No | `1000` |
| `ATLAS_WEBHOOK_MAX_ATTEMPTS` | No | `5` |
| `ATLAS_WEBHOOK_DELIVERY_TIMEOUT_MS` | No | `10000` |
| `ATLAS_WEBHOOK_MAX_CONCURRENT` | No | `16` |
| `ATLAS_WEBHOOK_BATCH_SIZE` | No | `32` |
| `ATLAS_WEBHOOK_LEASE_SECS` | No | `30` |

The server starts a background webhook dispatcher after building application state and shuts it down gracefully when the HTTP server exits.

### CLI and MCP client-side variables

| Variable | Default | Used by |
|---|---|---|
| `ATLAS_BASE_URL` | `http://localhost:8080` | CLI and MCP |
| `ATLAS_TOKEN` | none | CLI fallback token and required stdio token for MCP |
| `ATLAS_MCP_TRANSPORT` | `stdio` | MCP only |
| `ATLAS_MCP_BIND` | `127.0.0.1` | MCP HTTP mode |
| `ATLAS_MCP_PORT` | `3001` | MCP HTTP mode |

## What server startup does

Backed by `crates/atlas_server/src/main.rs` and `state.rs`, startup:

1. loads env config
2. connects to Postgres
3. applies migrations
4. runs bootstrap for the root user
5. builds application state
6. initializes the configured attachment backend
7. starts the webhook dispatcher background task
8. serves HTTP

## OpenAPI and web type generation

The web app uses generated types. The supported generation path is:

```sh
just gen-types
```

That runs `cargo run -p atlas_server --bin dump_openapi > apps/web/openapi.json` and then `openapi-typescript` into `apps/web/src/api/types.d.ts`.

Do not hand-edit the generated type file.

## Safe docs-only validation

For docs-only changes, the requested lightweight validation is:

```sh
git diff --check
```

Optionally add a lightweight markdown link check if you touched many links.
