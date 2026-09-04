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

Each variable's **Owner** names the typed config struct that reads it after
`v2-e11-s1-config-composition` (`crates/atlas_server/src/config/`):
`platform` (`PlatformConfig`), `custos` (`CustosConfig`), `acta`
(`ActaConfig`), `storage` (`StorageConfig`), `search.postgres_fts`
(`SearchLexicalConfig`), or `search.pgvector_embeddings`
(`SearchSemanticConfig`).

### Server boot and HTTP

| Variable | Owner | Required? | Default / behavior |
|---|---|---:|---|
| `DATABASE_URL` | platform | Yes | no default at runtime; required for server startup |
| `ATLAS_DB_MAX_CONNECTIONS` | platform | No | `20` |
| `ATLAS_DB_MIN_CONNECTIONS` | platform | No | `1` |
| `ATLAS_DB_ACQUIRE_TIMEOUT_SECS` | platform | No | `10` seconds |
| `ATLAS_SHUTDOWN_TIMEOUT_SECS` | platform | No | `20` seconds; upper bound on the graceful-drain window after a shutdown signal |
| `ATLAS_ROOT_PASSWORD` | custos | First boot only | used by bootstrap when no users exist yet |
| `ATLAS_PORT` | platform | No | `8080`; server binds `0.0.0.0:<port>` |
| `RUST_LOG` | — | No | `info,atlas_server=debug,tower_http=info` |
| `ATLAS_SERVER_URL` | platform | No | public base URL reported by `/api/meta` and used in activation links |
| `ATLAS_BUILD` | platform | No | build identifier surfaced by `/api/meta` |

### Sessions, cookies, and document internals

| Variable | Owner | Default | Notes |
|---|---|---|---|
| `ATLAS_SESSION_TTL_HOURS` | platform | `168` | session sliding TTL |
| `ATLAS_SESSION_MAX_TTL_HOURS` | platform | `720` | max session age |
| `ATLAS_IDEMPOTENCY_RETENTION_HOURS` | platform | `24` | response-retention TTL for the `Idempotency-Key` store |
| `ATLAS_COOKIE_SECURE` | platform | `true` | set `false` or `0` for local HTTP dev |
| `ATLAS_ANCHOR_INTERVAL` | acta | `50` | must be `>= 2` |

### Attachments

| Variable | Owner | Default | Notes |
|---|---|---|---|
| `ATLAS_ATTACHMENT_BACKEND` | storage | `disk` | `disk` or `s3` |
| `ATLAS_ATTACHMENT_ROOT` | storage | `./data/attachments` | disk backend root |
| `ATLAS_ACTA_MAX_ATTACHMENT_BYTES` | acta | `20971520` (20 MiB) | upper bound on an uploaded attachment's size, in bytes |
| `ATLAS_UPLOAD_ALLOWED_EXTENSIONS` | acta | — | comma-separated allow-list of upload extensions (e.g. `png,jpg,pdf,txt`); when set, an upload's declared extension must be in the list and pass the content check. Empty/unset keeps the default (all safe types allowed; executables always blocked). |
| `ATLAS_S3_BUCKET` | storage | — | required when backend is `s3` |
| `ATLAS_S3_ENDPOINT` | storage | — | required when backend is `s3` |
| `ATLAS_S3_ACCESS_KEY_ID` | storage | — | required when backend is `s3` |
| `ATLAS_S3_SECRET_ACCESS_KEY` | storage | — | required when backend is `s3` |
| `ATLAS_S3_REGION` | storage | `auto` | suitable for R2-style endpoints |

### Semantic search embeddings

Semantic search is an optional API and MCP surface. Lexical `/search` stays enabled
and unchanged when embeddings are disabled.

| Variable | Owner | Default | Notes |
|---|---|---|---|
| `ATLAS_EMBEDDINGS_ENABLED` | search.pgvector_embeddings | `false` | Enables `/api/v2/acta/workspaces/{ws}/semantic-search` and the `semantic`/`hybrid` search modes. Disabled returns `503` on semantic search only; `mode=hybrid` degrades to lexical. |
| `ATLAS_EMBEDDINGS_PROVIDER` | search.pgvector_embeddings | — | Required when `ATLAS_EMBEDDINGS_ENABLED=true`. `openai_compatible` for an OpenAI-compatible embeddings API, or `deterministic`/`test` for offline development — the latter hashes text into a valid-looking vector that encodes no meaning, so it must be asked for by name and is never inherited. |
| `ATLAS_EMBEDDINGS_MODEL` | search.pgvector_embeddings | `atlas-test-embedding` | Stored with each embedding row; changing it requires re-indexing content for the new model. |
| `ATLAS_EMBEDDINGS_DIMENSIONS` | search.pgvector_embeddings | `1536` | Must match the provider output. The `search_embeddings.embedding` column is declared `vector(1536)`, so any other value fails startup with an explicit error instead of failing on the first insert. |
| `ATLAS_EMBEDDINGS_API_KEY` | search.pgvector_embeddings | — | Required only when `ATLAS_EMBEDDINGS_ENABLED=true` and provider is `openai_compatible`. |
| `ATLAS_EMBEDDINGS_BASE_URL` | search.pgvector_embeddings | `https://api.openai.com/v1` | Base URL for OpenAI-compatible providers. |
| `ATLAS_EMBEDDINGS_BATCH_SIZE` | search.pgvector_embeddings | `64` | Maximum inputs per provider request; larger sets are split across successive requests. |
| `ATLAS_EMBEDDINGS_TIMEOUT_MS` | search.pgvector_embeddings | `30000` | Provider request timeout. |
| `ATLAS_EMBEDDINGS_RETRY_ATTEMPTS` | search.pgvector_embeddings | `2` | Retries per batch, with exponential backoff, for transport failures, `429`, and `5xx`. Other failures (a rejected key, a malformed response) are not retried. |

Backfill/indexing behavior:

- Missing or stale embeddings are skipped by semantic search; they do not break lexical search.
- Re-indexing hashes normalized chunk text and skips unchanged chunks for the active model/dimensions.
- Task indexing includes readable ID, title, description, labels, visible comments, attachment file names, checklist items, and direct visible subtask text.
- Document indexing includes title, content, visible comments, and attachment file names.
- Deferred scope: HNSW tuning is not part of this slice.

Workspace backfill (`/api/v2/acta/workspaces/{ws}/semantic-search/reindex`, workspace admin or owner):

- `GET` returns what a reindex would embed — documents, tasks, characters, estimated chunks and tokens — plus how far the workspace already is (`indexed_resources`, `queued_resources`). It queues nothing, so it is the read to take before paying a provider to embed a corpus. The chunk and token figures are approximations from stored character counts (~4 characters per token), not a provider quote.
- `POST` queues every live document and task, then returns the same plan alongside how many resources were newly queued. The running indexer worker drains the queue; content whose hash is unchanged is not re-embedded, so re-running the backfill costs queue rows rather than embeddings, and an interrupted run resumes by simply running it again.
- `POST` returns `503` when embeddings are disabled or the pgvector schema is missing: queueing work that nothing drains would only grow a backlog an operator would read as progress.
- Re-run it after changing `ATLAS_EMBEDDINGS_MODEL` or `ATLAS_EMBEDDINGS_DIMENSIONS`; embeddings are keyed by model and dimensions, so the old rows stay but no longer answer queries.

### Hybrid search

`GET /api/v2/acta/workspaces/{ws}/search` takes `mode`: `lexical` (default), `semantic`, or `hybrid`.
Hybrid fuses the two arms with Reciprocal Rank Fusion, combining them by rank rather than by
score — a `ts_rank_cd` value and a cosine distance are not comparable quantities. It is the mode
that answers both "how do we authenticate" against a document that only says "OAuth flow" and a
literal `ATL-1247`.

| Variable | Owner | Default | Notes |
|---|---|---|---|
| `ATLAS_SEARCH_RRF_K` | search.pgvector_embeddings | `60` | RRF damping constant (must be >= 1). Smaller lets one arm's top hit dominate; larger weighs the arms more evenly. 60 is the published default — worth measuring against a real corpus. Lives on the semantic module's config because RRF only runs when semantic search is present. |
| `ATLAS_SEARCH_HYBRID_POOL` | search.pgvector_embeddings | `50` | Candidates each arm contributes before fusion. |

- Both non-lexical modes rank by fused relevance, so `sort=updated` is rejected with `422`.
- Fusion happens over the two candidate pools only, so a fused result set pages no deeper than
  the pool: this mode is for finding the answer in the first few hits, not for scrolling a corpus.
- `mode=hybrid` falls back to lexical results when embeddings are unavailable — those results are
  correct, only less complete. `mode=semantic` returns `503` instead, rather than silently
  answering from a different retriever than the one asked for.

### Rate limiting

The authenticated API surface is rate-limited per principal (the resolved user or
API key), not per IP, because the volume risk comes from programmatic clients (the
CLI and MCP server), which are always authenticated. IP-based limiting still
guards the unauthenticated login and activation routes.

| Variable | Owner | Default | Notes |
|---|---|---|---|
| `ATLAS_RATE_LIMIT_ENABLED` | platform | `true` | set `false`/`0` to disable the per-principal limiter |
| `ATLAS_RATE_LIMIT_PER_SECOND` | platform | `20` | steady-state requests per second per principal |
| `ATLAS_RATE_LIMIT_BURST` | platform | `40` | maximum instantaneous burst per principal |

The limiter is in-memory (GCRA via `governor`); it is per-process and not shared
across replicas. A rejected request returns `429 Too Many Requests` with a
`Retry-After` header. The `atlas_client` used by the CLI and MCP honors that
header and retries automatically with bounded backoff, so bulk operations
self-throttle instead of failing on the first rejection.

### Webhooks and integrations

| Variable | Owner | Required? | Default / notes |
|---|---|---:|---|
| `ATLAS_WEBHOOK_ENC_KEY` | acta | Yes | base64 value that must decode to exactly 32 bytes; `.env.example` suggests `openssl rand -base64 32` |
| `ATLAS_WEBHOOK_POLL_INTERVAL_MS` | acta | No | `1000` |
| `ATLAS_WEBHOOK_MAX_ATTEMPTS` | acta | No | `5` |
| `ATLAS_WEBHOOK_DELIVERY_TIMEOUT_MS` | acta | No | `10000` |
| `ATLAS_WEBHOOK_MAX_CONCURRENT` | acta | No | `16` |
| `ATLAS_WEBHOOK_BATCH_SIZE` | acta | No | `32` |
| `ATLAS_WEBHOOK_LEASE_SECS` | acta | No | `30` |
| `ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS` | acta | No | `false`; allows a webhook target URL to resolve to a private/loopback address, for local development only |

The server starts a background webhook dispatcher after building application state and shuts it down gracefully when the HTTP server exits.

### CLI and MCP client-side variables

| Variable | Default | Used by |
|---|---|---|
| `ATLAS_BASE_URL` | `http://localhost:8080` | CLI and MCP |
| `ATLAS_TOKEN` | none | CLI fallback token and required stdio token for MCP |
| `ATLAS_MCP_TRANSPORT` | `stdio` | MCP only |
| `ATLAS_MCP_BIND` | `127.0.0.1` | MCP HTTP mode |
| `ATLAS_MCP_PORT` | `3001` | MCP HTTP mode |

In HTTP mode the bind address and port also decide which callers the transport accepts: the `Host`
header must name the bind address or a loopback name, and a request carrying a browser `Origin`
must name one of the server's own `http://<host>:<port>` origins. Agents and the CLI send no
`Origin`, so only browsers are affected. Behind the bundled nginx the `/mcp` location rewrites
`Host` to `localhost`, so the host check does not currently constrain the public hostname; see
`docs/mcp.md` for what that does and does not cover.

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
