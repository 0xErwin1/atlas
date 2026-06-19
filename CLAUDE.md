# CLAUDE.md — Atlas

Guidance for AI agents working in this repository. Atlas is an AI-first knowledge + project-management platform (markdown documents + kanban tasks) exposed through one REST API consumed by a web UI, an MCP server, and a CLI. For the canonical structure (crate map, request lifecycle, data model, permission model), see `ARCHITECTURE.md`.

## Environment (read first)

This is a NixOS host with **no system Rust toolchain**. Every cargo/just/rust command MUST run inside the dev shell:

```bash
nix develop --command <cmd>      # one-off
nix develop                      # interactive shell (direnv loads it via .envrc)
```

The dev shell provides the pinned Rust 1.96 toolchain, `pnpm`, `just`, `podman`, `sea-orm-cli`, `cargo-nextest`, `mold`, and `actionlint`. Containers use **podman, not docker**.

## Commands

Run everything through `just` (the canonical command surface) inside the dev shell:

| Task | Command |
|------|---------|
| Type-check | `just check` |
| Lint (fails the build) | `just clippy` — `cargo clippy --workspace --all-targets -- -D warnings` |
| Format / check format | `just fmt` / `just fmt-check` |
| Tests | `just test` — starts Postgres wait, runs `cargo nextest run --workspace` + doctests |
| Build | `just build` |
| Full gate (matches CI) | `just verify` — fmt-check + clippy + test + build + web lint |
| Start dev Postgres | `just db-up` (podman compose, `postgres:17`) |
| Reset schema | `just db-reset` / apply migrations `just migrate` |
| Seed dev data | `just seed-dev` |
| Run the server | `just dev` |
| Web dev / build / lint | `just dev-web` / `just build-web` / `just lint-web` (Biome) |

Integration tests need Postgres running (`just db-up`); the harness creates and drops one database per test.

## Workspace layout

Seven crates. The dependency direction is strict and **compiler-enforced** — `atlas_domain` is pure and never imports HTTP/SQL.

| Crate | Role | May depend on |
|-------|------|---------------|
| `atlas_domain` | Pure types, value objects, errors, **repository ports** (traits taking `WorkspaceCtx`), pure permission/diff/position logic | serde, thiserror, uuid, chrono only — **no axum, no sea-orm, no tokio** |
| `atlas_api` | Shared DTOs + OpenAPI schemas (the wire contract) | atlas_domain |
| `atlas_client` | Typed HTTP client speaking `atlas_api`/`atlas_domain` types | atlas_api, atlas_domain, reqwest |
| `atlas_server` | axum binary; SeaORM **adapters** implementing the ports; auth, permissions, routing | everything |
| `atlas_cli` | clap CLI over `atlas_client` | atlas_client |
| `atlas_mcp` | MCP server (rmcp) over `atlas_client` | atlas_client |
| `migration` | sea-orm-migration tool crate (run via `cargo run -p migration -- <up\|fresh>`) | — |

Persistence pattern: SeaORM entities live in `atlas_server/src/persistence/entities/`, adapters in `.../repos/`, and map to/from domain types — SeaORM types never leak into `atlas_domain`.

## Web frontend (`apps/web`)

A Vue 3 SPA (Vite, Pinia per-domain stores, vue-router, Tailwind v4) — one of the three API consumers. It only speaks the REST contract; it never touches the DB.

- **Generated API client.** A typed `openapi-fetch` client over `src/api/types.d.ts`, generated from the served OpenAPI by `just gen-types`. After ANY backend contract change, regenerate it; never hand-edit `types.d.ts`. A thin wrapper adds the session cookie + CSRF header and surfaces the RFC 9457 `hint`.
- **Forms.** Validate with **zod** through the shared `FormField` (`src/components/ui/FormField.vue`) + `validateForm` (`src/lib/validation.ts`); show the API `hint`, never a stack. No native browser validation bubbles.
- **Editor.** Shared CodeMirror 6 "live preview" `MarkdownEditor` — markdown is the source of truth. Wikilinks are id-bound `[[<uuid>|Title]]` (rename-stable; legacy `[[Title]]` resolves by slug) and render the target's current title.
- **Tooling.** Biome (not eslint/prettier), Vitest, vue-tsc — all in `just verify`. Match existing component/store patterns; same English-only, comment-sparing conventions as the Rust side.

## Conventions

- **Strict TDD.** Write the failing test first, see it red, then implement to green. Tests run with `cargo nextest`; doctests run separately.
- **No panics.** Lints deny `unwrap_used`, `expect_used`, `panic`, `unwrap_in_result`, `dbg_macro`; `unsafe_code` is forbidden. Propagate with `?`; return `Result`. `todo!`/`unimplemented!` warn — never ship them.
- **Commits.** Conventional Commits (`feat:`, `fix:`, `chore:`, `refactor:`, `docs:`, `test:`, `ci:`) directly to `main`, one atomic work-unit per commit (code + its test together). Author identity `Ignacio Perez <ignacio@feuer.me>`. No co-author trailers.
- **English** for all code, comments, docs, and commit messages.
- **Comments.** Default to none. Add one only when the *why* is non-obvious (a constraint, an invariant, a workaround). Never restate what the code does; never reference a task/PR/ticket. Function-level doc comments for intent/invariants are welcome.
- **Secrets.** `ATLAS_ROOT_PASSWORD` and any session/API-key value must never be logged or printed. Hash passwords with argon2 inside `spawn_blocking`.

## Architecture notes

- **Multi-tenant by construction:** every domain repository method takes a `WorkspaceCtx`; a query that forgets `workspace_id` cannot be written through the port. Cross-tenant isolation is covered by integration tests.
- **Auth:** humans authenticate with username+password sessions (argon2; session token stored as SHA-256 hash) delivered as both an HttpOnly cookie (browser) and a bearer token (CLI/MCP). API keys (`atlas_` prefix, hash-only) are for agents exclusively. First boot seeds a root user from `ATLAS_ROOT_PASSWORD` (fail-fast if unset).
- **Permissions** are resource-sharing style (not IAM): grants `(principal, resource, role)` with `viewer`/`editor`/`admin` roles inheriting down `workspace > project > folder > document|board`, visibility as sugar, default deny, agents capped at `editor` and never managing grants. Every protected route declares its target resource + minimum role via the `Authorized<…>` extractor. Route coverage relies on `ROUTE_REGISTRY` (`src/routes/registry.rs`): the registry→router direction is audited at runtime by `all_registry_entries_are_wired_in_router`; the reverse (a route added to `lib.rs` without a registry entry) is not automatically caught — axum 0.8 exposes no Router introspection. Developers must update `ROUTE_REGISTRY` when adding routes.
- **Sub-tasks** are full tasks linked by `tasks.parent_task_id`, not lightweight rows: they carry every task field and their own `readable_id`, so all existing task endpoints work on them and they are wikilink-referenceable. The invariant is that board/column listings filter `parent_task_id IS NULL`, so a sub-task never shows on the kanban; `…/tasks/{id}/promote` clears the parent to surface it. (The older `task_checklist_items` table and its endpoints remain but are no longer used by the web UI.)
- **Errors** follow RFC 9457 (`application/problem+json`) extended with `request_id` and an actionable `hint`. **Pagination** is opaque base64url cursors over UUIDv7 in a `Page<T>` envelope.
- **OpenAPI** is generated from the code (utoipa) and served with a zero-drift test; `atlas_client` stays aligned with it.

## Database

Postgres 17. IDs are app-generated UUIDv7 (time-ordered). Document content is `TEXT` (TOAST-backed); attachments live in object storage (disk → Cloudflare R2), never as DB blobs. Document revisions are line diffs with periodic full-snapshot anchors.
