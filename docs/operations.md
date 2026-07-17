# Operations and setup

This page covers the practical setup and runtime knobs visible from `flake.nix`, `.env.example`, and server startup code.

## Local development flow

Atlas is not run locally — it is deployed as containers, with its runtime configuration injected at deploy time. The dev shell is for building, linting, and testing:

```sh
direnv allow
tests
```

Postgres for tests is managed automatically: `tests` starts an ephemeral `pgvector/pgvector:pg17` container per run via `atlas_test_harness` and tears it down when the run finishes. There is no manual DB lifecycle command.

Prerequisites for the test container harness:

- rootless Podman with `podman.socket` enabled: `systemctl --user enable --now podman.socket`
- subuid/subgid ranges configured for your user (`/etc/subuid`, `/etc/subgid`)

The dev shell auto-exports `DOCKER_HOST` for the standard rootless Podman socket path when one isn't already set.

## Useful commands

| Command | Purpose |
|---|---|
| `check` | `cargo check --workspace` |
| `tests` | ephemeral Postgres container, then nextest + doctests |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `build` | `cargo build --workspace` |
| `gen-types` | dump OpenAPI and regenerate web types |
| `build-web` | regenerate types, then build the web app |
| `lint-web` | Biome CI check |
| `format` | `cargo fmt --all` + Biome format, repo-wide |
| `fmt-check` | `cargo fmt --all -- --check` |
| `verify` | full local gate: fmt-check, clippy, tests, build, lint-web, build-web |

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
gen-types
```

That runs `cargo run -p atlas_server --bin dump_openapi > apps/web/openapi.json` and then `openapi-typescript` into `apps/web/src/api/types.d.ts`.

Do not hand-edit the generated type file.

## Safe docs-only validation

For docs-only changes, the requested lightweight validation is:

```sh
git diff --check
```

Optionally add a lightweight markdown link check if you touched many links.
