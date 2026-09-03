# Registry route ownership — `v2-e3-s1-registry-population` (PR1)

This is the reviewable grounding artifact for T1.1–T1.3 of
`v2-e3-s1-registry-population`. It records, independently of the Rust
`ComponentEntry` values in `crates/atlas_server/src/reg5.rs`, which of the 138
manual `.route()` calls in `atlas_server::lib::app()` map to which of the
three HTTP-facing REG-5 components (`platform`, `custos`, `acta`), and the
basis for every resolution — including the eight ambiguities (A1–A8) the
tasks file flagged and the orchestrator's ruling resolved.

S2's bidirectional router↔registry audit checks its derived router against
this table; a reviewer can diff "declared" against "intended" here without
reading Rust.

## Method

1. Every `.route()` call in `atlas_server::lib::app()`'s two `Router` trees
   (`public`, `protected`) was enumerated mechanically from `lib.rs` source:
   138 calls, expanding to 210 `(method, path)` pairs once each call's
   `.get()/.post()/.put()/.patch()/.delete()` combinators are counted
   individually.
2. Each pair was cross-checked against `atlas_server::routes::registry::ROUTE_REGISTRY`
   (the pre-existing V1 audit tool, keyed by `openapi_path` where present) to
   recover its `capability` gate (`Option<"family:action">`), which is the
   same closed catalog `authz::authorized::RequiredScope` markers enforce via
   `Authorized<T>`.
3. Every module in `crates/atlas_server/src/routes/mod.rs` was assigned to
   exactly one component using the baseline rule, the schema-ownership
   extension table, and the orchestrator's A1–A8 ruling, all from the tasks
   file (reproduced below for traceability).
4. `RouteDeclaration.action` is `Some(ActionId::new(component, family, action))`
   when `ROUTE_REGISTRY`'s `capability` is `Some("family:action")`, `None`
   otherwise. This is a direct, mechanical translation of an existing,
   already-enforced fact (D2, declare-and-verify) — no new enforcement is
   added.

## Module → component assignment

| Module | Component | Basis |
|---|---|---|
| `activate` | custos | account activation is part of the auth/users lifecycle |
| `api_keys` | custos | baseline (explicit) |
| `attachments` | acta | baseline (explicit) |
| `audit` | custos | **A1**: `security_audit_log` is one of the eight `custos.*` tables |
| `auth` | custos | baseline (explicit) |
| `automation_rules` | acta | baseline family ("automations") |
| `boards` | acta | baseline (explicit) |
| `documents` | acta | baseline (explicit) |
| `events` | acta | `events_outbox` is an Acta-owned table (E2 S4 PR1/PR5) |
| `folders` | acta | baseline (explicit) |
| `grants` | custos | **A2**: `permission_grants` is `custos.*`; Custos owns authorization |
| `groups` | custos | baseline (explicit) |
| `health` | platform | baseline (explicit) |
| `integration_configs` | acta | same family as `webhooks`/`automation_rules` |
| `integrations_ingest` | acta | **A8**: `integration_configs` is `acta.*`; the GitHub ingest webhook is Acta's ingress |
| `members` | acta | **A5**: `workspace_memberships` is `acta.*` (moved E2 S4 batch 1); the FK into `custos.users` does not transfer ownership. This **overrides** the tasks file's own baseline list (which named `members` under `custos`) |
| `openapi` | platform | baseline (explicit) |
| `platform_status_templates` | acta | verified E2 S4 PR10: "Acta-owned despite the name" |
| `presence` | acta | **A3**: collaborative presence over Acta content (boards, documents); no Custos table involved |
| `projects` | acta | same table family as `documents`/`folders` (E2 S4 PR2) |
| `property_definitions` | acta | same table family (E2 S4 PR2) |
| `registry` | *(not a route)* | **A4**: audit-only `ROUTE_REGISTRY` data module, serves no HTTP path; excluded from the partition (deleted in S2) |
| `saved_searches` | acta | search family |
| `search` | acta | baseline (explicit) |
| `semantic_search` | acta | search family |
| `status_templates` | acta | `workspace_status_templates` table, boards family |
| `tags` | acta | `tags` table is Acta-owned |
| `task_views` | acta | search/views family |
| `tasks` | acta | baseline (explicit) |
| `trash` | acta | **A6**: purges Acta content (documents/folders/projects/boards) |
| `ui_state` | platform | `user_ui_state` moved to `platform.ui_state` in E2 S4 PR9 |
| `users` | custos | baseline (explicit) |
| `validation` | *(not a route)* | **A7**: shared input-check helpers, no HTTP path |
| `webhooks` | acta | baseline (explicit) |
| `workspaces` | acta | `workspaces`/`workspace_memberships` moved to the `acta` Postgres schema (E2 S4 PR1/PR11); SHELL-NAV-1 confirms the Shell does not own workspaces |

`meta` and `/health`/`/ready`/`/version` are served by the `health` module
(`platform`); `/openapi.json` and `/scalar` are served by the `openapi`
module (`platform`). Both are now ordinary `RouteDeclaration`s as of
`v2-e3-s4` PR1 (see the update on findings 3–4 below); at the time this
table was first written (S1/S2), neither was representable, and so neither
appeared in `api.routes`.

## Genuinely resolved tensions (kept explicit, not silently absorbed)

- **A5 `members` vs. the baseline list.** The tasks file's own baseline table
  named `members` under `custos`, but the orchestrator's ruling assigns it to
  `acta` on verified schema evidence (`workspace_memberships` lives in the
  `acta` schema, same as `workspaces`). This artifact follows the ruling
  (authoritative), not the earlier baseline entry, and records the
  discrepancy here so it is never mistaken for an oversight.

## Findings discovered while enumerating (not ownership ambiguities — reported per instruction, not guessed)

1. **The tasks file's "139 `.route()` calls" is off by one.** Direct,
   reproducible enumeration of `atlas_server::lib::app()` finds exactly
   **138** `.route()` calls (76 single-method, 52 two-method, 10
   three-method = 210 `(method, path)` pairs, one of which — `/openapi.json`
   — is separately excluded below). The 139th `.route()` call in the whole
   `lib.rs` file belongs to `test_app_with_route`, a test-only helper outside
   `app()` that builds an unrelated single-route `Router` for
   `tests/error_model.rs` — it is not part of the live server and is
   correctly excluded from this partition.
2. **`ROUTE_REGISTRY` (the pre-existing V1 audit tool) is missing one entry**:
   `PATCH /api/workspaces/{ws}/integration-configs/{config_id}`
   (`integration_configs::patch_integration_config`) has no corresponding
   `RouteEntry`, confirming the tool's own documented limitation ("a route
   added to `lib.rs` without a registry entry is not caught"). This is a
   pre-existing V1 drift, not something this slice introduces or is asked to
   fix (S2 deletes `ROUTE_REGISTRY`). Its `action` is declared `None` here by
   consistency with its three sibling routes in the same resource (all
   `capability: None`, admin-only/agent-unreachable), not by silent omission.
3. **`/openapi.json` could not be declared as a `RouteDeclaration` at S1/S2
   time.** `RoutePath::new` rejected any segment containing `.`
   (`IllegalCharacter`), and `/openapi.json`'s only segment is literally
   `openapi.json` — confirmed by running the workspace test against it
   (`IllegalCharacter { ch: '.' }`). It also had no `ROUTE_REGISTRY` entry
   (that V1 audit tool deliberately excluded the OpenAPI document endpoint
   itself). It was therefore excluded from `api.routes` for the same reason
   as `/scalar` below, dropping the enumerated total this PR's `api.routes`
   could actually hold to **210**. **Update (`v2-e3-s4` PR1, D3)**:
   `RoutePath` was widened to accept exactly one interior `.` in a path's
   final segment, making `/openapi.json` representable; it is now an
   ordinary `platform`-owned `RouteDeclaration` (raising the total to
   **212**), and `ROUTE_SET_EXCLUSIONS`'s two former entries are both
   retired to an empty list.
4. **`/scalar`** (the Scalar docs UI) was excluded from `api.routes` at
   S1/S2 time: it is mounted via `.merge(routes::openapi::scalar_router())`,
   not a `.route()` call, so it fell outside the "139/138 manual `.route()`
   calls" enumeration this slice's Requirement and T1.11 count-check were
   scoped to, and it carries no operation contract (no DTOs, no
   operation_id) to declare as a `RouteDeclaration`'s method/path pair
   alone required. **Update (`v2-e3-s4` PR1, D3)**: declared anyway, as an
   ordinary `platform`-owned `RouteDeclaration` with `operation_id: "scalar"`
   — a `RouteDeclaration` needs no real utoipa operation contract, only a
   `(method, path)` pair and an `operation_id` string, which `/scalar` can
   supply trivially even though it has no OpenAPI document entry of its own.
5. **`storage.filesystem` and `storage.s3` cannot both be mandatory
   `storage.blob` providers in the same `registry::build()` call.** Both
   Module entries exist in `reg5.rs` (T1.8), but `reg5_component_entries()`
   takes a `StorageBackend` selector and includes exactly one, mirroring the
   documented rationale already present on the pre-existing
   `shell_reg_5_valid_entries()` workspace-test fixture in
   `crates/atlas_core/tests/registry_build.rs` (which excludes `storage.s3`
   for the identical reason: `MandatoryCapabilityAmbiguous`). PR2 wires the
   real selection from `atlas_server` configuration at startup. Read literally,
   the spec's "the set of `stable_id` values ... equals exactly" REG-5
   scenario names all seven components; this PR treats that as the release's
   allowed universe (no Hermes/Infrastructure/Minerva/Mnemosyne, and no more
   than one active storage backend at a time), consistent with the existing
   validator behavior and the existing passing test, rather than inventing a
   new capability-identity split to force seven simultaneous entries.
6. **`RouteDeclaration.path` is the full, absolute, currently-served path**
   (e.g. `/api/workspaces/{ws}/tasks/{readable_id}`), not a
   namespace-relative path as `entry.rs`'s own illustrative
   `full_entry()` test fixture models (`/tasks/{task_id}`). This slice's
   Requirement text is explicit ("match the live server exactly ... it only
   describes the router as it exists"; "no route or URL change of any
   kind"). `v2-e3-s4` PR4 later made `RouteDeclaration.path` namespace-
   relative, and PR7 landed what a namespace-relative path means under the
   `/api/v2/{component}` mount: a namespace-relative path `<rel>` is served
   at `/api/<rel>` (V1, unchanged) and at `/api/v2/<component>/<rel>` (V2),
   where `<component>` is the owning `ComponentEntry.identity.stable_id`.

   > **`v2-e3-s7` update (dated note, table below unchanged):** the V1 half
   > of the statement above — `/api/<rel>` as a second live mount — was
   > retired by `v2-e3-s7`. Since that slice, every route is reachable at
   > exactly its own `/api/v2/<component>/<rel>` mount; `/api/<rel>` answers
   > the generic protected fallback (401 unauthenticated, 404 authenticated)
   > like any other unrouted path. The 212-row table below is a historical
   > record of `RouteDeclaration.path` as it was at `v2-e3-s2` and is not
   > rewritten to reflect the retirement — see this file's allowlist entry
   > in `crates/atlas_server/tests/api_path_literal_guard.rs`.

## `idempotent` inference rule

Not derivable from any existing source of truth (`ROUTE_REGISTRY` does not
carry it), so it is inferred structurally from HTTP method semantics:
`GET`/`PUT`/`DELETE`/`HEAD`/`OPTIONS` → `true`, `POST`/`PATCH` → `false`. No
per-route business-idempotency override was applied.

## `operation_id` rule

`utoipa`'s default (the handler function name), except for the 26 explicit
`operation_id = "..."` overrides already present in `documents.rs`,
`tasks.rs`, and `attachments.rs` (used to disambiguate handlers that share a
literal Rust function name across modules, e.g. `documents::create_comment`
vs. `tasks::create_comment`, both flattened into one OpenAPI document today).
Each override was verified against the source line it annotates, not assumed
from the override string alone.

## Full route → component table (212 declarations)

### platform (8 routes)

| Method | Path | operation_id | action | idempotent |
|---|---|---|---|---|
| GET | `/api/me/ui-state` | `get_ui_state` | `—` | true |
| PUT | `/api/me/ui-state` | `set_ui_state` | `—` | true |
| GET | `/api/meta` | `meta` | `—` | true |
| GET | `/health` | `health` | `—` | true |
| GET | `/ready` | `ready` | `—` | true |
| GET | `/version` | `version` | `—` | true |
| GET | `/openapi.json` | `openapi_json` | `—` | false |
| GET | `/scalar` | `scalar` | `—` | false |

### custos (35 routes)

| Method | Path | operation_id | action | idempotent |
|---|---|---|---|---|
| GET | `/api/activate/{token}` | `get_activation_info` | `—` | true |
| POST | `/api/activate/{token}` | `post_activate` | `—` | false |
| GET | `/api/admin/audit` | `list_platform_audit` | `—` | true |
| GET | `/api/api-keys` | `list_user_api_keys` | `—` | true |
| POST | `/api/api-keys` | `create_user_api_key` | `—` | false |
| DELETE | `/api/api-keys/{key_id}` | `revoke_user_api_key` | `—` | true |
| PATCH | `/api/api-keys/{key_id}` | `update_user_api_key` | `—` | false |
| GET | `/api/api-keys/{key_id}/grants` | `list_api_key_grants` | `—` | true |
| DELETE | `/api/api-keys/{key_id}/grants/{grant_id}` | `delete_api_key_grant` | `—` | true |
| POST | `/api/auth/change-password` | `change_password` | `—` | false |
| POST | `/api/auth/login` | `login` | `—` | false |
| POST | `/api/auth/logout` | `logout` | `—` | false |
| GET | `/api/auth/me` | `me` | `—` | true |
| GET | `/api/users` | `list_users` | `—` | true |
| POST | `/api/users` | `create_user` | `—` | false |
| PATCH | `/api/users/me` | `update_me` | `—` | false |
| POST | `/api/users/{user_id}/activation-link` | `regenerate_activation_link` | `—` | false |
| POST | `/api/users/{user_id}/disable` | `disable_user` | `—` | false |
| POST | `/api/users/{user_id}/enable` | `enable_user` | `—` | false |
| GET | `/api/users/{user_id}/memberships` | `list_user_memberships` | `—` | true |
| POST | `/api/users/{user_id}/reset-password` | `reset_password` | `—` | false |
| POST | `/api/users/{user_id}/system-admin` | `set_system_admin` | `—` | false |
| GET | `/api/workspaces/{ws}/audit` | `list_workspace_audit` | `—` | true |
| GET | `/api/workspaces/{ws}/grants` | `list_workspace_grants` | `custos::grants::read` | true |
| POST | `/api/workspaces/{ws}/grants` | `create_workspace_grant` | `—` | false |
| DELETE | `/api/workspaces/{ws}/grants/{grant_id}` | `delete_workspace_grant` | `—` | true |
| GET | `/api/workspaces/{ws}/groups` | `list_groups` | `—` | true |
| POST | `/api/workspaces/{ws}/groups` | `create_group` | `—` | false |
| DELETE | `/api/workspaces/{ws}/groups/{group_id}` | `delete_group` | `—` | true |
| GET | `/api/workspaces/{ws}/groups/{group_id}/members` | `list_group_members` | `—` | true |
| POST | `/api/workspaces/{ws}/groups/{group_id}/members` | `add_group_member` | `—` | false |
| DELETE | `/api/workspaces/{ws}/groups/{group_id}/members/{user_id}` | `remove_group_member` | `—` | true |
| GET | `/api/workspaces/{ws}/projects/{project_slug}/grants` | `list_project_grants` | `custos::grants::read` | true |
| POST | `/api/workspaces/{ws}/projects/{project_slug}/grants` | `create_project_grant` | `—` | false |
| DELETE | `/api/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}` | `delete_project_grant` | `—` | true |

### acta (169 routes)

| Method | Path | operation_id | action | idempotent |
|---|---|---|---|---|
| GET | `/api/admin/status-templates` | `list_platform_status_templates` | `—` | true |
| POST | `/api/admin/status-templates` | `create_platform_status_template` | `—` | false |
| PATCH | `/api/admin/status-templates/{template_id}` | `update_platform_status_template` | `—` | false |
| DELETE | `/api/admin/status-templates/{template_id}` | `delete_platform_status_template` | `—` | true |
| GET | `/api/admin/trash` | `list_trash` | `—` | true |
| POST | `/api/admin/trash/purge` | `purge_trash` | `—` | false |
| GET | `/api/admin/trash/purges/{operation_id}` | `get_purge_status` | `—` | true |
| POST | `/api/admin/trash/restore` | `restore_trash` | `—` | false |
| GET | `/api/admin/workspaces` | `admin_list_workspaces` | `—` | true |
| DELETE | `/api/admin/workspaces/{ws}` | `admin_delete_workspace` | `—` | true |
| PATCH | `/api/admin/workspaces/{ws}` | `admin_update_workspace` | `—` | false |
| GET | `/api/workspaces` | `list_workspaces` | `—` | true |
| POST | `/api/workspaces` | `create_workspace` | `—` | false |
| GET | `/api/workspaces/{ws}` | `get_workspace` | `—` | true |
| PATCH | `/api/workspaces/{ws}` | `update_workspace` | `acta::config::update` | false |
| GET | `/api/workspaces/{ws}/activity` | `list_workspace_activity` | `—` | true |
| GET | `/api/workspaces/{ws}/assignable-users` | `list_assignable_users` | `—` | true |
| GET | `/api/workspaces/{ws}/attachments` | `list_workspace_attachments` | `—` | true |
| DELETE | `/api/workspaces/{ws}/attachments/{attachment_id}` | `delete_attachment` | `acta::docs::update` | true |
| GET | `/api/workspaces/{ws}/attachments/{attachment_id}` | `download_attachment` | `acta::docs::read` | true |
| PATCH | `/api/workspaces/{ws}/attachments/{attachment_id}` | `rename_workspace_attachment` | `—` | false |
| GET | `/api/workspaces/{ws}/automation-rules` | `list_automation_rules` | `—` | true |
| POST | `/api/workspaces/{ws}/automation-rules` | `create_automation_rule` | `—` | false |
| DELETE | `/api/workspaces/{ws}/automation-rules/{rule_id}` | `delete_automation_rule` | `—` | true |
| GET | `/api/workspaces/{ws}/automation-rules/{rule_id}` | `get_automation_rule` | `—` | true |
| PATCH | `/api/workspaces/{ws}/automation-rules/{rule_id}` | `patch_automation_rule` | `—` | false |
| DELETE | `/api/workspaces/{ws}/boards/{board_id}` | `delete_board` | `acta::boards::delete` | true |
| GET | `/api/workspaces/{ws}/boards/{board_id}` | `get_board` | `acta::boards::read` | true |
| PATCH | `/api/workspaces/{ws}/boards/{board_id}` | `update_board` | `acta::boards::update` | false |
| POST | `/api/workspaces/{ws}/boards/{board_id}/apply-status-templates` | `apply_status_templates` | `acta::boards::update` | false |
| POST | `/api/workspaces/{ws}/boards/{board_id}/archive` | `archive_board` | `acta::boards::update` | false |
| GET | `/api/workspaces/{ws}/boards/{board_id}/columns` | `list_columns` | `acta::boards::read` | true |
| POST | `/api/workspaces/{ws}/boards/{board_id}/columns` | `create_column` | `acta::boards::update` | false |
| DELETE | `/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}` | `delete_column` | `acta::boards::update` | true |
| PATCH | `/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}` | `update_column` | `acta::boards::update` | false |
| PATCH | `/api/workspaces/{ws}/boards/{board_id}/move` | `move_board` | `acta::boards::update` | false |
| DELETE | `/api/workspaces/{ws}/boards/{board_id}/presence` | `leave` | `acta::boards::read` | true |
| POST | `/api/workspaces/{ws}/boards/{board_id}/presence` | `heartbeat` | `acta::boards::read` | false |
| GET | `/api/workspaces/{ws}/boards/{board_id}/tasks` | `list_tasks` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/boards/{board_id}/tasks` | `create_task` | `acta::tasks::create` | false |
| POST | `/api/workspaces/{ws}/boards/{board_id}/unarchive` | `unarchive_board` | `acta::boards::update` | false |
| POST | `/api/workspaces/{ws}/documents/moves/batch` | `move_documents_batch` | `acta::docs::update` | false |
| DELETE | `/api/workspaces/{ws}/documents/{slug}` | `delete_document` | `acta::docs::delete` | true |
| GET | `/api/workspaces/{ws}/documents/{slug}` | `get_document` | `acta::docs::read` | true |
| PATCH | `/api/workspaces/{ws}/documents/{slug}` | `update_document` | `acta::docs::update` | false |
| GET | `/api/workspaces/{ws}/documents/{slug}/attachments` | `list_attachments` | `acta::docs::read` | true |
| POST | `/api/workspaces/{ws}/documents/{slug}/attachments` | `upload_attachment` | `acta::docs::update` | false |
| GET | `/api/workspaces/{ws}/documents/{slug}/backlinks` | `list_backlinks` | `acta::docs::read` | true |
| POST | `/api/workspaces/{ws}/documents/{slug}/comment-drafts` | `create_document_comment_draft` | `acta::docs::update` | false |
| DELETE | `/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}` | `cancel_document_comment_draft` | `acta::docs::update` | true |
| POST | `/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments` | `upload_document_comment_draft_attachment` | `acta::docs::update` | false |
| GET | `/api/workspaces/{ws}/documents/{slug}/comments` | `list_document_comments` | `acta::docs::read` | true |
| POST | `/api/workspaces/{ws}/documents/{slug}/comments` | `create_document_comment` | `acta::docs::update` | false |
| DELETE | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}` | `delete_document_comment` | `acta::docs::update` | true |
| PATCH | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}` | `update_document_comment` | `acta::docs::update` | false |
| GET | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments` | `list_document_comment_attachments` | `acta::docs::read` | true |
| POST | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments` | `upload_document_comment_attachment` | `acta::docs::update` | false |
| DELETE | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}` | `delete_document_comment_attachment` | `acta::docs::update` | true |
| GET | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}` | `download_document_comment_attachment` | `acta::docs::read` | true |
| GET | `/api/workspaces/{ws}/documents/{slug}/compact` | `get_document_compact` | `acta::docs::read` | true |
| PUT | `/api/workspaces/{ws}/documents/{slug}/content` | `update_content` | `acta::docs::update` | true |
| GET | `/api/workspaces/{ws}/documents/{slug}/content/range` | `get_content_range` | `acta::docs::read` | true |
| PATCH | `/api/workspaces/{ws}/documents/{slug}/content/range` | `edit_content_range` | `acta::docs::update` | false |
| POST | `/api/workspaces/{ws}/documents/{slug}/content/search` | `search_content` | `acta::docs::read` | false |
| POST | `/api/workspaces/{ws}/documents/{slug}/copy` | `copy_document` | `acta::docs::create` | false |
| GET | `/api/workspaces/{ws}/documents/{slug}/frontmatter` | `get_frontmatter` | `acta::docs::read` | true |
| GET | `/api/workspaces/{ws}/documents/{slug}/history` | `list_history` | `acta::docs::read` | true |
| PATCH | `/api/workspaces/{ws}/documents/{slug}/move` | `move_document` | `acta::docs::update` | false |
| DELETE | `/api/workspaces/{ws}/documents/{slug}/presence` | `document_leave` | `acta::docs::read` | true |
| POST | `/api/workspaces/{ws}/documents/{slug}/presence` | `document_heartbeat` | `acta::docs::read` | false |
| GET | `/api/workspaces/{ws}/documents/{slug}/revisions/{seq}` | `get_revision_content` | `acta::docs::read` | true |
| GET | `/api/workspaces/{ws}/events` | `stream_events` | `—` | true |
| DELETE | `/api/workspaces/{ws}/folders/{folder_id}` | `delete_folder` | `acta::folders::delete` | true |
| GET | `/api/workspaces/{ws}/folders/{folder_id}` | `get_folder` | `acta::folders::read` | true |
| PATCH | `/api/workspaces/{ws}/folders/{folder_id}` | `rename_folder` | `acta::folders::update` | false |
| POST | `/api/workspaces/{ws}/folders/{folder_id}/copy` | `copy_folder` | `acta::folders::create` | false |
| PATCH | `/api/workspaces/{ws}/folders/{folder_id}/move` | `move_folder` | `acta::folders::update` | false |
| GET | `/api/workspaces/{ws}/integration-configs` | `list_integration_configs` | `—` | true |
| POST | `/api/workspaces/{ws}/integration-configs` | `create_integration_config` | `—` | false |
| DELETE | `/api/workspaces/{ws}/integration-configs/{config_id}` | `delete_integration_config` | `—` | true |
| GET | `/api/workspaces/{ws}/integration-configs/{config_id}` | `get_integration_config` | `—` | true |
| PATCH | `/api/workspaces/{ws}/integration-configs/{config_id}` | `patch_integration_config` | `—` | false |
| POST | `/api/workspaces/{ws}/integrations/{integration}/events` | `ingest_github_event` | `—` | false |
| GET | `/api/workspaces/{ws}/members` | `list_workspace_members` | `—` | true |
| POST | `/api/workspaces/{ws}/members` | `add_member` | `—` | false |
| DELETE | `/api/workspaces/{ws}/members/{user_id}` | `remove_member` | `—` | true |
| PATCH | `/api/workspaces/{ws}/members/{user_id}` | `update_member_role` | `—` | false |
| GET | `/api/workspaces/{ws}/projects` | `list_projects` | `acta::projects::read` | true |
| POST | `/api/workspaces/{ws}/projects` | `create_project` | `acta::projects::create` | false |
| DELETE | `/api/workspaces/{ws}/projects/{project_slug}` | `delete_project` | `acta::projects::delete` | true |
| GET | `/api/workspaces/{ws}/projects/{project_slug}` | `get_project` | `acta::projects::read` | true |
| PATCH | `/api/workspaces/{ws}/projects/{project_slug}` | `update_project` | `acta::projects::update` | false |
| GET | `/api/workspaces/{ws}/projects/{project_slug}/boards` | `list_boards` | `acta::boards::read` | true |
| POST | `/api/workspaces/{ws}/projects/{project_slug}/boards` | `create_board` | `acta::boards::create` | false |
| GET | `/api/workspaces/{ws}/projects/{project_slug}/documents` | `list_documents` | `acta::docs::read` | true |
| POST | `/api/workspaces/{ws}/projects/{project_slug}/documents` | `create_document` | `acta::docs::create` | false |
| GET | `/api/workspaces/{ws}/projects/{project_slug}/folders` | `list_folders` | `acta::folders::read` | true |
| POST | `/api/workspaces/{ws}/projects/{project_slug}/folders` | `create_folder` | `acta::folders::create` | false |
| GET | `/api/workspaces/{ws}/property-definitions` | `list_property_definitions` | `acta::config::read` | true |
| POST | `/api/workspaces/{ws}/property-definitions` | `create_property_definition` | `acta::config::create` | false |
| DELETE | `/api/workspaces/{ws}/property-definitions/{property_definition_id}` | `delete_property_definition` | `acta::config::delete` | true |
| GET | `/api/workspaces/{ws}/saved-searches` | `list_saved_searches` | `acta::saved_searches::read` | true |
| POST | `/api/workspaces/{ws}/saved-searches` | `create_saved_search` | `acta::saved_searches::create` | false |
| DELETE | `/api/workspaces/{ws}/saved-searches/{id}` | `delete_saved_search` | `acta::saved_searches::delete` | true |
| PATCH | `/api/workspaces/{ws}/saved-searches/{id}` | `rename_saved_search` | `acta::saved_searches::update` | false |
| GET | `/api/workspaces/{ws}/search` | `search` | `—` | true |
| GET | `/api/workspaces/{ws}/semantic-search` | `semantic_search` | `—` | true |
| GET | `/api/workspaces/{ws}/semantic-search/reindex` | `semantic_reindex_plan` | `acta::config::read` | true |
| POST | `/api/workspaces/{ws}/semantic-search/reindex` | `semantic_reindex_start` | `acta::config::update` | false |
| GET | `/api/workspaces/{ws}/status-templates` | `list_status_templates` | `acta::boards::read` | true |
| POST | `/api/workspaces/{ws}/status-templates` | `create_status_template` | `acta::boards::create` | false |
| DELETE | `/api/workspaces/{ws}/status-templates/{template_id}` | `delete_status_template` | `acta::boards::delete` | true |
| PATCH | `/api/workspaces/{ws}/status-templates/{template_id}` | `update_status_template` | `acta::boards::update` | false |
| GET | `/api/workspaces/{ws}/tags` | `list_tags` | `acta::config::read` | true |
| POST | `/api/workspaces/{ws}/tags` | `create_tag` | `acta::config::create` | false |
| GET | `/api/workspaces/{ws}/tags/used` | `list_used_labels` | `acta::config::read` | true |
| DELETE | `/api/workspaces/{ws}/tags/{tag_id}` | `delete_tag` | `acta::config::delete` | true |
| PATCH | `/api/workspaces/{ws}/tags/{tag_id}` | `patch_tag` | `acta::config::update` | false |
| GET | `/api/workspaces/{ws}/task-views` | `list_task_views` | `acta::task_views::read` | true |
| POST | `/api/workspaces/{ws}/task-views` | `create_task_view` | `acta::task_views::create` | false |
| DELETE | `/api/workspaces/{ws}/task-views/{id}` | `delete_task_view` | `acta::task_views::delete` | true |
| GET | `/api/workspaces/{ws}/task-views/{id}` | `get_task_view` | `acta::task_views::read` | true |
| PATCH | `/api/workspaces/{ws}/task-views/{id}` | `update_task_view` | `acta::task_views::update` | false |
| GET | `/api/workspaces/{ws}/tasks` | `list_workspace_tasks` | `acta::tasks::read` | true |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}` | `delete_task` | `acta::tasks::delete` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}` | `get_task` | `acta::tasks::read` | true |
| PATCH | `/api/workspaces/{ws}/tasks/{readable_id}` | `update_task` | `acta::tasks::update` | false |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/activity` | `list_activity` | `acta::tasks::read` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/assignees` | `list_assignees` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/assignees` | `add_assignee` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}` | `remove_assignee` | `acta::tasks::update` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/attachments` | `list_task_attachments` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/attachments` | `upload_task_attachment` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}` | `delete_task_attachment` | `acta::tasks::update` | true |
| PATCH | `/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}` | `rename_task_attachment` | `acta::tasks::update` | false |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content` | `download_task_attachment` | `acta::tasks::read` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/backlinks` | `list_task_backlinks` | `acta::tasks::read` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/checklist` | `list_checklist` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/checklist` | `create_checklist_item` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}` | `delete_checklist_item` | `acta::tasks::update` | true |
| PATCH | `/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}` | `update_checklist_item` | `acta::tasks::update` | false |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote` | `promote_checklist_item` | `acta::tasks::create` | false |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts` | `create_task_comment_draft` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}` | `cancel_task_comment_draft` | `acta::tasks::update` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments` | `upload_task_comment_draft_attachment` | `acta::tasks::update` | false |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/comments` | `list_comments` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/comments` | `create_task_comment` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}` | `delete_comment` | `acta::tasks::update` | true |
| PATCH | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}` | `update_comment` | `acta::tasks::update` | false |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments` | `list_task_comment_attachments` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments` | `upload_task_comment_attachment` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}` | `delete_task_comment_attachment` | `acta::tasks::update` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content` | `download_task_comment_attachment` | `acta::tasks::read` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/graph` | `get_task_graph` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/move` | `move_task` | `acta::tasks::update` | false |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/parent` | `set_task_parent` | `acta::tasks::update` | false |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/promote` | `promote_subtask` | `acta::tasks::update` | false |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/references` | `list_references` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/references` | `create_reference` | `acta::tasks::update` | false |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/references/batch` | `create_references_batch` | `acta::tasks::update` | false |
| DELETE | `/api/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}` | `delete_reference` | `acta::tasks::update` | true |
| GET | `/api/workspaces/{ws}/tasks/{readable_id}/subtasks` | `list_subtasks` | `acta::tasks::read` | true |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/subtasks` | `create_subtask` | `acta::tasks::create` | false |
| GET | `/api/workspaces/{ws}/webhooks` | `list_webhooks` | `acta::webhooks::read` | true |
| POST | `/api/workspaces/{ws}/webhooks` | `create_webhook` | `acta::webhooks::create` | false |
| DELETE | `/api/workspaces/{ws}/webhooks/{webhook_id}` | `delete_webhook` | `acta::webhooks::delete` | true |
| GET | `/api/workspaces/{ws}/webhooks/{webhook_id}` | `get_webhook` | `acta::webhooks::read` | true |
| PATCH | `/api/workspaces/{ws}/webhooks/{webhook_id}` | `update_webhook` | `acta::webhooks::update` | false |
| GET | `/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries` | `list_webhook_deliveries` | `acta::webhooks::read` | true |

## Correction (post-verify, PR1)

The first draft of this table dropped `PATCH /api/admin/status-templates/{template_id}`
(`update_platform_status_template`), whose `DELETE` sibling on the same path was
declared. The independent verify pass caught it by comparing the live router's
route set against the declarations element by element; the count assertion in
`reg5_registry_build.rs` could not, because the manual enumeration that produced
the expected total was undercounted by the same one route. Corrected here and in
the declarations: acta 168 → 169, total 209 → 210, live pairs 210 → 211.

The lesson carries into S2: its router↔registry audit must compare SETS in both
directions, never totals.
