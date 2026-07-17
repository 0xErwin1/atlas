# AGENTS.md — Atlas

Entry point and single source of truth for AI coding agents (Claude Code, Cursor, and any AGENTS.md-aware tool). Atlas is an AI-first knowledge + project-management platform (markdown documents + kanban tasks) exposed through one REST API consumed by a web UI, an MCP server, and a CLI. For the canonical structure (crate map, request lifecycle, data model, permission model), see `ARCHITECTURE.md`. For coding conventions and verification expectations, see `CODE_STYLE.md`.

`CLAUDE.md` is a thin pointer to this file; keep repository-specific agent behavior here, not duplicated there.

## Working principles

- **Truth & grounding.** Do not invent APIs, flags, types, library behavior, or codebase details. Prefer reading the existing repository over assuming how things work. If something is unclear or missing from the context, say so explicitly instead of guessing. Distinguish facts from inferences and hypotheses.
- **Finished features, not mockups.** Unless the user explicitly says a change is a test, MVP, or mockup, treat it as a final feature. Half-finished features are not acceptable. If a requirement cannot be met as stated, say so instead of shipping a partial version.
- **Scope & minimalism.** Limit edits to the files and regions the requested task needs. Make the smallest change that solves the problem. Do not refactor or "clean up" unrelated areas, and do not rename or reformat for churn, unless explicitly asked.
- **Function size.** Treat a function growing beyond ~100 lines as a design smell. Prefer extracting well-named helpers that preserve behavior exactly; keep refactors local (no public-API or cross-module moves unless requested). If a refactor is risky, explain the tradeoffs instead of proceeding blindly.
- **Critical feedback.** If something is incorrect, misleading, risky, or poorly designed, say so and explain why. Do not reinforce flawed logic to match intent; call out tradeoffs instead of defaulting to the safest-sounding answer.

## Environment (read first)

This is a NixOS host with **no system Rust toolchain**. `direnv` loads the dev shell automatically on `cd` (via `.envrc` → `flake.nix`); every cargo/rust command MUST run inside it. `nix develop` still works as a manual entrypoint if direnv is unavailable.

The dev shell provides the pinned Rust 1.96 toolchain, `pnpm`, `podman`, `cargo-nextest`, `mold`, `cargo-tauri`, and `actionlint`. Containers use **podman, not docker**.

## Commands

Run everything as a bare command (a devenv `script`) inside the dev shell — there is no `just` prefix:

| Task | Command |
|------|---------|
| Type-check | `check` |
| Lint (fails the build) | `clippy` — `cargo clippy --workspace --all-targets -- -D warnings` |
| Format / check format | `format` / `fmt-check` |
| Tests | `tests` — spins up an ephemeral Postgres container, then runs `cargo nextest run --workspace` + doctests |
| Build | `build` |
| Full gate (matches CI) | `verify` — backend and frontend compile, lint, format checks, and tests |
| Web build / lint | `build-web` / `lint-web` (Biome) |

Atlas is not run locally: it is deployed as containers, with its runtime configuration injected at deploy time. `tests` manages its own Postgres via `atlas_test_harness` (an ephemeral `pgvector/pgvector:pg17` container per run, started and torn down automatically); there is no manual DB lifecycle command to run first.

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

- **Generated API client.** A typed `openapi-fetch` client over `src/api/types.d.ts`, generated from the served OpenAPI by `gen-types`. After ANY backend contract change, regenerate it; never hand-edit `types.d.ts`. A thin wrapper adds the session cookie + CSRF header and surfaces the RFC 9457 `hint`.
- **Forms.** Validate with **zod** through the shared `FormField` (`src/components/ui/FormField.vue`) + `validateForm` (`src/lib/validation.ts`); show the API `hint`, never a stack. No native browser validation bubbles.
- **Editor.** Shared CodeMirror 6 "live preview" `MarkdownEditor` — markdown is the source of truth. Wikilinks are id-bound `[[<uuid>|Title]]` (rename-stable; legacy `[[Title]]` resolves by slug) and render the target's current title.
- **Shared components — reuse, never duplicate (non-negotiable).** Use the design-system primitives instead of re-implementing dropdowns, menus, toggles, confirmations, rows, headers, or empty states per panel: `Dropdown` (single-select), `Popover` (anchored surface), `ConfirmDialog`, `FormField` + `validateForm`, `SettingsTable`, `ExpandableRow` (collapsed-summary + inline manage panel), `PanelHeader` (title + subtitle + actions), `RowAction` (compact row button), `EmptyState` (full + `compact`). Shared **logic** is reused the same way, never re-inlined: `errorHint` (`lib/apiError`), `initials`/`formatDate` (`lib/format`), workspace/grant role helpers (`lib/workspaceRoles`, `lib/grantRoles`), `useLoadingMap` (`composables/`). The moment a visual/behavioral pattern recurs, extract one component or helper and have every call site use it; duplicated markup/CSS/logic across components is a defect to remove, not extend. Full rule in `CODE_STYLE.md` → TypeScript / Vue → Patterns.
- **Tooling.** Biome (not eslint/prettier), Vitest, vue-tsc — all in `verify`. Match existing component/store patterns; same English-only, comment-sparing conventions as the Rust side (see `CODE_STYLE.md`).

## Conventions

- **Merge gate.** Merging to `main` is forbidden unless `verify` passes in the dev shell. This gate compiles, lints, and checks formatting for both the backend and frontend, and runs the test suite.
- **Strict TDD.** Write the failing test first, see it red, then implement to green. Tests run with `cargo nextest`; doctests run separately.
- **No panics.** Lints deny `unwrap_used`, `expect_used`, `panic`, `unwrap_in_result`, `dbg_macro`; `unsafe_code` is forbidden. Propagate with `?`; return `Result`. `todo!`/`unimplemented!` warn — never ship them.
- **No silently discarded errors.** Never use `let _ =` on a fallible expression. Propagate with `?`, branch explicitly with `match`/`if let`, log it, or surface it to the user — but never swallow a `Result`/`Option` error.
- **Commits.** Conventional Commits (`feat:`, `fix:`, `chore:`, `refactor:`, `docs:`, `test:`, `ci:`) directly to `main`, one atomic work-unit per commit (code + its test together). Author identity `Ignacio Perez <ignacio@feuer.me>`. No co-author trailers.
- **English** for all code, comments, docs, and commit messages.
- **Comments.** Default to none. Add one only when the *why* is non-obvious (a constraint, an invariant, a workaround). Never restate what the code does; never reference a task/PR/ticket. Function-level doc comments for intent/invariants are welcome. See `CODE_STYLE.md` for the full coding conventions.
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
