# `idempotent` re-derivation — per-route judgment (D8, `v2-e3-s3` PR4, T4.11)

The written rule lives on `RouteDeclaration.idempotent`'s doc comment
(`crates/atlas_core/src/registry/route.rs`). Every non-`POST` route is
mechanically `false` under that rule (146 entries, T4.10) and needs no
judgment. Of the 64 `POST` routes, a `create_*` operation id names a plain
resource-creation mechanically (25 entries: `idempotent: true`). This file
records the reasoning for every one of the remaining 39 `POST` routes, where
the rule's application is a judgment call, not a name match — this includes
`create_user_api_key`, `create_webhook`, `create_integration_config`, and
`create_user`, whose operation ids match the mechanical `create_*` pattern
but are judged `false` under the one-shot-secret clause (F2, `v2-e3-s3` PR4
correction; `create_webhook`/`create_integration_config`/`create_user` in the
F3 follow-up below), and the six `upload_*` attachment routes, judged `false`
under the streamed-body clause (F4 follow-up below). The exhaustive
rule-conformance test
(`crates/atlas_server/tests/reg5_idempotent_rule_conformance.rs`) encodes
these same 64 decisions and fails, naming the entry, on any disagreement with
`reg5.rs`.

| Method | Path | `operation_id` | `idempotent` | Reasoning |
|---|---|---|---|---|
| POST | `/api/auth/logout` | `logout` | `false` | Ends a session; not a creation, and a duplicate logout is a no-op (already logged out), not a wrongly-duplicated side effect. |
| POST | `/api/auth/change-password` | `change_password` | `false` | Mutates an existing resource's field; setting the same password twice produces the same end state, the `PATCH`-shaped case the rule already covers by analogy. |
| POST | `/api/users/{user_id}/disable` | `disable_user` | `false` | State-flag mutation on an existing resource; disabling an already-disabled user is a no-op. |
| POST | `/api/users/{user_id}/enable` | `enable_user` | `false` | Same as `disable_user`, opposite direction. |
| POST | `/api/users/{user_id}/reset-password` | `reset_password` | `false` | Correction (F2, `v2-e3-s3` PR4): the response carries the new reset credential in plaintext, a one-shot secret. Dedup requires storing the response, so a completed row would keep that plaintext secret in `platform.idempotency_keys.response_body` for the full retention window — the one-shot-secret clause on `RouteDeclaration.idempotent`'s doc comment. Superseded the original "mints a credential" `true` reasoning below the strikethrough note. |
| POST | `/api/users/{user_id}/activation-link` | `regenerate_activation_link` | `false` | Correction (F2, `v2-e3-s3` PR4): same reasoning as `reset_password` — the response carries the new activation link/token in plaintext, a one-shot secret that dedup storage would retain. |
| POST | `/api/api-keys` | `create_user_api_key` | `false` | Correction (F2, `v2-e3-s3` PR4): matches the mechanical `create_*` name pattern, but the response carries the newly minted API key token in plaintext (shown exactly once by design) — the same one-shot-secret class as `reset_password`/`regenerate_activation_link`, moved here out of the mechanical-`true` bucket. |
| POST | `/api/workspaces/{ws}/webhooks` | `create_webhook` | `false` | Correction (F3, `v2-e3-s3` PR4 follow-up): matches the mechanical `create_*` name pattern, but the response (`WebhookCreatedDto.secret`) carries the plaintext HMAC signing secret, shown exactly once by design — the same one-shot-secret class as `create_user_api_key`, moved here out of the mechanical-`true` bucket. Flagged as a scope gap by F2, corrected here. |
| POST | `/api/workspaces/{ws}/integration-configs` | `create_integration_config` | `false` | Correction (F3, `v2-e3-s3` PR4 follow-up): same reasoning as `create_webhook` — the response (`IntegrationConfigCreatedDto.secret`) carries a plaintext one-shot HMAC secret. Flagged as a scope gap by F2, corrected here. |
| POST | `/api/users` | `create_user` | `false` | Correction (F3, `v2-e3-s3` PR4 follow-up): matches the mechanical `create_*` name pattern, but the response (`CreateUserResponse.activation_link`) carries a freshly issued single-use activation link — the same one-shot-secret class as `regenerate_activation_link`, found by this follow-up's re-grep, not by F2. |
| POST | `/api/users/{user_id}/system-admin` | `set_system_admin` | `false` | State-flag mutation; setting the same value twice is a no-op, not a duplicated effect. |
| POST | `/api/auth/login` | `login` | `false` | No authenticated principal exists yet — `login` is what produces one. The mechanism scopes dedup to `principal_id` and runs after `require_authn`; this route sits in the unauthenticated `public` router and structurally cannot carry the middleware. |
| POST | `/api/activate/{token}` | `post_activate` | `false` | Same structural reason as `login`: unauthenticated (token-in-path auth, not a `Principal` from `require_authn`), no `principal_id` to scope by. |
| POST | `/api/admin/trash/restore` | `restore_trash` | `false` | Restores existing (already-created) entities back to a non-deleted state; a retry either no-ops (already restored) or re-restores the same set — no new resource, no duplicated side effect. |
| POST | `/api/admin/trash/purge` | `purge_trash` | `true` | Triggers an async, tracked purge operation (paired with `GET /api/admin/trash/purges/{operation_id}`); a retry would enqueue a second purge job — the "enqueueing a background job" one-shot-side-effect case the rule names. |
| POST | `/api/workspaces/{ws}/members` | `add_member` | `true` | Creates a new membership (join) row; a retry would create a duplicate membership record, the same resource-creation class as `add_group_member`/`add_assignee`. |
| POST | `/api/workspaces/{ws}/groups/{group_id}/members` | `add_group_member` | `true` | Creates a new group-membership (join) row; same reasoning as `add_member`. |
| POST | `/api/workspaces/{ws}/boards/{board_id}/apply-status-templates` | `apply_status_templates` | `false` | Applies a status-template configuration onto an existing board; reapplying the same configuration produces the same end state — not a creation. |
| POST | `/api/workspaces/{ws}/boards/{board_id}/archive` | `archive_board` | `false` | State transition on an existing resource; archiving an already-archived board is a no-op. |
| POST | `/api/workspaces/{ws}/boards/{board_id}/unarchive` | `unarchive_board` | `false` | Same as `archive_board`, opposite direction. |
| POST | `/api/workspaces/{ws}/boards/{board_id}/presence` | `heartbeat` | `false` | Periodic, intentionally-repeated presence ping; each call is meant to be sent again on an interval, so there is no "wrongly duplicated" retry to protect against. |
| POST | `/api/workspaces/{ws}/documents/{slug}/presence` | `document_heartbeat` | `false` | Same as `heartbeat`, document-scoped. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/move` | `move_task` | `false` | Mutates an existing task's position/parent; re-issuing the same move produces the same end state. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/assignees` | `add_assignee` | `true` | Creates a new assignment (join) row; a retry that races the first would create a duplicate assignment record — the same resource-creation class as `add_group_member`/`add_member`. |
| POST | `.../tasks/{readable_id}/checklist/{item_id}/promote` | `promote_checklist_item` | `true` | Converts a checklist item into a new task/subtask entity; a retry would wrongly create a second promoted entity from the same source item. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/promote` | `promote_subtask` | `true` | Same promotion-creates-an-entity reasoning as `promote_checklist_item`. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/parent` | `set_task_parent` | `false` | Sets a relationship field on an existing task; re-setting the same parent is a no-op. |
| POST | `/api/workspaces/{ws}/documents/{slug}/content/search` | `search_content` | `false` | A `POST` used only to carry a search query body; the rule's explicit read/search-shaped-POST exception. |
| POST | `/api/workspaces/{ws}/documents/{slug}/copy` | `copy_document` | `true` | Creates a new resource (the copy); a retry would create a second, distinct copy — a duplicated creation, not a no-op. |
| POST | `/api/workspaces/{ws}/documents/moves/batch` | `move_documents_batch` | `false` | Batch mutation of existing documents' positions/parents; re-issuing the same batch move produces the same end state. |
| POST | `/api/workspaces/{ws}/folders/{folder_id}/copy` | `copy_folder` | `true` | Same reasoning as `copy_document`: creates a new resource, a retry duplicates it. |
| POST | `/api/workspaces/{ws}/semantic-search/reindex` | `semantic_reindex_start` | `true` | Triggers an async reindex job; a retry would enqueue a second job — the same one-shot-background-job class as `purge_trash`. |
| POST | `/api/workspaces/{ws}/integrations/{integration}/events` | `ingest_github_event` | `false` | Unauthenticated webhook ingest (HMAC-verified by its own extractor, no `Authorized<...>`/`Principal`); sits in the `public` router outside `require_authn`, so there is no `principal_id` to scope dedup by — same structural reason as `login`/`post_activate`. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/attachments` | `upload_task_attachment` | `false` | Correction (F4, `v2-e3-s3` PR4 follow-up): matches the mechanical `upload_*` name pattern, but the handler streams the multipart body straight to the attachment store; dedup requires buffering the whole body in memory to compute the fingerprint, which is a memory regression for an upload route — the streamed-body clause on `RouteDeclaration.idempotent`'s doc comment. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments` | `upload_task_comment_attachment` | `false` | Same streamed-body reasoning as `upload_task_attachment`. |
| POST | `/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments` | `upload_task_comment_draft_attachment` | `false` | Same streamed-body reasoning as `upload_task_attachment`. |
| POST | `/api/workspaces/{ws}/documents/{slug}/attachments` | `upload_attachment` | `false` | Same streamed-body reasoning as `upload_task_attachment`. |
| POST | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments` | `upload_document_comment_attachment` | `false` | Same streamed-body reasoning as `upload_task_attachment`. |
| POST | `/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments` | `upload_document_comment_draft_attachment` | `false` | Same streamed-body reasoning as `upload_task_attachment`. |

The remaining 25 `POST` routes not listed above are mechanical `create_*`
name matches — plain resource creation, `idempotent: true`, no judgment
call: `create_automation_rule`, `create_board`,
`create_checklist_item`, `create_column`, `create_document`,
`create_document_comment`, `create_document_comment_draft`, `create_folder`,
`create_group`, `create_platform_status_template`,
`create_project`, `create_project_grant`, `create_property_definition`,
`create_reference`, `create_references_batch`, `create_saved_search`,
`create_status_template`, `create_subtask`, `create_tag`, `create_task`,
`create_task_comment`, `create_task_comment_draft`, `create_task_view`,
`create_workspace`,
`create_workspace_grant`.

25 mechanical + 39 judged = 64 `POST` routes total. 34 are `true`, 30 are
`false` (`logout`, `change_password`, `disable_user`, `enable_user`,
`set_system_admin`, `login`, `post_activate`, `restore_trash`,
`apply_status_templates`, `archive_board`, `unarchive_board`, `heartbeat`,
`document_heartbeat`, `move_task`, `set_task_parent`, `search_content`,
`move_documents_batch`, `ingest_github_event`, `reset_password`,
`regenerate_activation_link`, `create_user_api_key`, `create_webhook`,
`create_integration_config`, `create_user`, `upload_task_attachment`,
`upload_task_comment_attachment`, `upload_task_comment_draft_attachment`,
`upload_attachment`, `upload_document_comment_attachment`,
`upload_document_comment_draft_attachment`).

## T4.23 — deferred findings (verbatim, from the S3 spec's findings table)

Recorded here for the PR4 description/changelog; no code change, explicitly
deferred to S6/S7 per D-SHAPE:

- `TaskSummaryDto` returns `Page<T>` in `list_tasks` but `Vec<T>` in
  `list_subtasks` — `crates/atlas_server/src/routes/tasks.rs:1248-1252` (Page)
  vs. `tasks.rs:3946-3949` (Vec).
- API-key grants return `Vec<ApiKeyGrantDto>` where workspace/project grants
  return `Page<GrantDto>` — `crates/atlas_server/src/routes/grants.rs:198,473`
  (Page) vs. `crates/atlas_server/src/routes/api_keys.rs:600-604` (Vec).
- `ResourceRef`-as-string is declared in `atlas_core` but consumed by 0 of 210
  routes; `AuditEntryDto` decomposes the equivalent data into
  `target_type: String` + `target_id: Option<Uuid>` instead —
  `crates/atlas_core/src/ids/resource_ref.rs:10-14` (type) vs.
  `crates/atlas_api/src/dtos/audit.rs:11-27` (closest analog).

## T4.24 — full cross-read (recorded per INV-CONTAINER-UNVERIFIABLE)

Read every file the orchestrator named, plus the per-resource `api_*.rs`
files covering the same paths as the 43 now-`idempotent:true` routes,
checking for a contradicting expectation on the same request class. Every
new test this PR adds (`idempotency_middleware.rs`,
`idempotency_declare_and_verify_audit.rs`,
`reg5_idempotent_rule_conformance.rs`, `idempotency_live_sweep.rs`) was read
against each of these in turn.

| File | What it asserts on the affected routes | Contradiction? |
|---|---|---|
| `api_rfc9457_sweep.rs` | Every provoked 4xx/5xx is `application/problem+json` with `type`/`title`/`status`/`request_id`/`instance`, and `request_id` in the body equals the `x-request-id` request header. Its `Provocation` enum only ever sends an UNAUTHENTICATED or malformed-credential request — never a second identical request under a shared key. | None. The two new `ApiError` variants (`IdempotencyKeyConflict`, `IdempotencyKeyInFlight`) render through the same shared `render_problem`/`ProblemDetails` path every other variant uses, so they already satisfy this sweep's shape; this PR does not add them to the sweep's own route list since the sweep provokes 4xx via missing-auth/malformed-body, not via a reused `Idempotency-Key`, a distinct provocation class outside this sweep's `Provocation` enum. |
| `api_401_sweep.rs` | Every non-public route returns 401 for a request with NO auth and NO other headers. | None. A request with no `Principal` never reaches the idempotency middleware at all (it is mounted innermost, T4.21/T4.22) and never carries an `Idempotency-Key` header either — the exact scenario this sweep drives is structurally identical to the layer-ordering proof's own setup. |
| `api_platform_router_parity.rs`, `api_custos_router_parity.rs`, `api_acta_router_parity.rs` | An UNAUTHENTICATED request against every protected route returns 401 (proving both `require_authn` gating and live mounting). | None, same reasoning as `api_401_sweep.rs` — unauthenticated, no idempotency header, never reaches this PR's middleware. |
| `api_page_conformance.rs` | `Page<T>` shape (`items`/`has_more`/`next_cursor`) on classified list routes; per-route minimal-parent provisioning arms for `create_project`/`create_board`/`create_column`/`create_task`/webhooks/etc. | None on response shape (none of the 43 `idempotent:true` routes return `Page<T>` — all are single-resource creates or one-shot actions). Provisioning-arm style cross-checked and reused: this PR's `idempotency_live_sweep.rs` follows the identical per-path `match` + fresh-session-per-arm pattern, so no divergent assumption about how to provision a project/board/task/document was introduced. |
| `api_login_rate_limit.rs` | A burst of 10 unauthenticated `POST /api/auth/login` requests: some 401 (wrong credentials), the rest 429 with `Retry-After`; a sibling public route is unaffected. | None. `login` is unauthenticated (`idempotent: false` under D8, no `Principal` exists yet) and this test sends no `Idempotency-Key` header — the governor's 429 is unrelated to and unreachable from this PR's middleware, which never wraps `login` (custos.rs hand-declares it `idempotent: false`). |
| Per-resource `api_*.rs` files on the same paths (`api_tags.rs`, `api_property_definitions.rs`, `api_boards.rs`/`api_workspace_tasks.rs`-family, `api_documents*.rs`, `api_webhooks.rs`, `api_automation_rules.rs`, `api_groups.rs`, `api_grants.rs`, `api_users.rs`, etc.) | Each creates its target resource once (or a small fixed number of times) via `AtlasClient`'s typed methods, asserting on the returned DTO shape and business-rule behavior (uniqueness, validation, authorization). None send a custom `Idempotency-Key` header (grepped: zero matches for `idempotency-key`/`Idempotency-Key`/`idempotent-replayed` across every pre-existing test file). | None, structurally: the middleware is header-gated — its entire branch table (`Fresh`/`InFlight`/`Replay`/`Mismatch`) is unreachable unless a request carries `Idempotency-Key`, which no pre-existing test does. A route now wrapped by the middleware behaves EXACTLY as before for every request that omits the header (confirmed by `idempotency_middleware.rs`'s own pass-through case and by inspection of `idempotency_middleware` in `middleware/idempotency.rs`: the very first check is `let Some(key) = ... else { return Ok(next.run(request).await) }`). |
| `grant_resource_ref_migration.rs` | Comments on the migration count now include PR3's `m20260906_000058_acta_platform_idempotency_keys` entry (already reconciled in PR3's own T3.16 cross-read). | None new; re-confirmed the comment still matches the current migration list (PR4 adds no new migration). |

**Conclusion**: no contradiction found in any of the seven named families or
the per-resource route files. The strongest structural guarantee is that
every pre-existing test omits the new `Idempotency-Key` header entirely, and
the middleware is a complete no-op (`next.run(request).await` unchanged) for
any request that omits it — so this PR cannot silently change ANY existing
test's observed behavior, only add new behavior gated behind an opt-in
header. CI shards remain the actual gate for the container-backed tests this
reading substitutes for (INV-CONTAINER-UNVERIFIABLE).

## F2 correction (`v2-e3-s3` PR4 review, secret-response-bodies-persisted)

Three routes were flipped from `idempotent: true` to `idempotent: false`:
`reset_password`, `regenerate_activation_link`, `create_user_api_key`. Each
returns a one-shot plaintext secret (a password-reset credential, an
activation link/token, or an API key token) in its response body. Storing
that response for dedup replay would keep the plaintext secret in
`platform.idempotency_keys.response_body` for the full retention window
(default 24h), defeating "shown exactly once, hashed at rest" handling for
these secrets. `RouteDeclaration.idempotent`'s doc comment
(`crates/atlas_core/src/registry/route.rs`) now states this as an explicit
rule clause; `reg5_idempotent_rule_conformance.rs` encodes the three flips
and the updated 43-true/167-false counts.

### F3 follow-up (`v2-e3-s3` PR4, acta.rs scope) — closed

F2 flagged two more secret-carrying routes as a scope gap:
`create_webhook` (`POST /api/workspaces/{ws}/webhooks`) —
`WebhookCreatedDto.secret` (`crates/atlas_api/src/dtos/webhooks.rs`), the
plaintext HMAC signing secret, "shown exactly once" per its own doc comment
— and `create_integration_config`
(`POST /api/workspaces/{ws}/integration-configs`) —
`IntegrationConfigCreatedDto.secret`
(`crates/atlas_api/src/dtos/integrations.rs`), same one-shot-HMAC-secret
shape. Both matched the mechanical `create_*` name pattern but carry the
same one-shot-secret defect as `reset_password`/`regenerate_activation_link`/
`create_user_api_key`.

This follow-up flips both to `idempotent: false`: the `idempotent` modifier
removed from their `component_routes!` declarations in
`crates/atlas_server/src/routes/acta.rs`, `reg5.rs` updated, both moved into
the judged table above, and the mechanical-`create_*` list and counts
updated.

Re-running the same grep (every `idempotent: true` route's response DTO for
a plaintext `secret`/`token`/`api_key`/`password`/`activation_link`/`link`
field) as part of this follow-up found one more match not caught by F2:
`create_user` (`POST /api/users`) — `CreateUserResponse.activation_link`
(`crates/atlas_api/src/dtos/mod.rs`), a freshly issued single-use activation
link, the same one-shot-secret class as `regenerate_activation_link`. Flipped
the same way: the `idempotent` modifier removed from its
`component_routes!` declaration in `crates/atlas_server/src/routes/custos.rs`,
`reg5.rs` updated, moved into the judged table above.

Final counts after all three flips: 40 true / 170 false.
`reg5_idempotent_rule_conformance.rs`, `idempotency_declare_and_verify_audit.rs`,
and `idempotency_live_sweep.rs` updated to match. No open item remains.

## F4 follow-up (`v2-e3-s3` PR4 review, upload-bodies-buffered-in-memory)

Six routes were flipped from `idempotent: true` to `idempotent: false`
(ORCHESTRATOR DECISION, 2026-09-02, `R4-upload-bodies-buffered-in-memory`):
`upload_task_attachment`, `upload_task_comment_attachment`,
`upload_task_comment_draft_attachment` (task-scoped attachment uploads,
`crates/atlas_server/src/routes/acta.rs`'s `layered` module), and
`upload_attachment`, `upload_document_comment_attachment`,
`upload_document_comment_draft_attachment` (the document-scoped
equivalents — `upload_attachment` lives in `documents_folders` inside
`component_routes!`, the other two also in `layered`). Each handler streams
its multipart request body straight to the attachment store; the
idempotency middleware buffers the whole request body in memory to compute
its fingerprint (`middleware::idempotency::compute_fingerprint`), which
would force every attachment upload through an in-memory buffer regardless
of size — a memory regression for exactly the routes streaming exists to
avoid. Uploads are not dedup-stored in S3 (an orchestrator-confirmed
non-goal): the fix is not to special-case storage, but to declare these six
`idempotent: false` and remove the layer.

`RouteDeclaration.idempotent`'s doc comment
(`crates/atlas_core/src/registry/route.rs`) now states this as an explicit
rule clause. The `idempotent` modifier (or hand-wired `.layer(idempotency)`
call, for the five routes outside `component_routes!`) was removed from all
six route declarations/wiring in
`crates/atlas_server/src/routes/acta.rs`, `reg5.rs` updated, and the six
routes moved into the judged table above (out of the mechanical
`create_*`/`upload_*` bucket, which is now `create_*`-only).

Final counts after this flip: 34 true / 176 false.
`reg5_idempotent_rule_conformance.rs`, `idempotency_declare_and_verify_audit.rs`,
and `idempotency_live_sweep.rs` updated to match. No open item remains.

## 5xx policy (`v2-e3-s3` PR4 scoped correction, `R4-5xx-release-duplicates-one-shot-jobs`)

`idempotent: true` does not carry one uniform 5xx behavior. It splits by
per-route judgment, carried structurally by which `component_routes!`
middleware entry point wires the route's layer (never read from the registry
at request time, D6):

- **One-shot side effect, no domain uniqueness check** (`purge_trash`,
  `semantic_reindex_start` — the only two of the 34 `idempotent: true` routes
  in this class): a 5xx is stored briefly
  (`middleware::idempotency::idempotency_middleware_store_briefly`,
  `FAILURE_RETENTION` = 5 minutes) and replayed within that window. An
  immediate retry gets the same 5xx back instead of enqueueing a second
  purge/reindex job; a retry after the window lapses re-executes. Named in
  `crates/atlas_server/src/router_audit.rs`'s `ONE_SHOT_IDEMPOTENT_ROUTES`
  constant, each with its own reason, and wired via `component_routes!`'s
  `one_shot` modifier (`post(handler, idempotent, one_shot)` /
  `post(handler, exempt, idempotent, one_shot)`).
- **Ordinary create, caught by a domain check** (every other `idempotent:
  true` route — the 25 mechanical `create_*` routes plus `add_member`,
  `add_group_member`, `add_assignee`, `promote_checklist_item`,
  `promote_subtask`, `copy_document`, `copy_folder`,
  `create_references_batch`): a 5xx releases the `in_flight` row
  (`middleware::idempotency::idempotency_middleware_release`,
  `PgIdempotencyRepo::release`), so an immediate retry re-executes the
  handler as `Fresh`. A genuine duplicate created by that re-execution is
  caught by the handler's own domain check (a duplicate-slug/name 409, a
  unique-constraint violation), not by the idempotency store.

`tests/idempotency_declare_and_verify_audit.rs` proves the set of routes
wired to `idempotency_middleware_store_briefly` equals
`ONE_SHOT_IDEMPOTENT_ROUTES` exactly, in both directions (INV-SET), the same
"one macro expansion, one fact" property already proven for `idempotent`
itself.
