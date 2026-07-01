# Contributor map

This page points contributors to the main implementation files behind the docs portal topics.

Keep [`ARCHITECTURE.md`](../ARCHITECTURE.md) authoritative for crate boundaries and request lifecycle. This page is a practical “where do I look first?” map.

## Start with the root docs

| File | Why it matters |
|---|---|
| [`README.md`](../README.md) | top-level repo overview and quick start |
| [`ARCHITECTURE.md`](../ARCHITECTURE.md) | canonical system map |
| [`CODE_STYLE.md`](../CODE_STYLE.md) | coding rules and verification expectations |
| [`CONTRIBUTING.md`](../CONTRIBUTING.md) | commit workflow and contributor policy |

## Backend and API contract

| Area | Main files |
|---|---|
| Router wiring | `crates/atlas_server/src/lib.rs` |
| Route inventory / auth drift checks | `crates/atlas_server/src/routes/registry.rs` |
| OpenAPI document and tags | `crates/atlas_server/src/routes/openapi.rs` |
| Route handlers | `crates/atlas_server/src/routes/*.rs` |
| Shared DTOs and request/response schemas | `crates/atlas_api/src/dtos/` |
| Typed Rust client over the API | `crates/atlas_client/src/lib.rs` |

## Requested source areas from this docs pass

### Server routes

Look here when documenting HTTP behavior:

- `crates/atlas_server/src/routes/registry.rs`
- `crates/atlas_server/src/routes/`
- especially: `auth.rs`, `users.rs`, `workspaces.rs`, `projects.rs`, `folders.rs`, `documents.rs`, `boards.rs`, `tasks.rs`, `grants.rs`, `members.rs`, `search.rs`, `api_keys.rs`, `audit.rs`, `webhooks.rs`, `integration_configs.rs`, `integrations_ingest.rs`, `automation_rules.rs`, `ui_state.rs`

### Shared API DTOs

Look here for exact request/response shapes:

- `crates/atlas_api/src/dtos/mod.rs`
- `crates/atlas_api/src/dtos/boards_tasks.rs`
- `crates/atlas_api/src/dtos/documents.rs`
- `crates/atlas_api/src/dtos/folders.rs`
- `crates/atlas_api/src/dtos/groups.rs`
- `crates/atlas_api/src/dtos/property_definitions.rs`
- `crates/atlas_api/src/dtos/saved_searches.rs`
- `crates/atlas_api/src/dtos/search.rs`
- `crates/atlas_api/src/dtos/status_templates.rs`
- `crates/atlas_api/src/dtos/tags.rs`
- `crates/atlas_api/src/dtos/task_views.rs`
- `crates/atlas_api/src/dtos/webhooks.rs`
- `crates/atlas_api/src/dtos/integrations.rs`
- `crates/atlas_api/src/dtos/automation_rules.rs`

## CLI map

| Concern | Main files |
|---|---|
| top-level parser | `crates/atlas_cli/src/cli.rs` |
| command dispatch | `crates/atlas_cli/src/commands/mod.rs` |
| auth/config precedence | `crates/atlas_cli/src/config.rs` |
| output mode selection | `crates/atlas_cli/src/output.rs` |
| workspace helper | `crates/atlas_cli/src/ctx.rs` |
| error mapping | `crates/atlas_cli/src/error.rs` |

High-value command groups:

- `crates/atlas_cli/src/commands/tasks.rs`
- `crates/atlas_cli/src/commands/docs.rs`
- `crates/atlas_cli/src/commands/users.rs`
- `crates/atlas_cli/src/commands/api_keys.rs`
- `crates/atlas_cli/src/commands/grants.rs`
- `crates/atlas_cli/src/commands/groups.rs`
- `crates/atlas_cli/src/commands/status_templates.rs`
- `crates/atlas_cli/src/commands/task_views.rs`
- `crates/atlas_cli/src/commands/property_definitions.rs`
- `crates/atlas_cli/src/commands/import/obsidian/`
- `crates/atlas_cli/src/commands/export/obsidian/`

## MCP map

| Concern | Main files |
|---|---|
| transport setup and auth boundary | `crates/atlas_mcp/src/main.rs` |
| tool/resource implementation | `crates/atlas_mcp/src/lib.rs` |
| response shaping helpers | `crates/atlas_mcp/src/response.rs` |

In `lib.rs`, useful sections are:

- `ATLAS_INSTRUCTIONS`
- parameter structs for each tool
- the `#[tool_router] impl AtlasMcp` block
- the `ServerHandler` implementation for resources (`atlas:///{workspace}/{slug}`)

## Web app map

| Concern | Main files |
|---|---|
| route list | `apps/web/src/router/routes.ts` |
| auth/navigation guard | `apps/web/src/router/index.ts` |
| notes view | `apps/web/src/views/Notes.vue` |
| tasks view | `apps/web/src/views/Tasks.vue` |
| task detail | `apps/web/src/views/TaskDetail.vue` |
| search view | `apps/web/src/views/Search.vue` |
| settings surface | `apps/web/src/views/SettingsView.vue` |
| shell and shared modal surfaces | `apps/web/src/views/AppShell.vue` |

Useful store areas:

- `apps/web/src/stores/auth.ts`
- `apps/web/src/stores/workspace.ts`
- `apps/web/src/stores/documents.ts`
- `apps/web/src/stores/boards.ts`
- `apps/web/src/stores/tasks.ts`
- `apps/web/src/stores/taskDetail.ts`
- `apps/web/src/stores/search.ts`
- `apps/web/src/stores/ui.ts`
- `apps/web/src/stores/uiState.ts`

Useful component areas:

- notes/editor: `apps/web/src/components/notas/`
- tasks: `apps/web/src/components/tareas/`
- settings/admin: `apps/web/src/components/settings/`
- sharing: `apps/web/src/components/share/`
- primitives: `apps/web/src/components/ui/`

## Tooling and setup files

| File | Purpose |
|---|---|
| `justfile` | local task recipes |
| `.env.example` | documented env vars |
| `compose.yaml` | local Postgres container |
| `process-compose.yaml` | full local stack orchestration |

## When you change a public contract

1. update the route handler and DTOs
2. update `ROUTE_REGISTRY`
3. update `routes/openapi.rs` tags/schemas when needed
4. update `atlas_client` if the typed client should expose the route
5. regenerate web types with `just gen-types`
6. update the relevant page in `docs/`
