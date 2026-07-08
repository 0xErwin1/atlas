# REST API

Atlas exposes one REST API for the web app, CLI, MCP server, and typed Rust client.

Primary implementation files:

- Router: `crates/atlas_server/src/lib.rs`
- Route registry: `crates/atlas_server/src/routes/registry.rs`
- OpenAPI document: `crates/atlas_server/src/routes/openapi.rs`
- Shared DTOs: `crates/atlas_api/src/dtos/`
- Typed client wrapper: `crates/atlas_client/src/lib.rs`

## Discovery and reference endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` | Public health check. |
| GET | `/version` | Public server version. |
| GET | `/openapi.json` | Generated OpenAPI document. |
| GET | `/scalar` | Scalar API reference UI. |
| GET | `/v1/meta` | Authenticated server build metadata. |

## Authentication and session model

Atlas accepts either bearer tokens or the `atlas_session` cookie.

### Token resolution

1. `Authorization: Bearer <token>`
2. `atlas_session` cookie

Bearer wins when both are present.

### Token type dispatch

- `atlas_...` → API key path
- any other bearer/session token → session path

### Practical implications

- Browser login returns a JSON token **and** sets the HttpOnly session cookie.
- API keys are appropriate for CLI, scripts, and agents.
- Disabled users invalidate both their sessions and the API keys they created.

## CSRF behavior

Cookie-authenticated mutating requests require:

```http
X-Atlas-CSRF: 1
```

Exemptions:

- safe methods: `GET`, `HEAD`, `OPTIONS`, `TRACE`
- any request using `Authorization: Bearer ...`

## Shared wire conventions

### Errors

Atlas returns RFC 9457 `application/problem+json` responses. Common fields include:

- `type`
- `title`
- `status`
- `detail`
- `hint`
- `request_id`

### Rate limiting

The authenticated API is rate-limited per principal (user or API key). When the
quota is exceeded the server returns `429 Too Many Requests` with a `Retry-After`
header (whole seconds). Clients should wait for that interval before retrying;
the official `atlas_client` (CLI and MCP) does this automatically. The
unauthenticated login and activation routes are separately rate-limited by IP.

### Pagination

Most list endpoints return:

```json
{
  "items": [],
  "next_cursor": null,
  "has_more": false
}
```

- most list limits clamp to `1..=200`
- lexical search uses a sort-aware cursor format distinct from ordinary list cursors
- semantic search uses a separate similarity cursor and compact hydrated-by-ID hits
- some operational endpoints use lower caps

### Resource addressing

- workspaces and projects are addressed by slug
- tasks are addressed by `readable_id` such as `ATL-42`
- document routes that target one document accept either the document slug or UUID
- many permission-denied reads intentionally return `404` to avoid leaking existence

### Upload conventions

| Surface | Request shape |
|---|---|
| Document attachment upload | raw body, `X-File-Name`, `Content-Type` |
| Task attachment upload | `multipart/form-data` with part name `file` |

## Endpoint map

### Public and activation

| Method | Path | Notes |
|---|---|---|
| POST | `/v1/auth/login` | Rate-limited login; returns token and sets session cookie. |
| GET | `/v1/activate/{token}` | Validate activation link and return display info. |
| POST | `/v1/activate/{token}` | Set initial password, activate account, and create session. |
| POST | `/v1/workspaces/{ws}/integrations/{integration}/events` | Public signed event ingest; currently used for GitHub-compatible events. |

### Authenticated self-service

| Method | Path | Notes |
|---|---|---|
| POST | `/v1/auth/logout` | End current session/token context. |
| GET | `/v1/auth/me` | Current principal summary. |
| POST | `/v1/auth/change-password` | Human password change. |
| PATCH | `/v1/users/me` | Update own email/display name. |
| GET | `/v1/me/ui-state` | Opaque per-user UI state. |
| PUT | `/v1/me/ui-state` | Replace per-user UI state. |
| GET | `/v1/workspaces` | List reachable workspaces. |
| POST | `/v1/workspaces` | Create workspace as a human user. |
| GET | `/v1/workspaces/{ws}` | Get workspace metadata. |
| PATCH | `/v1/workspaces/{ws}` | Rename workspace display name; member-facing path does not re-slug. |

### Platform administration

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/users` | Root/system-admin user list. |
| POST | `/v1/users` | Create pending user and activation link. |
| POST | `/v1/users/{user_id}/disable` | Disable user. |
| POST | `/v1/users/{user_id}/enable` | Re-enable user. |
| POST | `/v1/users/{user_id}/reset-password` | Reset user password. |
| POST | `/v1/users/{user_id}/activation-link` | Regenerate one-time activation link. |
| POST | `/v1/users/{user_id}/system-admin` | Root-only system-admin toggle. |
| GET | `/v1/users/{user_id}/memberships` | User workspace memberships. |
| GET | `/v1/admin/workspaces` | List all workspaces. |
| PATCH | `/v1/admin/workspaces/{ws}` | Admin rename/re-slug. |
| DELETE | `/v1/admin/workspaces/{ws}` | Soft-delete workspace. |
| GET | `/v1/admin/audit` | Platform security audit log. |

### User-owned API keys

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/api-keys` | List own API keys. |
| POST | `/v1/api-keys` | Create API key; secret returned exactly once. |
| PATCH | `/v1/api-keys/{key_id}` | Currently only updates `is_global`. |
| DELETE | `/v1/api-keys/{key_id}` | Revoke key. |
| GET | `/v1/api-keys/{key_id}/grants` | List grants attached to that key. |
| DELETE | `/v1/api-keys/{key_id}/grants/{grant_id}` | Delete a key grant. |

### Workspace members, groups, and grants

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/workspaces/{ws}/members` | List grant-addressable principals in workspace. |
| POST | `/v1/workspaces/{ws}/members` | Add human user to workspace. |
| GET | `/v1/workspaces/{ws}/assignable-users` | Active users not yet in workspace. |
| PATCH | `/v1/workspaces/{ws}/members/{user_id}` | Change membership role. |
| DELETE | `/v1/workspaces/{ws}/members/{user_id}` | Remove member. |
| GET | `/v1/workspaces/{ws}/groups` | List groups. |
| POST | `/v1/workspaces/{ws}/groups` | Create group. |
| DELETE | `/v1/workspaces/{ws}/groups/{group_id}` | Soft-delete group. |
| GET | `/v1/workspaces/{ws}/groups/{group_id}/members` | List group members. |
| POST | `/v1/workspaces/{ws}/groups/{group_id}/members` | Add user to group. |
| DELETE | `/v1/workspaces/{ws}/groups/{group_id}/members/{user_id}` | Remove user from group. |
| GET | `/v1/workspaces/{ws}/grants` | List workspace grants. |
| POST | `/v1/workspaces/{ws}/grants` | Create workspace grant. |
| DELETE | `/v1/workspaces/{ws}/grants/{grant_id}` | Delete workspace grant. |
| GET | `/v1/workspaces/{ws}/projects/{project_slug}/grants` | List project grants. |
| POST | `/v1/workspaces/{ws}/projects/{project_slug}/grants` | Create project grant. |
| DELETE | `/v1/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}` | Delete project grant. |

### Projects, folders, and documents

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/workspaces/{ws}/projects` | List projects. |
| POST | `/v1/workspaces/{ws}/projects` | Create project. |
| GET | `/v1/workspaces/{ws}/projects/{project_slug}` | Get project. |
| PATCH | `/v1/workspaces/{ws}/projects/{project_slug}` | Update project. |
| DELETE | `/v1/workspaces/{ws}/projects/{project_slug}` | Soft-delete project. |
| GET | `/v1/workspaces/{ws}/projects/{project_slug}/folders` | List folders in project. |
| POST | `/v1/workspaces/{ws}/projects/{project_slug}/folders` | Create folder. |
| GET | `/v1/workspaces/{ws}/folders/{folder_id}` | Get folder. |
| PATCH | `/v1/workspaces/{ws}/folders/{folder_id}` | Rename folder. |
| DELETE | `/v1/workspaces/{ws}/folders/{folder_id}` | Delete folder. |
| PATCH | `/v1/workspaces/{ws}/folders/{folder_id}/move` | Move folder. |
| POST | `/v1/workspaces/{ws}/folders/{folder_id}/copy` | Recursive folder copy. |
| GET | `/v1/workspaces/{ws}/projects/{project_slug}/documents` | List documents in project. |
| POST | `/v1/workspaces/{ws}/projects/{project_slug}/documents` | Create document. |
| GET | `/v1/workspaces/{ws}/documents/{slug}` | Get document by slug or UUID. |
| PATCH | `/v1/workspaces/{ws}/documents/{slug}` | Update document metadata. |
| DELETE | `/v1/workspaces/{ws}/documents/{slug}` | Delete document. |
| PUT | `/v1/workspaces/{ws}/documents/{slug}/content` | CAS content update; returns conflict details on `409`. |
| GET | `/v1/workspaces/{ws}/documents/{slug}/history` | Revision metadata page. |
| GET | `/v1/workspaces/{ws}/documents/{slug}/revisions/{seq}` | Full content for one revision. |
| GET | `/v1/workspaces/{ws}/documents/{slug}/backlinks` | Document backlinks. |
| GET | `/v1/workspaces/{ws}/documents/{slug}/frontmatter` | Parsed frontmatter. |
| PATCH | `/v1/workspaces/{ws}/documents/{slug}/move` | Move document. |
| POST | `/v1/workspaces/{ws}/documents/{slug}/copy` | Copy document. |
| GET | `/v1/workspaces/{ws}/documents/{slug}/attachments` | List document attachments. |
| POST | `/v1/workspaces/{ws}/documents/{slug}/attachments` | Upload document attachment. |
| GET | `/v1/workspaces/{ws}/attachments/{attachment_id}` | Download document attachment. |
| DELETE | `/v1/workspaces/{ws}/attachments/{attachment_id}` | Delete document attachment row. |

### Boards and tasks

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/workspaces/{ws}/projects/{project_slug}/boards` | List boards. |
| POST | `/v1/workspaces/{ws}/projects/{project_slug}/boards` | Create board. |
| GET | `/v1/workspaces/{ws}/boards/{board_id}` | Get board. |
| PATCH | `/v1/workspaces/{ws}/boards/{board_id}` | Update board. |
| DELETE | `/v1/workspaces/{ws}/boards/{board_id}` | Soft-delete board. |
| GET | `/v1/workspaces/{ws}/boards/{board_id}/columns` | List columns. |
| POST | `/v1/workspaces/{ws}/boards/{board_id}/columns` | Create column. |
| PATCH | `/v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}` | Update column. |
| DELETE | `/v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}` | Delete column if empty. |
| POST | `/v1/workspaces/{ws}/boards/{board_id}/apply-status-templates` | Apply workspace status templates to board. |
| GET | `/v1/workspaces/{ws}/boards/{board_id}/tasks` | List tasks on one board. |
| POST | `/v1/workspaces/{ws}/boards/{board_id}/tasks` | Create task. |
| GET | `/v1/workspaces/{ws}/tasks` | Workspace-wide task list/filtering. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}` | Get task. |
| PATCH | `/v1/workspaces/{ws}/tasks/{readable_id}` | Update task fields/properties. |
| DELETE | `/v1/workspaces/{ws}/tasks/{readable_id}` | Delete task. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/move` | Move task across columns. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/assignees` | List assignees. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/assignees` | Add assignee. |
| DELETE | `/v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}` | Remove assignee. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/references` | List outbound references. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/references` | Create outbound reference. |
| DELETE | `/v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}` | Delete reference. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/backlinks` | List inbound backlinks. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/checklist` | List checklist items. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/checklist` | Create checklist item. |
| PATCH | `/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}` | Update checklist item. |
| DELETE | `/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}` | Delete checklist item. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote` | Promote checklist item to task. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/subtasks` | List subtasks. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/subtasks` | Create subtask. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/promote` | Promote subtask to top-level task. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/attachments` | List task attachments. |
| POST | `/v1/workspaces/{ws}/tasks/{readable_id}/attachments` | Upload task attachment. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content` | Download task attachment. |
| DELETE | `/v1/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}` | Delete task attachment row. |
| GET | `/v1/workspaces/{ws}/tasks/{readable_id}/activity` | Per-task activity feed. |
| GET | `/v1/workspaces/{ws}/activity` | Workspace-wide task activity feed. |
| GET | `/v1/workspaces/{ws}/audit` | Workspace security audit feed. |

### Search and workspace registries

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/workspaces/{ws}/search` | Unified lexical note/task search. |
| GET | `/api/workspaces/{ws}/semantic-search` | Optional embedding-backed document/task discovery with compact hits; requires embeddings to be enabled. |
| GET | `/v1/workspaces/{ws}/tags` | List tags. |
| POST | `/v1/workspaces/{ws}/tags` | Create tag. |
| GET | `/v1/workspaces/{ws}/tags/used` | List task label strings currently in use. |
| PATCH | `/v1/workspaces/{ws}/tags/{tag_id}` | Update tag. |
| DELETE | `/v1/workspaces/{ws}/tags/{tag_id}` | Soft-delete tag. |
| GET | `/v1/workspaces/{ws}/status-templates` | List status templates. |
| POST | `/v1/workspaces/{ws}/status-templates` | Create status template. |
| PATCH | `/v1/workspaces/{ws}/status-templates/{template_id}` | Update status template. |
| DELETE | `/v1/workspaces/{ws}/status-templates/{template_id}` | Soft-delete status template. |
| GET | `/v1/workspaces/{ws}/property-definitions` | List custom-field definitions. |
| POST | `/v1/workspaces/{ws}/property-definitions` | Create custom-field definition. |
| DELETE | `/v1/workspaces/{ws}/property-definitions/{property_definition_id}` | Delete custom-field definition. |
| GET | `/v1/workspaces/{ws}/saved-searches` | List saved searches for current owner. |
| POST | `/v1/workspaces/{ws}/saved-searches` | Create saved search. |
| PATCH | `/v1/workspaces/{ws}/saved-searches/{id}` | Rename saved search. |
| DELETE | `/v1/workspaces/{ws}/saved-searches/{id}` | Delete saved search. |
| GET | `/v1/workspaces/{ws}/task-views` | List task views for current owner. |
| POST | `/v1/workspaces/{ws}/task-views` | Create task view. |
| GET | `/v1/workspaces/{ws}/task-views/{id}` | Get task view. |
| PATCH | `/v1/workspaces/{ws}/task-views/{id}` | Update task view. |
| DELETE | `/v1/workspaces/{ws}/task-views/{id}` | Delete task view. |

### Integrations, automation, and webhooks

| Method | Path | Notes |
|---|---|---|
| GET | `/v1/workspaces/{ws}/integration-configs` | List integration configs. |
| POST | `/v1/workspaces/{ws}/integration-configs` | Create integration config; secret returned once. |
| GET | `/v1/workspaces/{ws}/integration-configs/{config_id}` | Get integration config. |
| DELETE | `/v1/workspaces/{ws}/integration-configs/{config_id}` | Soft-delete config and revoke its integration key. |
| GET | `/v1/workspaces/{ws}/automation-rules` | List automation rules. |
| POST | `/v1/workspaces/{ws}/automation-rules` | Create automation rule. |
| GET | `/v1/workspaces/{ws}/automation-rules/{rule_id}` | Get automation rule. |
| PATCH | `/v1/workspaces/{ws}/automation-rules/{rule_id}` | Patch automation rule. |
| DELETE | `/v1/workspaces/{ws}/automation-rules/{rule_id}` | Soft-delete automation rule. |
| GET | `/v1/workspaces/{ws}/webhooks` | List webhooks. |
| POST | `/v1/workspaces/{ws}/webhooks` | Create webhook; secret returned once. |
| GET | `/v1/workspaces/{ws}/webhooks/{webhook_id}` | Get webhook. |
| PATCH | `/v1/workspaces/{ws}/webhooks/{webhook_id}` | Update webhook. |
| DELETE | `/v1/workspaces/{ws}/webhooks/{webhook_id}` | Soft-delete webhook. |
| GET | `/v1/workspaces/{ws}/webhooks/{webhook_id}/deliveries` | Delivery attempts, newest first. |

## Integration notes for API consumers

- `atlas_client` covers the core product API extensively, but not every REST route. Notable REST-first areas include activation, system-admin toggles, webhooks, integration configs, and automation rules.
- The OpenAPI document is the easiest way to inspect exact request/response schemas.
- When adding a route, update both the Axum router and `ROUTE_REGISTRY`; the reverse drift direction is not auto-detected by Axum.
