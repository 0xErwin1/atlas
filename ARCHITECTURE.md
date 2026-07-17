# Architecture — Atlas

Atlas is a hexagonal (ports-and-adapters) Rust monorepo: a pure domain core, a server that adapts it to HTTP + PostgreSQL, and thin clients (CLI, MCP) that speak the same types over the wire. One REST API serves humans (web), agents (MCP), and scripts (CLI) alike. This document is the canonical map of where things live and why. For per-change detail, see the SDD artifacts mirrored under the Obsidian vault `sdd/atlas/`.

## Layered crate map

The dependency direction is strict and **compiler-enforced**: `atlas_domain` declares only `serde`/`thiserror`/`uuid`/`chrono`, so an accidental `use sea_orm` or `use axum` in the domain fails to compile.

```mermaid
flowchart TD
    cli[atlas_cli<br/>clap] --> client[atlas_client<br/>typed HTTP]
    mcp[atlas_mcp<br/>rmcp] --> client
    client --> api[atlas_api<br/>DTOs + OpenAPI schemas]
    server[atlas_server<br/>axum + SeaORM adapters] --> api
    server --> domain[atlas_domain<br/>pure: types, ports, logic]
    api --> domain
    migration[migration<br/>sea-orm-migration tool] -.schema.-> server
    server --> pg[(PostgreSQL 17)]
```

| Crate | Responsibility | Notable contents |
|-------|----------------|------------------|
| `atlas_domain` | Pure types, value objects, errors, **repository ports** (traits taking `WorkspaceCtx`), and pure logic (permission resolution, revision diff/anchor, fractional positions) | `entities/`, `ports/`, `permissions.rs`, `ids.rs` |
| `atlas_api` | The wire contract: shared DTOs + their OpenAPI (`utoipa`) schemas + the pagination codec | `dtos/`, `pagination.rs`, `problem.rs` |
| `atlas_client` | Typed HTTP client over `atlas_api`/`atlas_domain` types; the single client used by CLI, MCP, and e2e tests | `lib.rs` |
| `atlas_server` | The axum binary: auth, permission enforcement, routing, and SeaORM **adapters** implementing the domain ports | see module tree below |
| `atlas_cli` | `atlas` command-line over `atlas_client` | `lib.rs` |
| `atlas_mcp` | MCP server (`rmcp`) over `atlas_client` | `lib.rs` |
| `migration` | `sea-orm-migration` tool crate, run via `cargo run -p migration -- <up\|fresh>` | one migration file per schema slice |

## Request lifecycle

Every request passes a fixed middleware stack, then a per-route authorization extractor. An undeclared route cannot reach a handler authenticated; a protected route declares its target resource + minimum role in its signature.

```mermaid
flowchart LR
    req[Request] --> rid[request-id]
    rid --> trace[trace]
    trace --> rl{login route?}
    rl -- yes --> gov[rate-limit]
    rl -- no --> authn
    gov --> authn[authn middleware<br/>bearer or cookie]
    authn --> csrf[CSRF check<br/>cookie mutations]
    csrf --> ext["Authorized&lt;Resource, MinRole&gt;<br/>extractor"]
    ext --> res[resolve effective role<br/>grants + visibility + defaults<br/>+ agent cap]
    res -- role &ge; min --> h[handler]
    res -- else --> deny[403 / 404]
```

- **authn**: `Authorization: Bearer` (sessions and `atlas_` API keys, distinguished by prefix) or the HttpOnly `atlas_session` cookie. Sessions enforce revocation + expiry + the user's `disabled_at`; API keys enforce revocation + expiry + the **creating user's** `disabled_at`.
- **CSRF**: cookie-authenticated state-changing requests require `X-Atlas-CSRF: 1` (SameSite=Lax + custom header); bearer and safe methods are exempt.
- **authz**: the `Authorized<R, M>` extractor loads the principal's applicable grants, runs the pure `resolve()` engine, and compares the effective role to the route's declared minimum.

## `atlas_server` module tree

| Module | Holds |
|--------|-------|
| `auth/` | `password` (argon2 in `spawn_blocking`), `tokens`, `middleware` (authn), `csrf` |
| `authz/` | `Authorized<R,M>` extractor; the `ResolvedResource` types it resolves (`WorkspaceRes`, `ProjectRes`, `FolderRes`, `BoardRes`, `TaskRes`, `DocumentRes`, `DocumentSlugRes`) + the non-resource extractors `WorkspaceMember` / `RequireUserAdmin`. `DocumentSlugRes` accepts **either** a stable document UUID or its slug |
| `routes/` | One module per resource (`auth`, `users`, `api_keys`, `workspaces`, `members`, `projects`, `folders`, `documents`, `boards`, `tasks`, `grants`, `search`, `health`); `registry` (route source of truth); `openapi` (utoipa doc + Scalar); `validation` (shared input checks) |
| `middleware/` | `problem_stamp` (request-id into error bodies) |
| `persistence/entities/` | SeaORM entity structs (DB shape) — never leak into `atlas_domain` |
| `persistence/repos/` | Adapters implementing the domain ports; map entity ↔ domain |
| `persistence/bootstrap` | Root-user seed (`ATLAS_ROOT_PASSWORD`, fail-fast) + dev seed |

`atlas_domain` mirrors the data subsystems in `entities/` and exposes them through `ports/` (one trait module per aggregate: identity, workspace_core, documents, boards_tasks, permission_grant_repo).

### HTTP surface

| Group | Endpoints (representative) |
|-------|----------------------------|
| Auth + account | `POST /v1/auth/login` · `POST /v1/auth/logout` · `GET /v1/auth/me` (returns id, username, email, display_name, is_root) · `POST /v1/auth/change-password` · `PATCH /v1/users/me` (email, display name) |
| Users (root/admin) | `GET /v1/users` · `POST /v1/users` · `POST /v1/users/{id}/disable\|enable` · `POST /v1/users/{id}/reset-password` |
| Workspaces | `GET /v1/workspaces` · `GET /v1/workspaces/{ws}/members` · agent API keys `…/api-keys` (create/list/revoke) |
| Notes | projects · folders · documents (CAS content save, revisions, **backlinks**); a document is addressable by stable **UUID or slug** |
| Tasks | boards · board columns · tasks (atomic move, assignees, references, activity) · sub-tasks (`…/tasks/{id}/subtasks` create/list, `…/tasks/{id}/promote` to detach onto the board) |
| Search | `GET /v1/workspaces/{ws}/search` (ranked docs+tasks, permission-filtered, filter tokens) |
| Sharing + meta | grants (`…/grants`) · `GET /v1/meta` (server version/build) |

## Data model

PostgreSQL 17, 20 tables. IDs are app-generated **UUIDv7** (time-ordered). Full schema and ER diagram: `sdd/atlas/atlas-e02-data-model-design-2026-06-12` (Obsidian). Highlights:

| Area | Tables | Notes |
|------|--------|-------|
| Tenancy + identity | workspaces, users, sessions, api_keys, workspace_memberships | `workspace_id NOT NULL` on every domain table; `users`/`sessions`/`api_keys` are the tenancy-root exceptions |
| Content | folders, documents, document_revisions, document_links, attachments | document content is `TEXT` (TOAST); revisions are line diffs with snapshot anchors; attachments are metadata-only (blobs live in object storage → Cloudflare R2). `document_links` is the wikilink/backlink graph, bound to the **stable target id** |
| Projects + tasks | projects, boards, board_columns, tasks, task_references, task_assignees, task_checklist_items, task_activity | readable IDs `PREFIX-n` per project (immutable); kanban order via `fractional_index` `TEXT` position; multiple assignees (user/agent), actor-attributed activity log. **Sub-tasks** are full tasks linked by `tasks.parent_task_id`: they carry every task field (status, assignees, description, tags, estimate, their own `readable_id` so they are wikilink-referenceable) but are excluded from the board listings (`parent_task_id IS NULL`); promoting one clears the parent so it appears on the board |
| Properties + access | property_definitions, permission_grants | hybrid free-frontmatter (jsonb) + typed properties; grants `(principal, resource, role)` |

Every domain row records its `created_by` actor (user XOR api_key, DB CHECK), enabling human-vs-agent attribution. `users` carry an optional `email` (recovery only). **Wikilinks** are written as `[[<uuid>|Display Title]]` — bound to the target's stable id so they survive renames; the legacy `[[Title]]` form still resolves by slug. Slugs are immutable after creation, so addressing a document by UUID or by slug both resolve.

## Permission model

Resource-sharing (not IAM). Grants `(principal, resource, role)` with roles `viewer < editor < admin` (+ `owner`, workspace-only) inheriting down `workspace > project > folder > document | board`. Most-specific grant wins; **default deny**. Visibility (`private` / `workspace` / `public`) is sugar over implicit grants. Defaults: a resource creator gets `admin`; workspace owner/admin hold implicit admin over all workspace resources; new resources default to `workspace`-edit visibility. **Agents (API keys) are capped at `editor` and never manage grants.** The list query (`list_visible`) mirrors the `resolve()` engine in both directions so a listed resource and its detail endpoint always agree. Full model: `Atlas/E00-diseno-de-producto/E00-permisos` (Obsidian).

## Web frontend (`apps/web`)

The browser UI is a Vue 3 SPA (Vite, Pinia, vue-router, Tailwind v4, Biome) — one of three first-class API consumers alongside the CLI and MCP. It never talks to the database; it speaks the same REST contract.

| Concern | Approach |
|---------|----------|
| API client | A typed `openapi-fetch` client over types generated from the served OpenAPI (`gen-types` → `src/api/types.d.ts`), wrapped thinly for the HttpOnly session cookie + CSRF header and RFC 9457 error `hint` surfacing. |
| Shell | App rail + collapsible contextual sidebar + main area + toggleable inspector dock; Ayu-dark tokens with a dark/light theme toggle. |
| Notes | CodeMirror 6 "live preview" markdown editor (markdown is the source of truth), `[[wikilink]]` autocomplete + id-bound links that render the target's current title, backlinks panel, CAS-409 three-way merge view. |
| Tasks | Kanban with optimistic drag-and-drop (rollback on conflict), Linear-style peek + full task detail, inline editing. Sub-tasks render inline (status, assignees, estimate), open as full tasks of their own, and can be promoted onto the board. |
| Cross-cutting | Command palette + global search (Cmd/Ctrl+K), per-resource Share dialog, Settings modal (account, agent API keys, root user management, about), consistent empty/loading/error states. Forms validate with **zod** through a shared `FormField`; the API's `hint` is shown, never a stack. |
| Shared design system | Reuse-never-duplicate primitives: UI in `src/components/ui` (`Dropdown`, `Popover`, `ConfirmDialog`, `FormField`) and `src/components/settings` (`SettingsTable`, `ExpandableRow`, `PanelHeader`, `RowAction`), plus `EmptyState` (full + `compact`). Cross-cutting logic lives in `src/lib` (`errorHint`, `initials`/`formatDate`, workspace/grant role helpers) and `src/composables` (`useLoadingMap`). See `CODE_STYLE.md` → TypeScript / Vue → Patterns. |

State lives in per-domain Pinia stores; `vue-router` owns navigation. Strict TDD applies here too (Vitest + vue-tsc + Biome, all in `verify`).

## Cross-cutting conventions

| Concern | Approach |
|---------|----------|
| Multi-tenancy | Every domain port takes `WorkspaceCtx`; a query that forgets `workspace_id` cannot be written through the port. Cross-tenant isolation has per-repository integration tests. |
| Errors | RFC 9457 `application/problem+json` + `request_id` + an actionable `hint`; internal errors return a generic detail and never leak internals. |
| Pagination | Opaque base64url cursors over UUIDv7 in a `Page<T>` envelope (default 50, max 200). |
| API contract | OpenAPI generated from `utoipa` annotations, served at `/openapi.json` + Scalar at `/scalar`. Route coverage is driven by `ROUTE_REGISTRY` (`routes/registry.rs`): the registry→router and registry→doc directions are audited; a route added to the router without a registry entry is **not** auto-caught (axum 0.8 exposes no Router introspection) — developers must update the registry. |
| Testing | Strict TDD; integration tests run against real Postgres with a database-per-test harness; e2e tests drive a real `TcpListener` server through `atlas_client`. |

## Next step

The API now spans identity, workspaces, notes (documents/folders), tasks (boards/tasks), search, and sharing; the web SPA (E07) consumes all of it. When a new subsystem lands (e.g. MCP tools in E08, realtime collaboration in E14), extend the matching `routes/` module, add its `ROUTE_REGISTRY` entries, regenerate the web client (`gen-types`), and update the relevant table here.
